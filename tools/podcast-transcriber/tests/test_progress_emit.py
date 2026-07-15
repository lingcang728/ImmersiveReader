from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from podcast_transcriber.progress_emit import StageProgressEmitter  # noqa: E402


def test_stage_progress_emits_on_percent_advance(capsys) -> None:
    emitter = StageProgressEmitter()
    emitter.emit(stage="transcribing", completed=1, total=10, unit="块", force=True)
    emitter.emit(stage="transcribing", completed=1, total=10, unit="块")  # same whole % -> drop
    emitter.emit(stage="transcribing", completed=5, total=10, unit="块", force=True)
    out = capsys.readouterr().out.strip().splitlines()
    assert len(out) == 2
    assert '"percent": 10' in out[0] or '"percent": 10.0' in out[0]
    assert '"completedUnits": 5' in out[1]


def test_unmeasurable_stage_omits_percent(capsys) -> None:
    emitter = StageProgressEmitter()
    emitter.emit(stage="prepare", message="start", force=True)
    line = capsys.readouterr().out.strip()
    assert "prepare" in line
    assert "percent" not in line
