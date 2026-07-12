from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sys
from pathlib import Path
from typing import Any

TASK_ID = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
COMPATIBILITY_FIELDS = (
    "inputSha256",
    "pipelineVersion",
    "engineVersion",
    "configHash",
    "modelHash",
)


class TaskSpecError(RuntimeError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


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
    if spec.get("schemaVersion") != 1:
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
        except (TypeError, ValueError):
            raise TaskSpecError("INVALID_TASK_SPEC", "Budget limit must be finite")
        if not math.isfinite(budget_limit_value) or budget_limit_value < 0:
            raise TaskSpecError("INVALID_TASK_SPEC", "Budget limit must be non-negative")
        if not math.isfinite(estimated_budget) or estimated_budget < 0 or budget_limit_value + 1e-9 < estimated_budget:
            raise TaskSpecError("BUDGET_CONFIRMATION_REQUIRED", "Budget limit is below the verified estimate")
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
        print(json.dumps({"type": "fatal", "errorCode": code, "message": str(error)}), file=sys.stderr)
        return 2
    os.environ["PODCAST_TRANSCRIBER_RUN_ID"] = spec["taskId"]
    import transcribe_podcasts
    from deepseek_pricing import PodcastBudgetExceededError, PodcastUpstreamError, classify_upstream_error

    options = spec.get("options") or {}
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

    sys.argv = ["transcribe_podcasts.py", "--force", "--no-open-output"]
    try:
        return transcribe_podcasts.main()
    except Exception as error:
        classified = error if isinstance(error, (PodcastBudgetExceededError, PodcastUpstreamError)) else classify_upstream_error(error)
        if classified is not None:
            payload = {
                "type": "fatal",
                "errorCode": getattr(classified, "code", "UNKNOWN"),
                "message": str(classified),
            }
            retry_after = getattr(classified, "retry_after_seconds", None)
            if retry_after is not None:
                payload["retryAfterSeconds"] = retry_after
        else:
            payload = {"type": "fatal", "errorCode": "UNKNOWN", "message": str(error)}
        print(json.dumps(payload, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
