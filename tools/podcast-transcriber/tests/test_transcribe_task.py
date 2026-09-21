from __future__ import annotations

import hashlib
import json
import sys
import types
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from transcribe_task import TaskSpecError, _exit_fatal_payload, load_task_spec  # noqa: E402


def fixture(tmp_path: Path) -> tuple[Path, dict[str, str]]:
    data_root = tmp_path / "Data" / "Podcast"
    cache_root = tmp_path / "Cache" / "Podcast" / "Tasks" / "task-1"
    library_root = tmp_path / "Library"
    task_root = data_root / "Tasks" / "task-1"
    task_root.mkdir(parents=True)
    input_path = cache_root / "input" / "sample.mp3"
    input_path.parent.mkdir(parents=True)
    input_path.write_bytes(b"audio-fixture")
    environment = {
        "IMMERSIVE_PODCAST_DATA_ROOT": str(data_root),
        "IMMERSIVE_PODCAST_CACHE_ROOT": str(cache_root),
        "IMMERSIVE_LIBRARY_ROOT": str(library_root),
    }
    spec = {
        "schemaVersion": 1,
        "taskId": "task-1",
        "input": {
            "relativePath": "input/sample.mp3",
            "inputSha256": hashlib.sha256(b"audio-fixture").hexdigest(),
            "bytes": len(b"audio-fixture"),
            "durationSeconds": 1,
        },
        "compatibility": {
            "pipelineVersion": "pipeline-1",
            "engineVersion": "engine-1",
            "configHash": "config-1",
            "modelHash": "model-1",
        },
        "publish": {
            "bookId": "podcast:sha",
            "sourceId": "sha",
            "revision": 1,
            "incomingRelativePath": ".incoming/transaction-1",
        },
    }
    path = task_root / "task.json"
    path.write_text(json.dumps(spec), encoding="utf-8")
    return path, environment


def test_single_task_spec_accepts_only_managed_verified_input(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)

    loaded = load_task_spec(path, environment)

    assert loaded["taskId"] == "task-1"
    assert Path(loaded["resolvedInputPath"]).read_bytes() == b"audio-fixture"


def test_single_task_spec_rejects_path_traversal(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)
    spec = json.loads(path.read_text(encoding="utf-8"))
    spec["input"]["relativePath"] = "../outside.mp3"
    path.write_text(json.dumps(spec), encoding="utf-8")

    try:
        load_task_spec(path, environment)
    except TaskSpecError as error:
        assert error.code == "PATH_OUTSIDE_MANAGED_ROOT"
    else:
        raise AssertionError("path traversal must be rejected")


def test_single_task_spec_rejects_changed_input_hash(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)
    input_path = Path(environment["IMMERSIVE_PODCAST_CACHE_ROOT"]) / "input" / "sample.mp3"
    input_path.write_bytes(b"changed-audio")

    try:
        load_task_spec(path, environment)
    except TaskSpecError as error:
        assert error.code == "INPUT_CHANGED"
    else:
        raise AssertionError("changed input must be rejected")


def test_single_task_spec_rejects_incompatible_recovery(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)
    recovery = {
        "compatibility": {
            "inputSha256": hashlib.sha256(b"audio-fixture").hexdigest(),
            "pipelineVersion": "old-pipeline",
            "engineVersion": "engine-1",
            "configHash": "config-1",
            "modelHash": "model-1",
        }
    }
    (path.parent / "recovery.json").write_text(json.dumps(recovery), encoding="utf-8")

    try:
        load_task_spec(path, environment)
    except TaskSpecError as error:
        assert error.code == "PIPELINE_INCOMPATIBLE"
    else:
        raise AssertionError("incompatible recovery must be rejected")


def test_single_task_spec_rejects_budget_below_verified_estimate(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)
    spec = json.loads(path.read_text(encoding="utf-8"))
    spec["options"] = {"maxApiCostCny": 0.0, "budgetLimitCny": 0.0}
    spec["budget"] = {"estimatedApiCostUpperCny": 0.1}
    path.write_text(json.dumps(spec), encoding="utf-8")

    try:
        load_task_spec(path, environment)
    except TaskSpecError as error:
        assert error.code == "BUDGET_CONFIRMATION_REQUIRED"
    else:
        raise AssertionError("budget below verified estimate must be rejected")


