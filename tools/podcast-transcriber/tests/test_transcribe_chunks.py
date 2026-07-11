from __future__ import annotations

import logging
import sys
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import transcribe_podcasts as tp  # noqa: E402


def test_audio_duration_uses_current_pyav_without_time_base(tmp_path: Path) -> None:
    audio = tmp_path / "duration.wav"
    with wave.open(str(audio), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(16_000)
        stream.writeframes(b"\x00\x00" * 16_000)

    assert tp.probe_duration(audio, None, logging.getLogger("duration-test")) == 1.0


def test_split_points_snap_to_nearby_silence() -> None:
    silences = [(1750.2, 1751.0), (3500.0, 3500.6)]

    points = tp.compute_silence_split_points(7200.0, 1800, silences)

    assert points[0] == 1750.6
    assert points[1] == 3500.3


def test_split_points_fall_back_to_hard_cut_outside_window() -> None:
    silences = [(100.0, 101.0)]

    points = tp.compute_silence_split_points(5400.0, 1800, silences)

    assert points == [1800.0, 3600.0]


def test_no_split_for_short_audio_or_no_silence() -> None:
    assert tp.compute_silence_split_points(1900.0, 1800, [(900.0, 901.0)]) == []
    assert tp.compute_silence_split_points(7200.0, 1800, []) == []


def test_split_spacing_respects_chunk_validation_tolerance() -> None:
    silences = [(float(t) - 80.0, float(t) - 79.0) for t in range(1800, 20000, 1800)]

    points = tp.compute_silence_split_points(20000.0, 1800, silences)

    previous = 0.0
    for point in points:
        assert point - previous >= 1800 * 0.5, (point, previous)
        previous = point
