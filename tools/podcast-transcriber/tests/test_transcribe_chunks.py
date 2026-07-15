from __future__ import annotations

import logging
import sys
import wave
from pathlib import Path
from types import SimpleNamespace

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


def test_chunk_plan_uses_actual_cumulative_offsets_for_nonuniform_chunks() -> None:
    plan = tp.ChunkPlan.from_durations([2.25, 5.5, 1.75])

    assert [(chunk.source_start, chunk.source_end) for chunk in plan.chunks] == [
        (0.0, 2.25),
        (2.25, 7.75),
        (7.75, 9.5),
    ]
    assert tp.ChunkPlan.from_metadata(plan.as_dict()).signature() == plan.signature()


def test_chunk_plan_rejects_noncontiguous_resume_coordinates() -> None:
    plan = tp.ChunkPlan.from_durations([2.0, 3.0]).as_dict()
    plan["chunks"][1]["source_start"] = 4.0

    try:
        tp.ChunkPlan.from_metadata(plan)
    except ValueError as error:
        assert "contiguous" in str(error)
    else:
        raise AssertionError("noncontiguous chunk coordinates must be rejected")


def test_transcribe_chunk_applies_plan_source_start(tmp_path: Path) -> None:
    chunk = tmp_path / "chunk.wav"
    with wave.open(str(chunk), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(16_000)
        stream.writeframes(b"\x00\x00" * 16_000)

    class Model:
        def transcribe(self, _path: str, **_kwargs: object):
            segment = SimpleNamespace(start=0.25, end=0.75, text="hello")
            return iter([segment]), SimpleNamespace(language="en", language_probability=1.0)

    state: dict[str, object] = {"segments": []}
    result = tp.transcribe_chunk(
        Model(),
        chunk,
        7.75,
        8.75,
        state,
        "00000",
        tmp_path / "state.json",
        {"asr": {"language": "en"}},
        {"batched": "false"},
        10.0,
        logging.getLogger("chunk-offset-test"),
    )

    assert result[0]["start"] == 8.0
    assert result[0]["end"] == 8.5


def test_normalized_audio_cache_requires_source_and_format_metadata(tmp_path: Path, monkeypatch) -> None:
    source = tmp_path / "episode.mp3"
    source.write_bytes(b"source-audio")
    task_dir = tmp_path / "task"
    calls: list[list[str]] = []

    def fake_run(command: list[str], _logger, timeout: int) -> None:
        calls.append(command)
        Path(command[-1]).write_bytes(b"normalized-audio")

    monkeypatch.setattr(tp, "run_command", fake_run)
    logger = logging.getLogger("normalization-cache-test")
    config = {"audio": {"sample_rate": 16_000, "channels": 1}}

    tp.normalize_audio(source, task_dir, config, "ffmpeg", logger)
    tp.normalize_audio(source, task_dir, config, "ffmpeg", logger)
    tp.normalize_audio(source, task_dir, {"audio": {"sample_rate": 16_000, "channels": 2}}, "ffmpeg", logger)

    assert len(calls) == 2


def test_cuda_inference_failure_retries_the_file_with_cpu(tmp_path: Path, monkeypatch) -> None:
    source = tmp_path / "episode.mp3"
    source.write_bytes(b"source-audio")
    monkeypatch.setattr(tp, "STATE_DIR", tmp_path / "state")
    monkeypatch.setattr(tp, "OUT_LOGS", tmp_path / "logs")
    monkeypatch.setattr(tp, "setup_file_logger", lambda _name: logging.getLogger("cuda-fallback-test"))
    calls: list[str] = []

    def fake_process_file(_source, _config, _manifest, _model, runtime, *_args, **_kwargs):
        calls.append(runtime["device"])
        if runtime["device"] == "cuda":
            raise RuntimeError("CUDA execution failed during inference")
        return {"_type": "audio_context", "all_segments": []}

    monkeypatch.setattr(tp, "process_file", fake_process_file)
    monkeypatch.setattr(
        tp,
        "load_whisper_model",
        lambda _config, _logger: (object(), {"device": "cpu"}, []),
    )

    result, failures = tp.process_source_with_fallback(
        source,
        {"asr": {"device_preference": "cuda"}},
        {},
        object(),
        {"device": "cuda"},
        None,
        None,
        False,
        logging.getLogger("cuda-fallback-test"),
        _audio_only=True,
    )

    assert calls == ["cuda", "cpu"]
    assert result["fallback_from"] == "cuda"
    assert any("CUDA inference" in failure for failure in failures)


def test_usable_sidecar_subtitle_does_not_require_whisper(tmp_path: Path) -> None:
    source = tmp_path / "episode.mp4"
    source.write_bytes(b"video")
    subtitle = tmp_path / "episode.srt"
    subtitle.write_text("1\n00:00:01,000 --> 00:00:02,000\nHello\n", encoding="utf-8")

    assert tp.has_usable_sidecar_subtitle(source, {}, None, logging.getLogger("subtitle-preflight")) is True
