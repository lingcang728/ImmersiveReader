from __future__ import annotations

import json
import re
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

    scripts_text = "\n".join(
        read(path).replace("\\", "/")
        for path in (ROOT / "scripts").glob("*")
        if path.is_file() and path.suffix in (".py", ".ps1") and path.name != "quick_validate.py"
    )
    if re.search(r"output/final", scripts_text):
        fail(errors, "scripts still reference output/final")
    if "OUT_FINAL_MARKDOWN = OUTPUT / \"final\"" in scripts_text:
        fail(errors, "transcribe script still writes final Markdown under output/final")
    has_ext_check = all(ext in scripts_text for ext in ['".mp3"', '".m4a"', '".wav"', 'SUPPORTED_EXTENSIONS'])
    if not has_ext_check:
        fail(errors, "transcribe script must limit input extensions to mp3/m4a/wav")
    if "--dry-run" not in scripts_text or "Does not load Whisper model" not in scripts_text:
        fail(errors, "dry-run help must make clear it does not load the model")
    if "Internal artifacts" not in scripts_text or "Final markdown output" not in scripts_text:
        fail(errors, "dry-run should report output/work directory semantics")
    launcher = config.get("launcher") or {}
    if launcher.get("open_folder_after_run") is not True or launcher.get("open_folder") != "output":
        fail(errors, "config launcher must open output after runs by default")
    run_script = read(ROOT / "scripts" / "run_podcast_transcriber.ps1")
    if "ShortcutMode" not in run_script or 'Start-Process -FilePath "explorer.exe"' not in run_script:
        fail(errors, "PowerShell launcher must support shortcut mode and opening the target folder")
    if "--force" not in run_script:
        fail(errors, "PowerShell launcher must force re-transcription for shortcut runs")
    if config.get("skip_processed_files") is not False or config.get("always_reprocess_inputs") is not True:
        fail(errors, "config must disable processed-file memory and always reprocess input audio")
    if "--force" not in scripts_text or "force_reprocess" not in scripts_text:
        fail(errors, "transcriber must support forced reprocessing")
    translation = config.get("translation") or {}
    if translation.get("backend") == "ollama" and translation.get("auto_start_ollama") is not True:
        fail(errors, "Ollama translation should auto-start Ollama by default")
    if "maybe_start_ollama" not in scripts_text or '"serve"' not in scripts_text:
        fail(errors, "transcriber must try to start Ollama during preflight")

    if not (ROOT / "input").exists() or not (ROOT / "output").exists():
        fail(errors, "input/ and output/ directories must exist")
    if not (ROOT / "input" / ".gitkeep").exists():
        fail(errors, "input/.gitkeep is missing")
    if not (ROOT / "output" / ".gitkeep").exists():
        fail(errors, "output/.gitkeep is missing")
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
