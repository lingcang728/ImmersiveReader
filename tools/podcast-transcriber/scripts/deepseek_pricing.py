from __future__ import annotations

import atexit
import itertools
import json
import logging
import math
import os
import socket
import threading
import urllib.error
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from pathlib import Path
from typing import Any

DEEPSEEK_DEFAULT_BASE_URL = "https://api.deepseek.com"
DEEPSEEK_CHAT_COMPLETIONS_PATH = "/chat/completions"
DEEPSEEK_DEFAULT_MODEL = "deepseek-v4-flash"
DEEPSEEK_MODEL_PRICING_PER_MILLION: dict[str, dict[str, float]] = {
    "deepseek-v4-flash": {"input": 0.14, "cache_hit_input": 0.0028, "output": 0.28},
    "deepseek-v4-pro": {"input": 1.74, "cache_hit_input": 0.0174, "output": 3.48},
}
DEEPSEEK_MODELS: list[dict[str, Any]] = [
    {
        "id": model,
        "label": "DeepSeek V4 Flash" if model == "deepseek-v4-flash" else "DeepSeek V4 Pro",
        "pricing": pricing,
    }
    for model, pricing in DEEPSEEK_MODEL_PRICING_PER_MILLION.items()
]

RETRYABLE_HTTP_STATUS_CODES = {429, 500, 502, 503, 504}


class PromptBudgetError(RuntimeError):
    """Raised when a prompt exceeds the estimated token budget.

    Carries a stable ``code`` so that if it ever reaches the worker's fatal
    NDJSON line it reports a meaningful errorCode instead of UNKNOWN. Routine
    per-segment prompt overflows are handled by batch splitting / per-segment
    missing markers upstream, not by failing the task.
    """

    code = "PROMPT_BUDGET_EXCEEDED"
    required_action: str | None = None
    retry_after_seconds: int | None = None


