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

        print(json.dumps(payload, ensure_ascii=False), flush=True, file=sys.stdout)
        self._last_stage = stage
        if percent is not None:
            self._last_whole = whole
        self._last_emit_at = now

    def heartbeat(self, stage: str, message: str | None = None) -> None:
        payload: dict[str, Any] = {"type": "heartbeat", "stage": stage}
        if message:
            payload["message"] = message[:180]
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
