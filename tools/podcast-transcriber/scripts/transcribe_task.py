from __future__ import annotations

import argparse
import hashlib
import json
import logging
import math
import os
import re
import sys
from pathlib import Path
from typing import Any

# P0-2: vendored Python (< PEP-686) defaults pipe stdio to the console
# codepage (cp936 on zh-CN Windows), while the host reads our pipes as
# UTF-8 — the first non-ASCII byte would kill the reader thread. Force
# both streams to UTF-8 before anything is printed. Best-effort only:
# streams without reconfigure (StringIO, swapped/captured stdio, None)
# are left untouched.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

from podcast_transcriber.common import strip_host_paths  # noqa: E402

TASK_ID = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
COMPATIBILITY_FIELDS = (
    "inputSha256",
    "pipelineVersion",
    "engineVersion",
    "configHash",
    "modelHash",
)


class TaskSpecError(RuntimeError):
    def __init__(self, code: str, message: str, required_action: str | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.required_action = required_action


# ---- Fatal NDJSON contract (P1-28 / P2-28) ---------------------------------
# The desktop host treats the LAST stderr line as the task's last_error and
# parses a JSON object on it:
#   {"type": "fatal", "errorCode": "...", "message": "...",
#    "retryAfterSeconds"?: int, "requiredAction"?: "approve_budget"}
# - type/errorCode/message are mandatory and stable; message is the human-
#   readable text the UI displays (never raw JSON).
# - errorCode maps onto TaskErrorCode (e.g. BUDGET_CONFIRMATION_REQUIRED →
#   RequiredAction::ApproveBudget); unmapped codes degrade to Unknown while
#   the message is still shown.
# - retryAfterSeconds / requiredAction are optional extras the host reads when
#   present.
# Every non-zero exit must leave exactly one such line as the final stderr
# output, printed AFTER the pipeline has gone quiet. Both stdio streams are
# UTF-8-reconfigured at module top (P0-2), so ensure_ascii=False output cannot
# emit cp936 bytes and kill the host's pipe reader.
_EXIT_FATAL = {
    1: ("TRANSCRIPTION_FAILED", "转写流水线存在失败项"),
    2: ("ENGINE_UNAVAILABLE", "未找到 ffmpeg，转写引擎不可用"),
    3: ("MODEL_LOAD_FAILED", "语音模型/推理运行时加载失败"),
    4: ("ENGINE_BUSY", "检测到另一个转写进程仍持有运行锁"),
}


# Error codes (MODEL_INCOMPATIBLE, PATH_OUTSIDE_MANAGED_ROOT, …) must be
# stripped before keyword-matching a log line to a stage: otherwise a fatal
# payload or error line containing e.g. "MODEL_INCOMPATIBLE" hits the
# "model" needle and the task's stage is mistranslated to load_model
# (P3-27). The stage whitelist below only sees prose after this scrub.
_ERROR_CODE_RE = re.compile(r"\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b")


def _stage_haystack(message: str) -> str:
    """Return ``message`` lowercased with SCREAMING error codes removed."""
    return _ERROR_CODE_RE.sub(" ", message).lower()


def _exit_fatal_payload(code: int, summary: dict[str, Any] | None, stage: str | None = None) -> dict[str, Any]:
    """Build the terminal fatal NDJSON for a non-zero pipeline exit code.

    ``stage`` echoes the last reported pipeline stage so the host's JSON
    stage branch wins over its text heuristics (a bare fatal line like
    ``MODEL_INCOMPATIBLE`` would otherwise parse as ``load_model``).
    """
    error_code, base = _EXIT_FATAL.get(code, ("TRANSCRIPTION_FAILED", f"转写流水线退出码 {code}"))
    details: list[str] = []
    for item in (summary or {}).get("results", []):
        if isinstance(item, dict) and item.get("status") == "failed":
            detail = str(item.get("file") or "?")
            if item.get("error"):
                detail += f": {item['error']}"
            details.append(detail)
    details.extend(str(failure) for failure in (summary or {}).get("failures", [])[-3:])
    message = base if not details else f"{base}；" + "；".join(details)
    # The fatal line is the host's last_error: basenames only, never
    # absolute host paths out of exception text (P3-27).
    message = strip_host_paths(message)
    payload: dict[str, Any] = {"type": "fatal", "errorCode": error_code, "message": message[:480]}
    if stage:
        payload["stage"] = stage
    return payload


def _managed_root(environment: dict[str, str], name: str) -> Path:
    value = environment.get(name, "").strip()
    if not value:
        raise TaskSpecError("PATH_OUTSIDE_MANAGED_ROOT", f"{name} is required")
    path = Path(value)
    if not path.is_absolute():
        raise TaskSpecError("PATH_OUTSIDE_MANAGED_ROOT", f"{name} must be absolute")
    return path.resolve()


def _under(root: Path, relative: str) -> Path:
    value = Path(relative)
    if value.is_absolute() or ".." in value.parts:
        raise TaskSpecError("PATH_OUTSIDE_MANAGED_ROOT", "TaskSpec path must be relative")
    candidate = (root / value).resolve()
    try:
        candidate.relative_to(root)
    except ValueError as error:
        raise TaskSpecError("PATH_OUTSIDE_MANAGED_ROOT", "TaskSpec path escaped its managed root") from error
    return candidate


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _required_string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise TaskSpecError("INVALID_TASK_SPEC", f"{field} is required")
    return value.strip()


def _verify_recovery(task_root: Path, compatibility: dict[str, str], input_sha256: str) -> None:
    recovery_path = task_root / "recovery.json"
    if not recovery_path.exists():
        return
    recovery = json.loads(recovery_path.read_text(encoding="utf-8-sig"))
    saved = recovery.get("compatibility") or {}
    expected = {"inputSha256": input_sha256, **compatibility}
    for field in COMPATIBILITY_FIELDS:
        if saved.get(field) != expected[field]:
            code = {
                "inputSha256": "INPUT_CHANGED",
                "pipelineVersion": "PIPELINE_INCOMPATIBLE",
                "engineVersion": "PIPELINE_INCOMPATIBLE",
                "configHash": "CONFIG_INCOMPATIBLE",
                "modelHash": "MODEL_INCOMPATIBLE",
            }[field]
            raise TaskSpecError(code, f"Recovery field is incompatible: {field}")


def load_task_spec(path: Path, environment: dict[str, str] | None = None) -> dict[str, Any]:
    environment = dict(os.environ if environment is None else environment)
    data_root = _managed_root(environment, "IMMERSIVE_PODCAST_DATA_ROOT")
    cache_root = _managed_root(environment, "IMMERSIVE_PODCAST_CACHE_ROOT")
    library_root = _managed_root(environment, "IMMERSIVE_LIBRARY_ROOT")
    resolved_spec = path.resolve()
    try:
        resolved_spec.relative_to(data_root)
    except ValueError as error:
        raise TaskSpecError("PATH_OUTSIDE_MANAGED_ROOT", "TaskSpec must be inside Podcast Data") from error
    spec = json.loads(resolved_spec.read_text(encoding="utf-8-sig"))
    schema_version = spec.get("schemaVersion")
    if schema_version not in (1, 2):
        raise TaskSpecError("INVALID_TASK_SPEC", "Unsupported TaskSpec schemaVersion")
    task_id = _required_string(spec.get("taskId"), "taskId")
    if not TASK_ID.fullmatch(task_id) or resolved_spec.parent.name != task_id:
        raise TaskSpecError("INVALID_TASK_SPEC", "taskId does not match the managed task directory")
    input_spec = spec.get("input") or {}
    relative_input = _required_string(input_spec.get("relativePath"), "input.relativePath")
    input_path = _under(cache_root, relative_input)
    if not input_path.is_file():
        raise TaskSpecError("INPUT_CHANGED", "Managed input file is missing")
    expected_bytes = input_spec.get("bytes")
    if not isinstance(expected_bytes, int) or expected_bytes < 0 or input_path.stat().st_size != expected_bytes:
        raise TaskSpecError("INPUT_CHANGED", "Managed input size changed")
    input_sha256 = _required_string(input_spec.get("inputSha256"), "input.inputSha256").lower()
    if _sha256(input_path) != input_sha256:
        raise TaskSpecError("INPUT_CHANGED", "Managed input SHA-256 changed")
    compatibility_spec = spec.get("compatibility") or {}
    compatibility = {
        field: _required_string(compatibility_spec.get(field), f"compatibility.{field}")
        for field in COMPATIBILITY_FIELDS
        if field != "inputSha256"
    }
    publish = spec.get("publish") or {}
    _under(library_root, _required_string(publish.get("incomingRelativePath"), "publish.incomingRelativePath"))
    options = spec.get("options") or {}
    budget = spec.get("budget") or {}
    budget_limit = options.get("budgetLimitCny", options.get("maxApiCostCny"))
    if budget_limit is not None:
        try:
            budget_limit_value = float(budget_limit)
            estimated_budget = float(budget.get("estimatedApiCostUpperCny", 0.0))
        except (TypeError, ValueError) as err:
            raise TaskSpecError("INVALID_TASK_SPEC", "Budget limit must be finite") from err
        if not math.isfinite(budget_limit_value) or budget_limit_value < 0:
            raise TaskSpecError("INVALID_TASK_SPEC", "Budget limit must be non-negative")
        if not math.isfinite(estimated_budget) or estimated_budget < 0 or budget_limit_value + 1e-9 < estimated_budget:
            raise TaskSpecError(
                "BUDGET_CONFIRMATION_REQUIRED",
                "Budget limit is below the verified estimate",
                required_action="approve_budget",
            )
    _verify_recovery(resolved_spec.parent, compatibility, input_sha256)
    spec["resolvedInputPath"] = str(input_path)
    return spec


def main() -> int:
    parser = argparse.ArgumentParser(description="Run one managed Podcast transcription task")
    parser.add_argument("--task-spec", required=True, type=Path)
    args = parser.parse_args()
    try:
        spec = load_task_spec(args.task_spec)
    except (OSError, ValueError, json.JSONDecodeError, TaskSpecError) as error:
        code = error.code if isinstance(error, TaskSpecError) else "INVALID_TASK_SPEC"
        payload: dict[str, Any] = {
            "type": "fatal",
            "errorCode": code,
            "message": strip_host_paths(str(error)),
            # The spec-validation failure happens during the prepare band;
            # pin it so host text heuristics cannot remap e.g.
            # MODEL_INCOMPATIBLE to load_model (P3-27).
            "stage": "prepare",
        }
        required_action = getattr(error, "required_action", None)
        if required_action:
            payload["requiredAction"] = required_action
        print(json.dumps(payload), file=sys.stderr, flush=True)
        return 2
    os.environ["PODCAST_TRANSCRIBER_RUN_ID"] = spec["taskId"]
    # Pin the pipeline to the TaskSpec-validated input instead of scanning
    # the whole inbox (P3-27): resolvedInputPath was verified (exists,
    # size, SHA-256) above and is the only file this task may process.
    os.environ["PODCAST_TRANSCRIBER_INPUT_FILE"] = spec["resolvedInputPath"]
    import transcribe_podcasts
    from deepseek_pricing import PodcastBudgetExceededError, PodcastUpstreamError, classify_upstream_error

    options = spec.get("options") or {}
    # Force-apply TaskSpec.options.translate into the runtime translation gate.
    # false: never call the translation service (local normalize + polish still run).
    # true: translate only non-Chinese (en/mixed) segments.
    if "translate" in options:
        os.environ["PODCAST_TRANSCRIBER_FORCE_TRANSLATE"] = "1" if options.get("translate") else "0"
    # polish: schema v1 / missing field defaults to true for compatibility.
    polish_enabled = True if "polish" not in options else bool(options.get("polish"))
    os.environ["PODCAST_TRANSCRIBER_FORCE_POLISH"] = "1" if polish_enabled else "0"
    budget_limit = options.get("budgetLimitCny", options.get("maxApiCostCny"))
    try:
        budget_limit_value = float(budget_limit)
    except (TypeError, ValueError):
        budget_limit_value = None
    if budget_limit_value is not None and budget_limit_value >= 0:
        os.environ["PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY"] = str(budget_limit_value)
        cache_root = Path(os.environ["IMMERSIVE_PODCAST_CACHE_ROOT"]).resolve()
        os.environ["PODCAST_TRANSCRIBER_BUDGET_STATE_PATH"] = str(
            cache_root / "work" / "state" / "budget.json"
        )

    # Default: resume from completed chunks / translation batches / output checkpoints.
    # Explicit "restart from scratch" paths pass --force via a dedicated entrypoint.
    from podcast_transcriber.progress_emit import get_emitter, report_stage_progress  # noqa: E402

    # Last stage surfaced to the host — echoed back on the fatal line so the
    # host's JSON stage branch wins over its text heuristics (P3-27).
    last_stage: dict[str, str] = {"stage": "prepare"}

    _emitter = get_emitter()
    _emitter_emit = _emitter.emit
    _emitter_heartbeat = _emitter.heartbeat

    def _tracked_emit(**kwargs: Any) -> None:
        if kwargs.get("stage"):
            last_stage["stage"] = str(kwargs["stage"])
        _emitter_emit(**kwargs)

    def _tracked_heartbeat(stage: str, message: str | None = None) -> None:
        if stage:
            last_stage["stage"] = str(stage)
        _emitter_heartbeat(stage, message)

    _emitter.emit = _tracked_emit  # type: ignore[method-assign]
    _emitter.heartbeat = _tracked_heartbeat  # type: ignore[method-assign]

    def emit(payload: dict[str, Any]) -> None:
        """Structured NDJSON for the desktop worker consumer (no secrets/full paths)."""
        safe = {
            key: value
            for key, value in payload.items()
            if key
            in {
                "type",
                "stage",
                "percent",
                "completedUnits",
                "totalUnits",
                "unit",
                "message",
                "errorCode",
                "retryAfterSeconds",
                "requiredAction",
            }
        }
        if safe.get("stage"):
            last_stage["stage"] = str(safe["stage"])
        print(json.dumps(safe, ensure_ascii=False), flush=True)

    # Unmeasurable prepare stage: heartbeat only — do not invent a percent.
    report_stage_progress("prepare", message="任务规格已校验，开始转写流水线", force=True)
    get_emitter().heartbeat("prepare", "worker alive")

    # Bridge polish callbacks (and other stage reporters) onto throttled NDJSON.
    def _bridge_units(stage: str, done: int, total: int, unit: str) -> None:
        report_stage_progress(stage, completed=done, total=total, unit=unit)

    # Install a lightweight logger hook so stage lines also surface as NDJSON.
    # Prefer structured unit reports from the bridge; log hook is fallback only.
    class _NdjsonHandler(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:  # noqa: A003
            message = record.getMessage()
            stripped = message.strip()
            # A log record that is itself a JSON document (structured error
            # dumps, API bodies) is not prose: keyword-matching it could map
            # embedded tokens ("model", "translate") to a bogus stage.
            if stripped.startswith("{"):
                return
            stage = "working"
            lower = _stage_haystack(message)
            if "polish" in lower:
                stage = "polishing"
            elif "chunk" in lower:
                stage = "chunking"
            elif "transcrib" in lower:
                stage = "transcribing"
            elif "translat" in lower:
                stage = "translating"
            elif "normal" in lower:
                # Canonical stage name — Rust maps "normalizing"; the bare
                # "normalize" used to fall into the catch-all band (P3-27).
                stage = "normalizing"
            elif "model" in lower:
                stage = "load_model"
            elif "publish" in lower or "output" in lower:
                stage = "writing_output"
            # Do not invent percent from log tokens; only stage/heartbeat.
            # Host-visible: scrub absolute paths from the log line (P3-27).
            try:
                get_emitter().heartbeat(stage, strip_host_paths(message[:180]))
            except Exception:
                pass

    logging.getLogger().addHandler(_NdjsonHandler())

    # Expose unit bridge for polish_interview_markdown progress callback.
    def _polish_bridge(done: int, total: int) -> None:
        _bridge_units("polishing", done, total, "块")

    import polish_interview_markdown as _pim  # noqa: E402

    if hasattr(_pim, "set_polish_progress_reporter"):
        _pim.set_polish_progress_reporter(_polish_bridge)

    sys.argv = ["transcribe_podcasts.py", "--no-open-output"]
    try:
        code = transcribe_podcasts.main()
        if code == 0:
            emit(
                {
                    "type": "completed",
                    "stage": "completed",
                    "percent": 100,
                    "message": "转写流水线完成",
                }
            )
        else:
            # Last stderr line wins as the host's last_error — emit the fatal
            # line after the pipeline has gone quiet so it is not overwritten.
            summary = getattr(transcribe_podcasts, "LAST_RUN_SUMMARY", None)
            print(
                json.dumps(_exit_fatal_payload(code, summary, stage=last_stage["stage"]), ensure_ascii=False),
                file=sys.stderr,
                flush=True,
            )
        return code
    except Exception as error:
        # Classified errors (upstream/local/budget) keep their stable errorCode
        # and any requiredAction for the host UI; anything else carrying a
        # ``code`` attribute (e.g. PromptBudgetError) also passes through.
        classified = error if isinstance(error, (PodcastBudgetExceededError, PodcastUpstreamError)) else classify_upstream_error(error)
        if classified is None and getattr(error, "code", None):
            classified = error
        if classified is not None:
            payload = {
                "type": "fatal",
                "errorCode": getattr(classified, "code", "UNKNOWN"),
                "message": strip_host_paths(str(classified)),
                "stage": last_stage["stage"],
            }
            retry_after = getattr(classified, "retry_after_seconds", None)
            if retry_after is not None:
                payload["retryAfterSeconds"] = retry_after
            required_action = getattr(classified, "required_action", None)
            if required_action:
                payload["requiredAction"] = required_action
        else:
            payload = {
                "type": "fatal",
                "errorCode": "UNKNOWN",
                "message": strip_host_paths(str(error)),
                "stage": last_stage["stage"],
            }
        print(json.dumps(payload, ensure_ascii=False), file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
