"""Task state machine: lifecycle log, guarded state writes, heartbeat, manifest."""
from __future__ import annotations

import json
import logging
import os
import threading
import traceback
from datetime import datetime
from pathlib import Path
from typing import Any

from podcast_transcriber.common import (
    CURRENT_RUN_ID,
    LIFECYCLE_LOG_PATH,
    MANIFEST_PATH,
    iso_now,
    load_json,
    save_json,
)

MANIFEST_LOCK = threading.Lock()


def append_lifecycle_log(event, task_id=None, audio=None, old_status=None, new_status=None, writer=None, worker_pid=None, run_id=None, reason=None):
    try:
        LIFECYCLE_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        entry = {
            "timestamp": datetime.now().isoformat(timespec="seconds"),
            "event": event,
            "task_id": task_id,
            "audio": audio,
            "old_status": old_status,
            "new_status": new_status,
            "writer": writer,
            "worker_pid": worker_pid,
            "run_id": run_id,
            "reason": reason,
        }
        with open(LIFECYCLE_LOG_PATH, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry, ensure_ascii=False, default=str) + "\n")
    except Exception:
        pass


ACTIVE_TASK_STATUSES = {
    "queued",
    "preparing",
    "normalizing",
    "chunking",
    "transcribing",
    "translating",
    "writing_output",
    "postprocess_queued",
}


TERMINAL_TASK_STATUSES = {"success", "partial_success", "completed", "failed", "stalled", "interrupted", "cancelled"}


def save_task_state_safe(state_path: Path, state: dict[str, Any], our_run_id: str = CURRENT_RUN_ID) -> bool:
    """Write task state only if on-disk state is not terminal and owned by another attempt.

    Returns True if write succeeded, False if blocked to preserve terminal state.
    """
    if _state_is_owned_terminal_on_disk(state_path, our_run_id):
        append_lifecycle_log(
            "worker_write_blocked_terminal",
            task_id=state.get("task_id"),
            audio=state.get("source_file"),
            old_status=state.get("status"),
            new_status=state.get("status"),
            writer="worker",
            worker_pid=os.getpid(),
            run_id=our_run_id,
            reason="save_task_state_safe blocked by terminal state on disk",
        )
        return False
    save_json(state_path, state)
    return True


def _state_is_owned_terminal_on_disk(state_path: Path, our_run_id: str) -> bool:
    """Return True if on-disk state is terminal and owned by someone else or explicitly terminal."""
    if not state_path.exists():
        return False
    try:
        disk = json.loads(state_path.read_text(encoding="utf-8-sig"))
    except Exception:
        return False
    disk_status = str(disk.get("status") or "").lower()
    if disk_status not in TERMINAL_TASK_STATUSES:
        return False
    disk_run_id = str(disk.get("run_id") or "")
    disk_terminal_by = str(disk.get("terminal_by") or "")
    if disk_terminal_by:
        return True
    if disk_run_id and disk_run_id != our_run_id:
        return True
    return False


def update_task_state(
    state_path: Path,
    state: dict[str, Any],
    *,
    status: str | None = None,
    stage: str | None = None,
    progress_percent: float | None = None,
    current_chunk: int | None = None,
    total_chunks: int | None = None,
    error_message: str | None = None,
    error_type: str | None = None,
    log_path: str | None = None,
    can_resume: bool | None = None,
    can_retry: bool | None = None,
    heartbeat: bool = True,
    _job_id: str | None = None,
) -> None:
    if _state_is_owned_terminal_on_disk(state_path, CURRENT_RUN_ID):
        append_lifecycle_log(
            "worker_write_blocked_terminal",
            task_id=state.get("task_id"),
            audio=state.get("source_file"),
            old_status=state.get("status"),
            new_status=status,
            writer="worker",
            worker_pid=os.getpid(),
            run_id=CURRENT_RUN_ID,
            reason="on-disk state is terminal and owned by another attempt",
        )
        return
    if status is not None:
        state["status"] = status
    if stage is not None:
        state["stage"] = stage
    if progress_percent is not None:
        state["progress_percent"] = round(max(0.0, min(100.0, float(progress_percent))), 2)
    if current_chunk is not None:
        state["current_chunk"] = int(current_chunk)
    if total_chunks is not None:
        state["total_chunks"] = int(total_chunks)
    if error_message is not None:
        state["error_message"] = error_message
        state["error"] = error_message
    if error_type is not None:
        state["error_type"] = error_type
    if log_path is not None:
        state["log_path"] = log_path
    if can_resume is not None:
        state["can_resume"] = bool(can_resume)
    if can_retry is not None:
        state["can_retry"] = bool(can_retry)
    state["worker_pid"] = os.getpid()
    state["run_id"] = CURRENT_RUN_ID
    stamp = iso_now()
    state["updated_at"] = stamp
    state["last_update_at"] = stamp
    if heartbeat and (status is None or status not in TERMINAL_TASK_STATUSES):
        state["last_heartbeat_at"] = stamp
    save_json(state_path, state)


