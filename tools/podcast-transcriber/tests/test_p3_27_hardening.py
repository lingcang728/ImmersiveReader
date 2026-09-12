from __future__ import annotations

import json
import logging
import os
import sys
import wave
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import transcribe_podcasts as tp  # noqa: E402
from podcast_transcriber.common import strip_host_paths  # noqa: E402
from podcast_transcriber.language import classify_segment_language  # noqa: E402
from transcribe_task import _exit_fatal_payload, _stage_haystack  # noqa: E402


@pytest.fixture(autouse=True)
def _reset_sticky_cpu_state(monkeypatch):
    """Sticky CPU fallback is process-global — never leak it between tests."""
    monkeypatch.setattr(tp, "_FORCE_CPU_DEVICE", False)
    monkeypatch.setattr(tp, "_CPU_FALLBACK_BUNDLE", None)


# ---- host path scrubbing -------------------------------------------------


def test_strip_host_paths_redacts_drive_and_unc_paths() -> None:
    assert strip_host_paths(r"failed: C:\Users\me\data\episode.mp3 boom") == "failed: episode.mp3 boom"
    assert strip_host_paths("failed: C:/Users/me/data/episode.mp3 boom") == "failed: episode.mp3 boom"
    assert strip_host_paths(r"open \\NAS\share\dir\file.wav") == "open file.wav"
    # Relative paths and URLs must survive untouched.
    assert strip_host_paths("work/chunks/chunk_00000.wav") == "work/chunks/chunk_00000.wav"
    assert strip_host_paths("https://api.deepseek.com/v1") == "https://api.deepseek.com/v1"
    assert strip_host_paths(None) == "None"


def test_fatal_payload_scrubs_paths_and_carries_stage() -> None:
    summary = {
        "results": [
            {"file": "ep.mp3", "status": "failed", "error": r"cannot read C:\data\in\ep.mp3"}
        ],
        "failures": [],
    }
    payload = _exit_fatal_payload(1, summary, stage="transcribing")
    assert payload["type"] == "fatal"
    assert payload["stage"] == "transcribing"
    assert "C:\\" not in payload["message"]
    assert "ep.mp3" in payload["message"]


def test_stage_haystack_strips_screaming_error_codes() -> None:
    # MODEL_INCOMPATIBLE must not leave a "model" needle behind.
    assert "model" not in _stage_haystack("MODEL_INCOMPATIBLE: hash changed")
    assert "model" in _stage_haystack("Trying model=large-v3 device=cuda")
    assert _stage_haystack("Normalizing audio") == "normalizing audio"


# ---- language classification (Han-only) -----------------------------------


def test_kana_and_hangul_are_not_chinese() -> None:
    # Pure kana / Hangul strings carry no Han characters and must not
    # classify as zh (they used to count toward the CJK bucket).
    assert classify_segment_language("これはひらがなだけのテストです") != "zh"
    assert classify_segment_language("이것은한국어테스트문장입니다") != "zh"
    # Han ideographs still classify as zh.
    assert classify_segment_language("这是一个中文句子用于测试") == "zh"


# ---- managed single-input discovery ---------------------------------------


def test_discover_audio_files_uses_pinned_input(tmp_path: Path, monkeypatch) -> None:
    managed = tmp_path / "only-this.mp3"
    managed.write_bytes(b"audio")
    inbox = tmp_path / "inbox"
    inbox.mkdir()
    (inbox / "other.mp3").write_bytes(b"other")
    monkeypatch.setattr(tp, "INBOX", inbox)
    monkeypatch.setattr(tp, "CONFIG_PATH", tmp_path / "missing-config.json")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_INPUT_FILE", str(managed))

    assert tp.discover_audio_files() == [managed]


def test_discover_audio_files_rejects_missing_pinned_input(tmp_path: Path, monkeypatch) -> None:
    inbox = tmp_path / "inbox"
    inbox.mkdir()
    monkeypatch.setattr(tp, "INBOX", inbox)
    monkeypatch.setattr(tp, "CONFIG_PATH", tmp_path / "missing-config.json")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_INPUT_FILE", str(tmp_path / "gone.mp3"))

    with pytest.raises(RuntimeError, match="Managed input file is missing"):
        tp.discover_audio_files()


def test_discover_audio_files_falls_back_to_inbox(tmp_path: Path, monkeypatch) -> None:
    inbox = tmp_path / "inbox"
    inbox.mkdir()
    audio = inbox / "a.mp3"
    audio.write_bytes(b"a")
    (inbox / "note.txt").write_text("x")
    monkeypatch.setattr(tp, "INBOX", inbox)
    monkeypatch.setattr(tp, "CONFIG_PATH", tmp_path / "missing-config.json")
    monkeypatch.delenv("PODCAST_TRANSCRIBER_INPUT_FILE", raising=False)

    assert tp.discover_audio_files() == [audio]


# ---- chunk overlap plan ----------------------------------------------------


