from __future__ import annotations

import importlib
import json
import logging
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig")


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def main() -> int:
    errors: list[str] = []
    readme = read(ROOT / "README.md") if (ROOT / "README.md").exists() else ""
    config_path = ROOT / "config.json"
    if not config_path.exists():
        config_path = ROOT / "config.example.json"
    config = json.loads(read(config_path)) if config_path.exists() else {}

    if not (ROOT / "requirements.txt").exists():
        fail(errors, "requirements.txt is missing")
    if "output/final" in readme.replace("\\", "/"):
        fail(errors, "README.md still mentions output/final")
    if config.get("output_dir") != "output":
        fail(errors, f"{config_path.name} output_dir must be output")
    if config.get("input_dir") != "input" or config.get("work_dir") != "work":
        fail(errors, f"{config_path.name} must define input_dir=input and work_dir=work")

    supported = {str(item).lower() for item in (config.get("audio") or {}).get("supported_extensions", [])}
    if supported != {".mp3", ".m4a", ".wav"}:
        fail(errors, "config audio.supported_extensions must be .mp3/.m4a/.wav only")

    video_supported = {str(item).lower() for item in (config.get("video") or {}).get("supported_extensions", [])}
    if ".mp4" not in video_supported:
        fail(errors, "config video.supported_extensions must include .mp4")

    scripts_path = ROOT / "scripts"
    if str(scripts_path) not in sys.path:
        sys.path.insert(0, str(scripts_path))
    try:
        worker = importlib.import_module("transcribe_podcasts")
        chunk_plan = importlib.import_module("podcast_transcriber.chunk_plan")
        common = importlib.import_module("podcast_transcriber.common")
        ChunkPlan = chunk_plan.ChunkPlan
        sidecar_extensions = common.SIDECAR_SUBTITLE_EXTENSIONS
        supported_extensions = common.SUPPORTED_EXTENSIONS

        if supported_extensions != {".mp3", ".m4a", ".wav"}:
            fail(errors, "runtime SUPPORTED_EXTENSIONS behavior is incorrect")
        if ".srt" not in sidecar_extensions:
            fail(errors, "runtime sidecar subtitle behavior is missing .srt")
        with tempfile.TemporaryDirectory(prefix="immersive-quick-validate-") as temporary:
            fixture_root = Path(temporary)
            source = fixture_root / "episode.mp3"
            subtitle = fixture_root / "episode.srt"
            source.write_bytes(b"synthetic audio placeholder")
            subtitle.write_text(
                "1\n00:00:00,000 --> 00:00:01,250\nhello\n\n2\n00:00:01,250 --> 00:00:02,000\nworld\n",
                encoding="utf-8",
            )
            logger = logging.getLogger("quick_validate")
            segments = worker.parse_subtitle_to_segments(subtitle, None, logger)
            if len(segments) != 2 or segments[1]["start"] != 1.25:
                fail(errors, "subtitle parsing behavior is incorrect")
            if not worker.has_usable_sidecar_subtitle(source, {}, None, logger):
                fail(errors, "sidecar subtitle preflight behavior is incorrect")
            plan = ChunkPlan.from_durations([1.25, 0.75])
            restored = ChunkPlan.from_metadata(plan.as_dict())
            if restored.signature() != plan.signature() or restored.chunks[-1].source_end != 2.0:
                fail(errors, "chunk plan round-trip behavior is incorrect")
    except Exception as exc:
        fail(errors, f"behavior tests could not run: {exc}")
    launcher = config.get("launcher") or {}
    if launcher.get("open_folder_after_run") is not True or launcher.get("open_folder") != "output":
        fail(errors, "config launcher must open output after runs by default")
    if config.get("skip_processed_files") is not False or config.get("always_reprocess_inputs") is not True:
        fail(errors, "config must disable processed-file memory and always reprocess input audio")
    translation = config.get("translation") or {}
    if translation.get("backend") == "ollama" and translation.get("auto_start_ollama") is not True:
        fail(errors, "Ollama translation should auto-start Ollama by default")

    if (ROOT / "output" / "final").exists():
        fail(errors, "output/final directory must not exist")

    if errors:
        print("quick_validate failed:")
        for item in errors:
            print(f"- {item}")
        return 1
    print("quick_validate OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
