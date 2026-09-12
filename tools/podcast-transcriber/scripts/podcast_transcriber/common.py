"""Project paths, directory bootstrap, and atomic JSON state IO."""
from __future__ import annotations

import io
import json
import logging
import os
import re
import sys
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any

APP_ROOT = Path(__file__).resolve().parents[2]
ROOT = Path(os.environ.get("IMMERSIVE_PODCAST_DATA_ROOT", APP_ROOT)).resolve()
CACHE_ROOT = Path(os.environ.get("IMMERSIVE_PODCAST_CACHE_ROOT", ROOT)).resolve()


INBOX = CACHE_ROOT / "input"


OUTPUT = CACHE_ROOT / "output"


OUT_FINAL = OUTPUT


OUT_FINAL_MARKDOWN = OUTPUT


WORK = CACHE_ROOT / "work"


INTERNAL = WORK / "internal"


OUT_MARKDOWN = INTERNAL / "markdown_raw"


OUT_MARKDOWN_BILINGUAL = INTERNAL / "markdown_bilingual"


OUT_SRT = INTERNAL / "srt"


OUT_JSON = INTERNAL / "json"


OUT_LOGS = WORK / "logs"


OUT_REPORTS = WORK / "reports"


STATE_DIR = WORK / "state"


CHUNKS_DIR = WORK / "chunks"


NORMALIZED_DIR = WORK / "audio_normalized"


MODELS_DIR = Path(os.environ.get("IMMERSIVE_PODCAST_MODEL_ROOT", ROOT / "models")).resolve()


def _ct2_model_dir(path: Path) -> bool:
    """A directory faster-whisper can load directly must hold a top-level model.bin."""
    return path.is_dir() and (path / "model.bin").is_file()


def _canonical_model_dir_name(name: str) -> str:
    """Strip the vendored ``faster-whisper-`` prefix / ``-local`` suffix."""
    stem = name
    if stem.startswith("faster-whisper-"):
        stem = stem[len("faster-whisper-"):]
    if stem.endswith("-local"):
        stem = stem[: -len("-local")]
    return stem


def resolve_model_reference(value: Any) -> str:
    """Resolve a configured ASR model name against the managed model root.

    Order (P1-27): exact ``MODELS_DIR/<basename>`` wins; then a loadable
    vendored ``<name>-local`` dir or one whose canonical name matches
    (``large-v3-turbo`` → ``faster-whisper-large-v3-turbo-local``); then a
    vendored HF cache ``models--<org>--<name>`` resolved to the snapshot dir
    that actually holds ``model.bin`` (the cache root itself is NOT loadable);
    finally any loadable vendored dir embedding the name. Unresolvable names
    pass through so faster-whisper can still handle well-known HF model ids.
    """
    reference = str(value or "").strip()
    if not reference or not os.environ.get("IMMERSIVE_PODCAST_MODEL_ROOT"):
        return reference
    name = Path(reference).name
    if not name:
        return reference
    candidate = MODELS_DIR / name
    if candidate.is_dir():
        return str(candidate)
    try:
        children = sorted(child for child in MODELS_DIR.iterdir() if child.is_dir())
    except OSError:
        return reference
    vendored = [
        child
        for child in children
        if not child.name.startswith("models--") and _ct2_model_dir(child)
    ]
    wanted = _canonical_model_dir_name(name)
    for child in vendored:
        if child.name == f"{name}-local" or _canonical_model_dir_name(child.name) == wanted:
            return str(child)
    for child in children:
        # HF cache layout is "models--<org>--<repo>" (e.g.
        # models--Systran--faster-whisper-large-v3); compare the repo
        # component canonically so "large-v3" reaches the real large-v3
        # snapshot instead of fuzzy-matching an unrelated vendored dir.
        if not child.name.startswith("models--"):
            continue
        repo = child.name.split("--")[-1]
        if repo == name or _canonical_model_dir_name(repo) == wanted:
            snapshots = sorted(
                (snap for snap in (child / "snapshots").glob("*") if _ct2_model_dir(snap)),
                key=lambda snap: snap.name,
                reverse=True,
            )
            if snapshots:
                return str(snapshots[0])
    for child in vendored:
        if name in child.name:
            return str(child)
    return reference


CONFIG_PATH = ROOT / "config.json"


MANIFEST_PATH = STATE_DIR / "manifest.json"


RUN_LOCK_PATH = CACHE_ROOT / ".podcast_transcriber.lock"


LIFECYCLE_LOG_PATH = WORK / "logs" / "task_lifecycle.jsonl"


CURRENT_RUN_ID = os.environ.get("PODCAST_TRANSCRIBER_RUN_ID", "")


SUPPORTED_EXTENSIONS = {".mp3", ".m4a", ".wav"}


# Video containers we accept as input. The audio track is extracted with ffmpeg
# (normalize_audio/-vn) and then runs through the normal audio pipeline. Kept
# separate from SUPPORTED_EXTENSIONS so audio-only validation stays unchanged.
VIDEO_EXTENSIONS = {".mp4", ".mkv", ".mov", ".webm", ".avi", ".m4v", ".ts", ".flv", ".wmv"}


# Sidecar subtitle files (same stem next to the input) accepted as a transcript
# source in place of ASR. ffmpeg converts non-srt formats to srt before parsing.
SIDECAR_SUBTITLE_EXTENSIONS = {".srt", ".ass", ".ssa", ".vtt"}