def test_task_spec_preserves_translate_option(tmp_path: Path) -> None:
    path, environment = fixture(tmp_path)
    spec = json.loads(path.read_text(encoding="utf-8"))
    spec["options"] = {"translate": False, "maxApiCostCny": 1.0, "budgetLimitCny": 1.0}
    spec["budget"] = {"estimatedApiCostUpperCny": 0.1}
    path.write_text(json.dumps(spec), encoding="utf-8")

    loaded = load_task_spec(path, environment)
    assert loaded["options"]["translate"] is False

    spec["options"]["translate"] = True
    path.write_text(json.dumps(spec), encoding="utf-8")
    loaded = load_task_spec(path, environment)
    assert loaded["options"]["translate"] is True


def _install_fake_pipeline(monkeypatch, exit_code: int, summary: dict | None = None) -> None:
    """Stub the in-process pipeline + optional imports so main() is hermetic."""
    fake_worker = types.ModuleType("transcribe_podcasts")
    fake_worker.main = lambda: exit_code
    fake_worker.LAST_RUN_SUMMARY = summary

    fake_pricing = types.ModuleType("deepseek_pricing")

    class _UpstreamError(Exception):
        pass

    class _BudgetExceededError(Exception):
        pass

    class _SecretMissingError(Exception):
        pass

    fake_pricing.PodcastUpstreamError = _UpstreamError
    fake_pricing.PodcastBudgetExceededError = _BudgetExceededError
    fake_pricing.PodcastSecretMissingError = _SecretMissingError
    fake_pricing.classify_upstream_error = lambda error: None

    fake_pim = types.ModuleType("polish_interview_markdown")

    monkeypatch.setitem(sys.modules, "transcribe_podcasts", fake_worker)
    monkeypatch.setitem(sys.modules, "deepseek_pricing", fake_pricing)
    monkeypatch.setitem(sys.modules, "polish_interview_markdown", fake_pim)


def test_exit_fatal_payload_maps_exit_codes() -> None:
    expected = {
        1: "TRANSCRIPTION_FAILED",
        2: "ENGINE_UNAVAILABLE",
        3: "MODEL_LOAD_FAILED",
        4: "ENGINE_BUSY",
        99: "TRANSCRIPTION_FAILED",
    }
    for code, error_code in expected.items():
        payload = _exit_fatal_payload(code, None)
        assert payload["type"] == "fatal"
        assert payload["errorCode"] == error_code
        assert payload["message"]

    summary = {
        "results": [{"file": "a.mp3", "status": "failed", "error": "boom"}],
        "failures": ["model=x device=cpu: nope"],
    }
    payload = _exit_fatal_payload(1, summary)
    assert "a.mp3" in payload["message"]
    assert "boom" in payload["message"]
    assert len(payload["message"]) <= 480


def test_main_emits_fatal_ndjson_on_nonzero_exit(tmp_path: Path, monkeypatch, capsys) -> None:
    path, environment = fixture(tmp_path)
    for key, value in environment.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["transcribe_task.py", "--task-spec", str(path)])
    _install_fake_pipeline(
        monkeypatch,
        3,
        {
            "results": [{"file": "model", "status": "failed", "error": "no runtime"}],
            "failures": [],
        },
    )

    from transcribe_task import main

    assert main() == 3
    captured = capsys.readouterr()
    stderr_lines = [line for line in captured.err.splitlines() if line.strip()]
    assert stderr_lines, "non-zero exit must leave a fatal line on stderr"
    fatal = json.loads(stderr_lines[-1])
    assert fatal["type"] == "fatal"
    assert fatal["errorCode"] == "MODEL_LOAD_FAILED"
    assert "no runtime" in fatal["message"]


def test_main_emits_completed_without_fatal_on_success(tmp_path: Path, monkeypatch, capsys) -> None:
    path, environment = fixture(tmp_path)
    for key, value in environment.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["transcribe_task.py", "--task-spec", str(path)])
    _install_fake_pipeline(monkeypatch, 0)

    from transcribe_task import main

    assert main() == 0
    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    last = json.loads(stdout_lines[-1])
    assert last["type"] == "completed"
    assert not any('"type": "fatal"' in line or '"type":"fatal"' in line for line in captured.err.splitlines())