def test_chunk_plan_overlap_keeps_logical_contiguity() -> None:
    plan = tp.ChunkPlan.from_boundaries([0.0, 100.0, 250.0], overlap_seconds=15.0, duration=250.0)

    assert [(c.source_start, c.source_end) for c in plan.chunks] == [(0.0, 100.0), (100.0, 250.0)]
    assert [(c.resolved_audio_start(), c.resolved_audio_end()) for c in plan.chunks] == [
        (0.0, 115.0),   # first chunk: no leading pad
        (85.0, 250.0),  # last chunk: no trailing pad
    ]
    # Round-trip through metadata preserves both coordinate sets + signature.
    restored = tp.ChunkPlan.from_metadata(plan.as_dict())
    assert restored.signature() == plan.signature()
    assert restored.chunks[1].resolved_audio_start() == 85.0


def test_chunk_plan_rejects_inverted_audio_span() -> None:
    raw = tp.ChunkPlan.from_boundaries([0.0, 10.0], overlap_seconds=5.0, duration=10.0).as_dict()
    raw["chunks"][0]["audio_end"] = 1.0  # inside the logical range — invalid

    with pytest.raises(ValueError, match="audio coordinates"):
        tp.ChunkPlan.from_metadata(raw)


def test_transcribe_chunk_drops_out_of_range_segments(tmp_path: Path) -> None:
    """Overlap audio produces segments outside the logical range; only the
    chunk owning a segment's start keeps it (no boundary duplicates)."""
    chunk = tmp_path / "chunk.wav"
    with wave.open(str(chunk), "wb") as stream:
        stream.setnchannels(1)
        stream.setsampwidth(2)
        stream.setframerate(16_000)
        stream.writeframes(b"\x00\x00" * 16_000)

    class Model:
        def transcribe(self, _path: str, **_kwargs: object):
            # Local times for a wav physically starting at 85.0: a sentence
            # spanning the 100.0 boundary appears whole in this chunk (kept by
            # the previous owner), plus this chunk's own segment.
            segments = [
                SimpleNamespace(start=0.0, end=16.5, text="spanning sentence tail"),
                SimpleNamespace(start=20.0, end=24.0, text="owned segment"),
            ]
            return iter(segments), SimpleNamespace(language="en", language_probability=0.9)

    state: dict[str, object] = {"segments": []}
    result = tp.transcribe_chunk(
        Model(),
        chunk,
        100.0,  # logical range [100, 200)
        200.0,
        state,
        "00001",
        tmp_path / "state.json",
        {"asr": {"language": "en"}},
        {"batched": "false"},
        250.0,
        logging.getLogger("overlap-ownership-test"),
        audio_start=85.0,
        audio_end=215.0,
    )

    # The segment starting at 85.0 (before the logical start) is dropped;
    # the one starting at 105.0 is kept with global coordinates.
    assert [(s["start"], s["end"]) for s in result] == [(105.0, 109.0)]
    # Per-chunk language is recorded on the chunk entry, not the file state.
    assert state["chunks"]["00001"]["language"] == "en"


def test_dominant_chunk_language_uses_mode_not_last_chunk() -> None:
    state = {
        "detected_language": "ja",
        "chunks": {
            "00000": {"language": "zh", "status": "done"},
            "00001": {"language": "zh", "status": "done"},
            "00002": {"language": "en", "status": "done"},
        },
    }
    assert tp._dominant_chunk_language(state) == "zh"


# ---- sticky CPU fallback ---------------------------------------------------