def touch_task_heartbeat(
    state_path: Path,
    state: dict[str, Any],
    *,
    status: str | None = None,
    stage: str | None = None,
    _job_id: str | None = None,
) -> None:
    if _state_is_owned_terminal_on_disk(state_path, CURRENT_RUN_ID):
        append_lifecycle_log(
            "heartbeat_blocked_terminal",
            task_id=state.get("task_id"),
            audio=state.get("source_file"),
            old_status=state.get("status"),
            new_status=status,
            writer="worker",
            worker_pid=os.getpid(),
            run_id=CURRENT_RUN_ID,
            reason="on-disk state is terminal and owned by another attempt",
        )
        return
    if status is not None:
        state["status"] = status
    if stage is not None:
        state["stage"] = stage
    state["worker_pid"] = os.getpid()
    state["run_id"] = CURRENT_RUN_ID
    stamp = iso_now()
    state["updated_at"] = stamp
    state["last_update_at"] = stamp
    state["last_heartbeat_at"] = stamp
    save_json(state_path, state)


class TaskHeartbeat:
    def __init__(
        self,
        state_path: Path,
        state: dict[str, Any],
        *,
        status: str,
        stage: str,
        interval_seconds: float = 15.0,
        _job_id: str | None = None,
    ) -> None:
        self.state_path = state_path
        self.state = state
        self.status = status
        self.stage = stage
        self.interval_seconds = max(5.0, float(interval_seconds))
        self._job_id = _job_id
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def __enter__(self) -> TaskHeartbeat:
        touch_task_heartbeat(self.state_path, self.state, status=self.status, stage=self.stage, _job_id=self._job_id)
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=1.0)

    def _run(self) -> None:
        while not self._stop.wait(self.interval_seconds):
            try:
                touch_task_heartbeat(self.state_path, self.state, status=self.status, stage=self.stage, _job_id=self._job_id)
            except Exception:
                logging.getLogger(__name__).debug("Could not refresh task heartbeat", exc_info=True)


def mark_task_failed(
    state_path: Path,
    state: dict[str, Any],
    exc: BaseException,
    *,
    stage: str | None = None,
    logger: logging.Logger | None = None,
    _job_id: str | None = None,
) -> None:
    if logger:
        logger.exception("Task failed at stage %s", stage or state.get("stage") or "unknown")
    state["traceback"] = traceback.format_exc()
    update_task_state(
        state_path,
        state,
        status="failed",
        stage=stage or str(state.get("stage") or "failed"),
        error_message=str(exc),
        error_type=type(exc).__name__,
        can_resume=False,
        can_retry=True,
        heartbeat=False,
        _job_id=_job_id,
    )


def load_manifest() -> dict[str, Any]:
    with MANIFEST_LOCK:
        return load_json(MANIFEST_PATH, {"processed": {}})


def update_manifest_entry(task_id: str, updates: dict[str, Any]) -> dict[str, Any]:
    with MANIFEST_LOCK:
        manifest = load_json(MANIFEST_PATH, {"processed": {}})
        entry = dict(manifest.setdefault("processed", {}).get(task_id, {}))
        entry.update(updates)
        manifest["processed"][task_id] = entry
        save_json(MANIFEST_PATH, manifest)
        return entry


def update_manifest_processed(task_id: str, entry: dict[str, Any]) -> None:
    with MANIFEST_LOCK:
        manifest = load_json(MANIFEST_PATH, {"processed": {}})
        manifest.setdefault("processed", {})[task_id] = entry
        save_json(MANIFEST_PATH, manifest)
