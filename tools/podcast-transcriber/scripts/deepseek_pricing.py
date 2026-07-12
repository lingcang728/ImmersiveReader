from __future__ import annotations

import urllib.error
import socket
import json
import os
import threading
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
import math
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
    """Raised when a prompt exceeds the estimated token budget."""


class PodcastUpstreamError(RuntimeError):
    def __init__(
        self,
        code: str,
        message: str,
        retry_after_seconds: int | None = None,
        status: int | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.retry_after_seconds = retry_after_seconds
        self.status = status


class PodcastBudgetExceededError(RuntimeError):
    code = "BUDGET_CONFIRMATION_REQUIRED"


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


def reserve_budget(prompt: str, config: dict[str, Any], retry_attempts: int = 4) -> float:
    limit = budget_limit_cny(config)
    path = _budget_state_path()
    if limit is None or path is None:
        return 0.0
    reservation = _request_cost_cny(prompt, config, retry_attempts)
    with _BUDGET_LOCK:
        state = _read_budget_state(path)
        spent = float(state.get("spent_cny") or 0.0)
        reserved = float(state.get("reserved_cny") or 0.0)
        if spent + reserved + reservation > limit + 1e-9:
            raise PodcastBudgetExceededError(
                f"Estimated Podcast API budget exceeds approval: spent={spent:.6f} CNY, "
                f"reserved={reserved:.6f} CNY, next={reservation:.6f} CNY, limit={limit:.6f} CNY"
            )
        state["reserved_cny"] = round(reserved + reservation, 8)
        _write_budget_state(path, state)
    return reservation


def settle_budget(reservation: float, usage: dict[str, Any] | None, config: dict[str, Any]) -> None:
    path = _budget_state_path()
    if reservation <= 0 or path is None:
        return
    if usage is None:
        actual = reservation
    else:
        model = normalize_deepseek_model(config.get("model"))
        pricing = dict(DEEPSEEK_MODEL_PRICING_PER_MILLION.get(model, DEEPSEEK_MODEL_PRICING_PER_MILLION[DEEPSEEK_DEFAULT_MODEL]))
        pricing.update(config.get("pricing_per_million_tokens") or {})
        prompt_tokens = int(usage.get("prompt_tokens") or 0)
        completion_tokens = int(usage.get("completion_tokens") or 0)
        actual = (
            prompt_tokens * float(pricing["input"]) + completion_tokens * float(pricing["output"])
        ) / 1_000_000.0 * BUDGET_CNY_PER_USD
    with _BUDGET_LOCK:
        state = _read_budget_state(path)
        reserved = max(0.0, float(state.get("reserved_cny") or 0.0) - reservation)
        state["reserved_cny"] = round(reserved, 8)
        state["spent_cny"] = round(float(state.get("spent_cny") or 0.0) + actual, 8)
        state["requests"] = int(state.get("requests") or 0) + 1
        _write_budget_state(path, state)


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
    if isinstance(exc, urllib.error.URLError):
        reason = getattr(exc, "reason", None)
        if isinstance(reason, (TimeoutError, socket.timeout)):
            return PodcastUpstreamError("UPSTREAM_TIMEOUT", f"{service} request timed out: {reason}")
        return PodcastUpstreamError("UPSTREAM_UNAVAILABLE", f"{service} network error: {reason}")
    if isinstance(exc, (TimeoutError, socket.timeout)):
        return PodcastUpstreamError("UPSTREAM_TIMEOUT", f"{service} request timed out: {exc}")
    if isinstance(exc, (ConnectionResetError, ConnectionRefusedError, OSError)):
        return PodcastUpstreamError("UPSTREAM_UNAVAILABLE", f"{service} network error: {exc}")
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