def test_sticky_cpu_fallback_skips_cuda_for_later_files(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr(tp, "STATE_DIR", tmp_path / "state")
    monkeypatch.setattr(tp, "OUT_LOGS", tmp_path / "logs")
    monkeypatch.setattr(tp, "setup_file_logger", lambda _name: logging.getLogger("sticky-cpu-test"))
    source = tmp_path / "a.mp3"
    source.write_bytes(b"audio")
    devices: list[str] = []

    def fake_process_file(_source, _config, _manifest, _model, runtime, *_args, **_kwargs):
        devices.append(runtime["device"])
        if runtime["device"] == "cuda":
            raise RuntimeError("CUDA execution failed during inference")
        return {"_type": "audio_context", "all_segments": [], "runtime": dict(runtime)}

    monkeypatch.setattr(tp, "process_file", fake_process_file)
    monkeypatch.setattr(
        tp,
        "load_whisper_model",
        lambda _config, _logger: (object(), {"device": "cpu"}, []),
    )

    config = {"asr": {"device_preference": "cuda"}}
    result, _ = tp.process_source_with_fallback(
        source, config, {}, object(), {"device": "cuda"}, None, None, False,
        logging.getLogger("sticky-cpu-test"), _audio_only=True,
    )
    assert result["fallback_from"] == "cuda"
    assert devices == ["cuda", "cpu"]

    # A second file must go straight to the shared CPU model — no CUDA retry.
    devices.clear()
    source2 = tmp_path / "b.mp3"
    source2.write_bytes(b"audio2")
    result2, _ = tp.process_source_with_fallback(
        source2, config, {}, object(), {"device": "cuda"}, None, None, False,
        logging.getLogger("sticky-cpu-test"), _audio_only=True,
    )
    assert devices == ["cpu"]
    assert result2["runtime"]["fallback_from"] == "cuda"


def test_load_whisper_model_stays_cpu_after_sticky_flag(monkeypatch) -> None:
    tp._FORCE_CPU_DEVICE = True
    seen: list[str] = []

    class FakeWhisper:
        def __init__(self, _name, **kwargs):
            seen.append(kwargs["device"])

    monkeypatch.setitem(sys.modules, "faster_whisper", SimpleNamespace(WhisperModel=FakeWhisper, BatchedInferencePipeline=None))
    monkeypatch.setattr(tp, "configure_nvidia_dll_paths", lambda *_a, **_k: None)

    _model, runtime, _failures = tp.load_whisper_model(
        {"asr": {"device_preference": "auto"}}, logging.getLogger("sticky-load-test")
    )
    assert runtime["device"] == "cpu"
    assert seen == ["cpu"]


# ---- per-task work artifact cleanup ----------------------------------------


def test_cleanup_task_work_artifacts_removes_only_managed_dirs(tmp_path: Path, monkeypatch) -> None:
    work = tmp_path / "work"
    chunks = work / "chunks" / "task-1"
    norm = work / "episode"
    chunks.mkdir(parents=True)
    norm.mkdir(parents=True)
    (chunks / "chunk_00000.wav").write_bytes(b"x")
    (norm / "source.wav").write_bytes(b"x")
    monkeypatch.setattr(tp, "WORK", work)
    monkeypatch.setattr(tp, "CHUNKS_DIR", work / "chunks")
    monkeypatch.delenv("PODCAST_TRANSCRIBER_KEEP_WORK", raising=False)

    tp.cleanup_task_work_artifacts("task-1", "episode", logging.getLogger("cleanup-test"))

    assert not chunks.exists()
    assert not norm.exists()


def test_cleanup_task_work_artifacts_keep_flag_preserves(tmp_path: Path, monkeypatch) -> None:
    work = tmp_path / "work"
    norm = work / "episode"
    norm.mkdir(parents=True)
    (norm / "source.wav").write_bytes(b"x")
    monkeypatch.setattr(tp, "WORK", work)
    monkeypatch.setattr(tp, "CHUNKS_DIR", work / "chunks")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_KEEP_WORK", "1")

    tp.cleanup_task_work_artifacts("task-1", "episode", logging.getLogger("cleanup-test"))

    assert norm.exists()


def test_cleanup_task_work_artifacts_never_leaves_work_root(tmp_path: Path, monkeypatch) -> None:
    work = tmp_path / "work"
    work.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    monkeypatch.setattr(tp, "WORK", work)
    monkeypatch.setattr(tp, "CHUNKS_DIR", work / "chunks")

    tp.cleanup_task_work_artifacts("task-1", ".." + os.sep + "outside", logging.getLogger("cleanup-test"))

    assert outside.exists()


# ---- state-write resilience ------------------------------------------------


def test_update_task_state_survives_unwritable_path(tmp_path: Path, monkeypatch, caplog) -> None:
    from podcast_transcriber import state as state_mod

    def _raise_oserror(*_args, **_kwargs):
        raise OSError("disk full")

    monkeypatch.setattr(state_mod, "save_json", _raise_oserror)

    with caplog.at_level(logging.WARNING):
        state_mod.update_task_state(tmp_path / "state.json", {"task_id": "t"}, status="preparing")

    assert any("state write failed" in record.getMessage().lower() for record in caplog.records)


def test_load_json_distinguishes_failure_modes(tmp_path: Path, caplog) -> None:
    from podcast_transcriber import common

    missing = tmp_path / "missing.json"
    corrupt = tmp_path / "corrupt.json"
    corrupt.write_text("{not json", encoding="utf-8")

    with caplog.at_level(logging.WARNING):
        assert common.load_json(missing, {"d": 1}) == {"d": 1}
        assert common.load_json(corrupt, {"d": 2}) == {"d": 2}

    assert not any("missing" in record.getMessage().lower() for record in caplog.records)
    assert any("decode error" in record.getMessage().lower() for record in caplog.records)


def test_error_message_and_traceback_are_scrubbed_in_state(tmp_path: Path) -> None:
    from podcast_transcriber import state as state_mod

    state_path = tmp_path / "state.json"
    state_mod.update_task_state(
        state_path,
        {"task_id": "t"},
        status="failed",
        error_message=r"cannot open C:\Users\me\secret\file.mp3",
    )
    saved = json.loads(state_path.read_text(encoding="utf-8"))
    assert "C:\\" not in saved["error_message"]
    assert "file.mp3" in saved["error_message"]
