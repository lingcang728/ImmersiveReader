from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from transcribe_task import TaskSpecError, load_task_spec  # noqa: E402


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