def test_main_fatal_keeps_budget_code_and_required_action(tmp_path: Path, monkeypatch, capsys) -> None:
    """P2-28: a mid-run budget stop must surface BUDGET_CONFIRMATION_REQUIRED +
    requiredAction=approve_budget on the fatal line — never UNKNOWN."""
    path, environment = fixture(tmp_path)
    for key, value in environment.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["transcribe_task.py", "--task-spec", str(path)])

    fake_worker = types.ModuleType("transcribe_podcasts")

    class _BudgetExceeded(Exception):
        code = "BUDGET_CONFIRMATION_REQUIRED"
        required_action = "approve_budget"
        retry_after_seconds = None

    def _boom() -> int:
        raise _BudgetExceeded("Estimated Podcast API budget exceeds approval")

    fake_worker.main = _boom
    fake_worker.LAST_RUN_SUMMARY = None

    fake_pricing = types.ModuleType("deepseek_pricing")
    fake_pricing.PodcastBudgetExceededError = _BudgetExceeded
    fake_pricing.PodcastUpstreamError = type("_Upstream", (Exception,), {})
    fake_pricing.PodcastSecretMissingError = type("_SecretMissing", (Exception,), {})
    fake_pricing.classify_upstream_error = lambda error: None

    fake_pim = types.ModuleType("polish_interview_markdown")
    monkeypatch.setitem(sys.modules, "transcribe_podcasts", fake_worker)
    monkeypatch.setitem(sys.modules, "deepseek_pricing", fake_pricing)
    monkeypatch.setitem(sys.modules, "polish_interview_markdown", fake_pim)

    from transcribe_task import main

    assert main() == 1
    captured = capsys.readouterr()
    stderr_lines = [line for line in captured.err.splitlines() if line.strip()]
    fatal = json.loads(stderr_lines[-1])
    assert fatal["type"] == "fatal"
    assert fatal["errorCode"] == "BUDGET_CONFIRMATION_REQUIRED"
    assert fatal["requiredAction"] == "approve_budget"
    assert "budget" in fatal["message"].lower()


def test_main_fatal_keeps_secret_missing_code_and_action(tmp_path: Path, monkeypatch, capsys) -> None:
    """A missing DeepSeek credential must surface SECRET_MISSING +
    requiredAction=configure_secret so the host routes to the key-input
    flow instead of offering a retry that always fails."""
    path, environment = fixture(tmp_path)
    for key, value in environment.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["transcribe_task.py", "--task-spec", str(path)])

    fake_worker = types.ModuleType("transcribe_podcasts")

    class _SecretMissing(Exception):
        code = "SECRET_MISSING"
        required_action = "configure_secret"
        retry_after_seconds = None

    def _boom() -> int:
        raise _SecretMissing("DeepSeek API key is missing")

    fake_worker.main = _boom
    fake_worker.LAST_RUN_SUMMARY = None

    fake_pricing = types.ModuleType("deepseek_pricing")
    fake_pricing.PodcastBudgetExceededError = type("_BudgetExceeded", (Exception,), {})
    fake_pricing.PodcastUpstreamError = type("_Upstream", (Exception,), {})
    fake_pricing.PodcastSecretMissingError = _SecretMissing
    fake_pricing.classify_upstream_error = lambda error: None

    fake_pim = types.ModuleType("polish_interview_markdown")
    monkeypatch.setitem(sys.modules, "transcribe_podcasts", fake_worker)
    monkeypatch.setitem(sys.modules, "deepseek_pricing", fake_pricing)
    monkeypatch.setitem(sys.modules, "polish_interview_markdown", fake_pim)

    from transcribe_task import main

    assert main() == 1
    captured = capsys.readouterr()
    stderr_lines = [line for line in captured.err.splitlines() if line.strip()]
    fatal = json.loads(stderr_lines[-1])
    assert fatal["type"] == "fatal"
    assert fatal["errorCode"] == "SECRET_MISSING"
    assert fatal["requiredAction"] == "configure_secret"