INVALID_FILENAME_CHARS = r'<>:"/\|?*'


DEFAULT_AUTO_TRANSLATE_LANGUAGES = {"en"}


STATE_IO_LOCK = threading.RLock()


def now_stamp() -> str:
    return datetime.now().strftime("%Y-%m-%d %H:%M:%S")


def ensure_dirs() -> None:
    for path in [
        INBOX,
        OUT_MARKDOWN,
        OUT_MARKDOWN_BILINGUAL,
        OUT_FINAL,
        OUT_FINAL_MARKDOWN,
        OUT_SRT,
        OUT_JSON,
        OUT_LOGS,
        OUT_REPORTS,
        NORMALIZED_DIR,
        CHUNKS_DIR,
        STATE_DIR,
        MODELS_DIR,
    ]:
        path.mkdir(parents=True, exist_ok=True)


def load_json(path: Path, default: Any) -> Any:
    if not path.exists():
        return default
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except json.JSONDecodeError as exc:
        # Corrupt payload — the caller falls back to ``default``, but the
        # fault must stay visible (a silently-degraded manifest/state file
        # previously looked identical to a missing one).
        logging.getLogger(__name__).warning(
            "Ignoring unreadable JSON %s (decode error): %s", path.name, exc
        )
        return default
    except OSError as exc:
        logging.getLogger(__name__).warning("Could not read JSON %s: %s", path.name, exc)
        return default
    except Exception as exc:
        logging.getLogger(__name__).warning(
            "Ignoring unreadable JSON %s (%s): %s", path.name, type(exc).__name__, exc
        )
        return default


def save_json(path: Path, data: Any) -> None:
    with STATE_IO_LOCK:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = json.dumps(data, ensure_ascii=False, indent=2)
        last_error: Exception | None = None
        for attempt in range(8):
            tmp = path.with_name(f"{path.name}.{os.getpid()}.{time.time_ns()}.tmp")
            try:
                tmp.write_text(payload, encoding="utf-8")
                os.replace(tmp, path)
                return
            except FileNotFoundError:
                # 目标路径或父目录可能被外部清理（如 reset_state）删除；重建后重试
                path.parent.mkdir(parents=True, exist_ok=True)
                try:
                    if tmp.exists():
                        tmp.rename(path)
                        return
                except OSError:
                    pass
                last_error = OSError(f"Target path {path} missing and rename failed")
            except OSError as exc:
                last_error = exc
                try:
                    if tmp.exists():
                        tmp.unlink()
                except OSError:
                    pass
                time.sleep(0.2 * (attempt + 1))
        if last_error:
            raise last_error


def iso_now() -> str:
    return datetime.now().isoformat(timespec="seconds")


# Absolute host paths (drive-letter or UNC) must never reach the desktop
# host: fatal NDJSON lines, task state and log records expose basenames only
# (P3-27). URLs are untouched — a scheme like ``https://`` ends in a letter
# before the ``:`` so the lookbehind rejects it; drive letters only count at
# a non-alphanumeric boundary (start, space, quote, paren…).
_HOST_ABS_PATH_RE = re.compile(
    r"(?<![A-Za-z0-9:])[A-Za-z]:[\\/][^\s\"'|><*?]*|\\\\[^\s\"'|><*?]+"
)


def strip_host_paths(text: Any) -> str:
    """Replace absolute host paths in ``text`` with their final component.

    Keeps the readable tail (usually the file or directory name) so error
    payloads and log lines stay meaningful without leaking the host layout.
    """
    def _basename(match: re.Match[str]) -> str:
        raw = match.group(0).rstrip("\\/.,:;)\"'")
        parts = [part for part in re.split(r"[\\/]+", raw) if part]
        tail = parts[-1].rstrip(".,:;)\"'") if parts else raw
        return tail or raw

    return _HOST_ABS_PATH_RE.sub(_basename, str(text))


def _ensure_utf8_stdio() -> None:
    """Force ``sys.stdout``/``sys.stderr`` to UTF-8 with ``errors="replace"``.

    The desktop host decodes worker stdio as UTF-8, but on zh-CN Windows a
    piped stdio defaults to cp936 (vendored Python 3.12 predates PEP 686), so
    the first non-ASCII line would emit GBK bytes and kill the host's line
    reader (P0-2). Call this at the top of every entry point before any
    logging/print. Idempotent: ``reconfigure`` on an already-UTF-8 stream is a
    no-op; streams without ``reconfigure`` (e.g. ``io.StringIO`` under pytest
    capture) are re-wrapped via their binary buffer when possible and
    otherwise left untouched — stdio repair must never take the worker down.
    """
    for name in ("stdout", "stderr"):
        stream = getattr(sys, name, None)
        try:
            reconfigure = getattr(stream, "reconfigure", None)
            if callable(reconfigure):
                reconfigure(encoding="utf-8", errors="replace")
            elif (
                getattr(stream, "buffer", None) is not None
                and (getattr(stream, "encoding", None) or "").replace("-", "").lower() != "utf8"
            ):
                # Real binary pipe whose text layer lacks reconfigure: wrap it.
                setattr(
                    sys,
                    name,
                    io.TextIOWrapper(stream.buffer, encoding="utf-8", errors="replace", line_buffering=True),
                )
        except Exception:
            pass
