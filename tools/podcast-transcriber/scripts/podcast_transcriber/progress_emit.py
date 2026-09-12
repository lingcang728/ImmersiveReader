"""Throttled stage progress emitter for managed podcast workers.

Reports real completed/total work units when available. Emits at most 4 events
per second, and only when the stage changes or the whole-percentage advances.
Unmeasurable stages omit percent (indeterminate on the desktop).
"""

from __future__ import annotations

import json
import math
import sys
import time
from typing import Any

_MIN_INTERVAL_S = 0.25  # max 4 events/sec

# Streams already verified/flipped to UTF-8. Checked lazily at emit time so
# that merely importing this module never mutates the host's stdio.
_utf8_checked: set[Any] = set()


def _ensure_utf8(stream: Any) -> None:
    """Best-effort, once per stream: switch to UTF-8 unless already there.

    This is a shared library (transcribe_task worker, transcribe_podcasts,
    sidecar importers, tests), so it must not reconfigure at import time —
    the importer may own these streams. But the desktop host reads worker
    pipes as UTF-8 and a zh-CN cp936 stream would break the reader thread,
    so on first emit we flip only streams whose encoding is not already
    UTF-8 (errors="replace" keeps odd bytes from killing the writer).
    Idempotent and failure-tolerant: emitting must never crash here.
    """
    try:
        if stream in _utf8_checked:
            return
        _utf8_checked.add(stream)
        encoding = str(getattr(stream, "encoding", "") or "")
        normalized = encoding.lower().replace("-", "").replace("_", "")
        if normalized in ("utf8", "cp65001"):
            return
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is not None:
            reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


class StageProgressEmitter:
    def __init__(self) -> None:
        self._last_stage: str | None = None
        self._last_whole: int = -1
        self._last_emit_at: float = 0.0

    def emit(
        self,
        *,
        stage: str,
        completed: int | None = None,
        total: int | None = None,
        unit: str | None = None,
        message: str | None = None,
        event_type: str = "progress",
        force: bool = False,
    ) -> None:
        stage = str(stage or "working")
        percent: float | None = None
        if (
            completed is not None
            and total is not None
            and total > 0
            and math.isfinite(float(completed))
            and math.isfinite(float(total))
        ):
            percent = max(0.0, min(100.0, (float(completed) / float(total)) * 100.0))

        whole = int(percent) if percent is not None else -1
        stage_changed = stage != self._last_stage
        percent_advanced = percent is not None and whole > self._last_whole
        now = time.monotonic()
        rate_ok = (now - self._last_emit_at) >= _MIN_INTERVAL_S

        if not force and not stage_changed and not (percent_advanced and rate_ok):
            # Allow first sample of a measurable stage even without advance.
            if not (percent is not None and self._last_whole < 0 and rate_ok):
                return

        payload: dict[str, Any] = {
            "type": event_type,
            "stage": stage,
        }
        if percent is not None:
            payload["percent"] = round(percent, 2)
        if completed is not None:
            payload["completedUnits"] = int(max(0, completed))
        if total is not None:
            payload["totalUnits"] = int(max(0, total))
        if unit:
            payload["unit"] = unit
        if message:
            payload["message"] = message[:180]

        _ensure_utf8(sys.stdout)
        print(json.dumps(payload, ensure_ascii=False), flush=True, file=sys.stdout)
        self._last_stage = stage
        if percent is not None:
            self._last_whole = whole
        self._last_emit_at = now

    def heartbeat(self, stage: str, message: str | None = None) -> None:
        payload: dict[str, Any] = {"type": "heartbeat", "stage": stage}
        if message:
            payload["message"] = message[:180]
        _ensure_utf8(sys.stdout)
        print(json.dumps(payload, ensure_ascii=False), flush=True, file=sys.stdout)


_GLOBAL: StageProgressEmitter | None = None


def get_emitter() -> StageProgressEmitter:
    global _GLOBAL
    if _GLOBAL is None:
        _GLOBAL = StageProgressEmitter()
    return _GLOBAL


def report_stage_progress(
    stage: str,
    *,
    completed: int | None = None,
    total: int | None = None,
    unit: str | None = None,
    message: str | None = None,
    force: bool = False,
) -> None:
    get_emitter().emit(
        stage=stage,
        completed=completed,
        total=total,
        unit=unit,
        message=message,
        force=force,
    )