class PodcastUpstreamError(RuntimeError):
    """Classified pipeline error carrying a stable host-facing ``code``.

    ``code`` is mapped to ``TaskErrorCode`` by the desktop host and
    ``required_action`` (e.g. ``"approve_budget"``) to ``RequiredAction``.
    Despite the class name it also carries ``LOCAL_*`` codes for
    transport/OS failures that never produced an upstream HTTP response —
    the code prefix, not the class, records the origin.
    """

    def __init__(
        self,
        code: str,
        message: str,
        retry_after_seconds: int | None = None,
        status: int | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.required_action: str | None = None
        self.retry_after_seconds = retry_after_seconds
        self.status = status


class PodcastLocalError(PodcastUpstreamError):
    """Local network/OS failure classified with a ``LOCAL_*`` code.

    Subclasses PodcastUpstreamError purely so existing
    ``except PodcastUpstreamError`` propagation sites forward it to the
    fatal NDJSON line unchanged; the ``LOCAL_*`` code keeps the host from
    mislabeling it as an upstream service failure.
    """


class PodcastBudgetExceededError(RuntimeError):
    """Estimated cumulative API spend exceeds the user-approved limit.

    Must propagate to the worker's fatal NDJSON line untouched: the host maps
    ``code`` → ``TaskErrorCode.BudgetConfirmationRequired`` → the
    ``ApproveBudget`` UI action, so folding this into a per-file failure or
    the generic TRANSCRIPTION_FAILED exit loses the user's approval gate.
    """

    code = "BUDGET_CONFIRMATION_REQUIRED"
    required_action = "approve_budget"
    retry_after_seconds: int | None = None


# Errors carrying a stable host-facing errorCode / requiredAction that must
# reach the worker's fatal NDJSON line untouched — pipeline stages must
# re-raise these instead of folding them into per-file results.
# (PromptBudgetError is deliberately absent: oversized prompts are split or
# marked per segment, never fatal.)
PROPAGATE_FATAL_ERRORS: tuple[type[BaseException], ...] = (
    PodcastUpstreamError,
    PodcastBudgetExceededError,
)

# Network-ish OSError subclasses → LOCAL_NETWORK (socket.herror is not defined
# on every platform).
_LOCAL_NETWORK_ERRORS: tuple[type[BaseException], ...] = tuple(
    error
    for error in (ConnectionError, socket.gaierror, getattr(socket, "herror", None))
    if isinstance(error, type)
)


BUDGET_CNY_PER_USD = 6.0
_BUDGET_LOCK = threading.Lock()


def budget_limit_cny(config: dict[str, Any] | None = None) -> float | None:
    config = config or {}
    raw = config.get("budget_limit_cny")
    if raw is None:
        raw = os.environ.get("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY")
    try:
        value = float(raw)
    except (TypeError, ValueError):
        return None
    return value if math.isfinite(value) and value >= 0 else None


def _budget_state_path() -> Path | None:
    raw = os.environ.get("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", "").strip()
    return Path(raw) if raw else None


def _read_budget_state(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {"spent_cny": 0.0, "reserved_cny": 0.0, "requests": 0}
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise PodcastBudgetExceededError(f"Budget ledger is unreadable: {error}") from error
    if not isinstance(value, dict):
        raise PodcastBudgetExceededError("Budget ledger is invalid")
    return value


def _write_budget_state(path: Path, state: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".partial")
    temporary.write_text(json.dumps(state, ensure_ascii=False, sort_keys=True), encoding="utf-8")
    os.replace(temporary, path)


def _request_cost_cny(prompt: str, config: dict[str, Any], retry_attempts: int) -> float:
    model = normalize_deepseek_model(config.get("model"))
    pricing = dict(DEEPSEEK_MODEL_PRICING_PER_MILLION.get(model, DEEPSEEK_MODEL_PRICING_PER_MILLION[DEEPSEEK_DEFAULT_MODEL]))
    pricing.update(config.get("pricing_per_million_tokens") or {})
    prompt_tokens = max(1, (len(prompt.encode("utf-8")) + 2) // 3)
    try:
        completion_tokens = max(1, int(config.get("max_tokens", config.get("num_predict", 2048))))
    except (TypeError, ValueError):
        completion_tokens = 2048
    completion_tokens = min(completion_tokens, 220_000)
    one_request_usd = (
        prompt_tokens * float(pricing["input"]) + completion_tokens * float(pricing["output"])
    ) / 1_000_000.0
    return one_request_usd * BUDGET_CNY_PER_USD * max(1, retry_attempts)


# ---- Reservation ledger ---------------------------------------------------
# A reservation is the worst-case cost of one API call (prompt + max output)
# times the allowed retry attempts. The ledger keeps one entry per reservation
# tagged with the owning PID so that:
#   * exceptions release the reservation instead of booking the N-times
#     reservation amount as spend;
#   * a killed worker (JobObject / TerminateProcess — no finally, no atexit)
#     leaves entries whose dead PID the next run sweeps, so `reserved_cny`
#     cannot leak across process restarts;
#   * interpreter exits release this process' leftover entries via atexit.
# `reserved_cny` is always rewritten as the sum of the live entries; ledger
# files written by older versions without per-entry tracking are reclaimed by
# the sweep (an untracked residue cannot be attributed to a live process).

_RESERVATION_SEQ = itertools.count(1)
# rid -> amount for reservations created by *this* process (under _BUDGET_LOCK).
_PROCESS_RESERVATIONS: dict[str, float] = {}

# Spend/reservations are bucketed per task run. The approved limit is a
# *per-task* number (TaskSpec.options.budgetLimitCny), so a shared ledger
# file must not let one task's historical spend consume the next task's
# approval. Runs without an explicit id (direct CLI use) share one bucket.
_UNSCOPED_RUN = "__unscoped__"


def _current_run_key() -> str:
    return os.environ.get("PODCAST_TRANSCRIBER_RUN_ID", "").strip() or _UNSCOPED_RUN


def _run_spend(state: dict[str, Any], run_key: str) -> float:
    by_run = state.get("spent_by_run")
    if isinstance(by_run, dict):
        try:
            return float(by_run.get(run_key) or 0.0)
        except (TypeError, ValueError):
            return 0.0
    return 0.0


def _run_reserved(live: dict[str, dict[str, Any]], run_key: str) -> float:
    return round(
        sum(
            float(entry.get("amount") or 0.0)
            for entry in live.values()
            if entry.get("run") == run_key
        ),
        8,
    )


def _pid_alive(pid: Any) -> bool:
    """Best-effort liveness check; conservative (True) when undecidable."""
    try:
        pid_value = int(pid)
    except (TypeError, ValueError):
        return False
    if pid_value <= 0:
        return False
    if pid_value == os.getpid():
        return True
    if os.name == "nt":
        # os.kill(pid, 0) is NOT a probe on Windows — any signal value would
        # TerminateProcess the target. Use OpenProcess + GetExitCodeProcess.
        try:
            import ctypes

            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            handle = kernel32.OpenProcess(0x1000, False, pid_value)  # PROCESS_QUERY_LIMITED_INFORMATION
            if not handle:
                # ERROR_ACCESS_DENIED (5): process exists but owned by someone else.
                return ctypes.get_last_error() == 5
            try:
                exit_code = ctypes.c_ulong(0)
                if not kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code)):
                    return True
                return exit_code.value == 259  # STILL_ACTIVE
            finally:
                kernel32.CloseHandle(handle)
        except Exception:
            return True
    try:
        os.kill(pid_value, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False
    return True


def _sweep_reservations(state: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Drop entries owned by dead PIDs and re-derive ``reserved_cny``.

    Must be called with ``_BUDGET_LOCK`` held. Also reclaims legacy
    ``reserved_cny`` residue left by pre-entry ledger files.
    """
    raw = state.get("reservations")
    live: dict[str, dict[str, Any]] = {}
    if isinstance(raw, dict):
        for rid, entry in raw.items():
            if not isinstance(entry, dict):
                continue
            try:
                amount = float(entry.get("amount") or 0.0)
                pid = int(entry.get("pid") or 0)
            except (TypeError, ValueError):
                continue
            if amount <= 0 or not _pid_alive(pid):
                continue
            run = entry.get("run")
            live[str(rid)] = {
                "pid": pid,
                "amount": amount,
                "run": str(run) if isinstance(run, str) and run else _UNSCOPED_RUN,
            }
    state["reservations"] = live
    state["reserved_cny"] = round(sum(entry["amount"] for entry in live.values()), 8)
    return live


def _drop_reservation_entry(state: dict[str, Any], live: dict[str, dict[str, Any]], reservation: float) -> None:
    """Release one of this process' reservations matching ``reservation``.

    Matching happens inside the *current* ledger's live entries — an equal
    amount reserved for a different ledger must not be mistaken for it.
    """
    rid = next(
        (
            key
            for key, entry in live.items()
            if entry.get("pid") == os.getpid() and abs(float(entry.get("amount") or 0.0) - reservation) < 1e-9
        ),
        None,
    )
    if rid is not None:
        live.pop(rid, None)
        _PROCESS_RESERVATIONS.pop(rid, None)
    state["reservations"] = live
    state["reserved_cny"] = round(sum(entry["amount"] for entry in live.values()), 8)


def reserve_budget(prompt: str, config: dict[str, Any], retry_attempts: int = 4) -> float:
    limit = budget_limit_cny(config)
    path = _budget_state_path()
    if limit is None or path is None:
        return 0.0
    reservation = _request_cost_cny(prompt, config, retry_attempts)
    rid = f"{os.getpid()}-{next(_RESERVATION_SEQ)}"
    run_key = _current_run_key()
    with _BUDGET_LOCK:
        state = _read_budget_state(path)
        live = _sweep_reservations(state)
        spent = _run_spend(state, run_key)
        reserved = _run_reserved(live, run_key)
        if spent + reserved + reservation > limit + 1e-9:
            _write_budget_state(path, state)  # persist the sweep reclaim
            raise PodcastBudgetExceededError(
                f"Estimated Podcast API budget exceeds approval: spent={spent:.6f} CNY, "
                f"reserved={reserved:.6f} CNY, next={reservation:.6f} CNY, limit={limit:.6f} CNY"
            )
        live[rid] = {"pid": os.getpid(), "amount": reservation, "run": run_key}
        state["reservations"] = live
        state["reserved_cny"] = round(
            sum(entry["amount"] for entry in live.values()), 8
        )
        _write_budget_state(path, state)
        _PROCESS_RESERVATIONS[rid] = reservation
    return reservation


def _usage_cost_cny(usage: dict[str, Any], config: dict[str, Any]) -> float:
    """Real billed cost for a completed call (cache-aware, per pricing table)."""
    model = normalize_deepseek_model(config.get("model"))
    pricing = dict(DEEPSEEK_MODEL_PRICING_PER_MILLION.get(model, DEEPSEEK_MODEL_PRICING_PER_MILLION[DEEPSEEK_DEFAULT_MODEL]))
    pricing.update(config.get("pricing_per_million_tokens") or {})
    prompt_tokens = int(usage.get("prompt_tokens") or 0)
    completion_tokens = int(usage.get("completion_tokens") or 0)
    cache_hit = int(usage.get("prompt_cache_hit_tokens") or usage.get("cache_hit_tokens") or 0)
    cache_miss = int(usage.get("prompt_cache_miss_tokens") or 0)
    uncached_input = cache_miss if cache_miss else max(0, prompt_tokens - cache_hit)
    return (
        cache_hit * float(pricing.get("cache_hit_input", pricing["input"]))
        + uncached_input * float(pricing["input"])
        + completion_tokens * float(pricing["output"])
    ) / 1_000_000.0 * BUDGET_CNY_PER_USD


def release_budget(reservation: float) -> None:
    """Return a reservation without billing — never raises."""
    path = _budget_state_path()
    if reservation <= 0 or path is None:
        return
    try:
        with _BUDGET_LOCK:
            state = _read_budget_state(path)
            live = _sweep_reservations(state)
            _drop_reservation_entry(state, live, reservation)
            _write_budget_state(path, state)
    except Exception as error:
        logging.getLogger(__name__).warning("Budget reservation release failed: %s", error)


def settle_budget(reservation: float, usage: dict[str, Any] | None, config: dict[str, Any]) -> None:
    """Release a reservation and, on success, bill the *real* token cost.

    ``usage=None`` means the call raised before/without a billable response —
    the reservation is released without spend (DeepSeek does not bill failed
    requests); it must not be booked as the N-times worst-case reservation.
    Best-effort: never raises into an active exception handler.
    """
    path = _budget_state_path()
    if reservation <= 0 or path is None:
        return
    try:
        actual = _usage_cost_cny(usage, config) if usage is not None else 0.0
        with _BUDGET_LOCK:
            state = _read_budget_state(path)
            live = _sweep_reservations(state)
            _drop_reservation_entry(state, live, reservation)
            if usage is not None:
                run_key = _current_run_key()
                by_run = state.get("spent_by_run")
                if not isinstance(by_run, dict):
                    by_run = {}
                by_run[run_key] = round(float(by_run.get(run_key) or 0.0) + actual, 8)
                state["spent_by_run"] = by_run
                # Global total kept for diagnostics only — the limit check
                # always uses the per-run bucket.
                state["spent_cny"] = round(float(state.get("spent_cny") or 0.0) + actual, 8)
                state["requests"] = int(state.get("requests") or 0) + 1
            _write_budget_state(path, state)
    except Exception as error:
        logging.getLogger(__name__).warning("Budget settle failed: %s", error)


def _release_process_reservations() -> None:
    """atexit: return every reservation this process still holds."""
    try:
        with _BUDGET_LOCK:
            if not _PROCESS_RESERVATIONS:
                return
            path = _budget_state_path()
            if path is None:
                _PROCESS_RESERVATIONS.clear()
                return
            state = _read_budget_state(path)
            live = _sweep_reservations(state)
            for rid in list(_PROCESS_RESERVATIONS):
                live.pop(rid, None)
            _PROCESS_RESERVATIONS.clear()
            state["reservations"] = live
            state["reserved_cny"] = round(sum(entry["amount"] for entry in live.values()), 8)
            _write_budget_state(path, state)
    except Exception:
        pass


atexit.register(_release_process_reservations)


def parse_retry_after(value: Any, now: datetime | None = None) -> int | None:
    if value is None:
        return None
    raw = str(value).strip()
    if not raw:
        return None
    try:
        return max(0, int(math.ceil(float(raw))))
    except (TypeError, ValueError):
        pass
    try:
        target = parsedate_to_datetime(raw)
        if target.tzinfo is None:
            target = target.replace(tzinfo=timezone.utc)
        current = now or datetime.now(timezone.utc)
        return max(0, int(math.ceil((target - current).total_seconds())))
    except (TypeError, ValueError, OverflowError):
        return None


def classify_upstream_error(exc: BaseException, service: str = "Upstream") -> PodcastUpstreamError | None:
    """Map an exception to a stable host-facing error code.

    ``UPSTREAM_*`` codes are reserved for real upstream API answers — an
    HTTPError means the server actually responded (401/429/5xx). Failures
    before a valid response exists — URLError transport errors, connect/DNS
    failures, timeouts, and bare ``OSError`` (local sockets, file IO, disk)
    — get ``LOCAL_*`` codes so the host never blames the upstream service
    for a local network/OS fault.
    """
    if isinstance(exc, urllib.error.HTTPError):
        status = int(exc.code)
        retry_after = parse_retry_after(exc.headers.get("Retry-After") if getattr(exc, "headers", None) else None)
        if status == 401:
            code = "UPSTREAM_UNAUTHORIZED"
        elif status == 429:
            code = "RATE_LIMITED"
        elif status >= 500:
            code = "UPSTREAM_UNAVAILABLE"
        else:
            return None
        detail = ""
        try:
            detail = exc.read().decode("utf-8", errors="replace").strip()[:500]
        except (OSError, UnicodeError):
            detail = ""
        message = f"{service} HTTP {status}"
        if detail:
            message = f"{message}: {detail}"
        return PodcastUpstreamError(code, message, retry_after, status)
    # Everything below never produced an upstream HTTP response → LOCAL_*.
    if isinstance(exc, urllib.error.URLError):
        reason = getattr(exc, "reason", None)
        if isinstance(reason, (TimeoutError, socket.timeout)):
            return PodcastLocalError("LOCAL_TIMEOUT", f"{service} connection timed out before a response: {reason}")
        return PodcastLocalError("LOCAL_NETWORK", f"{service} connection failed before a response: {reason}")
    if isinstance(exc, (TimeoutError, socket.timeout)):
        return PodcastLocalError("LOCAL_TIMEOUT", f"{service} local operation timed out: {exc}")
    if isinstance(exc, _LOCAL_NETWORK_ERRORS):
        return PodcastLocalError("LOCAL_NETWORK", f"{service} local network error: {exc}")
    if isinstance(exc, OSError):
        return PodcastLocalError("LOCAL_IO", f"{service} local OS error: {exc}")
    return None


def is_retryable_http_error(exc: BaseException) -> bool:
    """Return True if the exception represents a transient error worth retrying."""
    if isinstance(exc, urllib.error.HTTPError):
        return exc.code in RETRYABLE_HTTP_STATUS_CODES
    if isinstance(exc, urllib.error.URLError):
        reason = getattr(exc, "reason", None)
        if isinstance(reason, (TimeoutError, ConnectionResetError, ConnectionRefusedError, OSError)):
            return True
        if reason is None:
            return True
    if isinstance(exc, (TimeoutError, ConnectionResetError, ConnectionRefusedError)):
        return True
    return False


def deepseek_chat_completions_url(base_url: Any) -> str:
    """Return the concrete ChatCompletions endpoint for DeepSeek."""
    raw = str(base_url or DEEPSEEK_DEFAULT_BASE_URL).strip().rstrip("/")
    if raw.endswith(DEEPSEEK_CHAT_COMPLETIONS_PATH):
        return raw
    return f"{raw}{DEEPSEEK_CHAT_COMPLETIONS_PATH}"


def normalize_deepseek_model(model: Any) -> str:
    value = str(model or "").strip()
    if value in DEEPSEEK_MODEL_PRICING_PER_MILLION:
        return value
    return DEEPSEEK_DEFAULT_MODEL


def deepseek_thinking_config(config: dict[str, Any]) -> dict[str, str]:
    raw = config.get("think", False)
    enabled = raw if isinstance(raw, bool) else str(raw).strip().lower() in {"1", "true", "yes", "on"}
    return {"type": "enabled" if enabled else "disabled"}
