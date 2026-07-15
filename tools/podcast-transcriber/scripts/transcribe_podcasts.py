from __future__ import annotations

import argparse
import atexit
import copy
import hashlib
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import Future, ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

from deepseek_pricing import (
    DEEPSEEK_DEFAULT_MODEL,
    PodcastUpstreamError,
    PromptBudgetError,
    classify_upstream_error,
)

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(ROOT / "scripts"))
# job_store removed during v2 refactor — DB is no longer used

# ---- Split-out modules (re-exported for backwards compatibility:
# `import transcribe_podcasts as tp` keeps exposing these names) ----
from podcast_transcriber.common import (  # noqa: E402, F401
    CHUNKS_DIR,
    CONFIG_PATH,
    CURRENT_RUN_ID,
    DEFAULT_AUTO_TRANSLATE_LANGUAGES,
    INBOX,
    INTERNAL,
    INVALID_FILENAME_CHARS,
    LIFECYCLE_LOG_PATH,
    MANIFEST_PATH,
    MODELS_DIR,
    NORMALIZED_DIR,
    OUT_FINAL,
    OUT_FINAL_MARKDOWN,
    OUT_JSON,
    OUT_LOGS,
    OUT_MARKDOWN,
    OUT_MARKDOWN_BILINGUAL,
    OUT_REPORTS,
    OUT_SRT,
    OUTPUT,
    RUN_LOCK_PATH,
    SIDECAR_SUBTITLE_EXTENSIONS,
    STATE_DIR,
    STATE_IO_LOCK,
    SUPPORTED_EXTENSIONS,
    VIDEO_EXTENSIONS,
    WORK,
    ensure_dirs,
    iso_now,
    load_json,
    now_stamp,
    resolve_model_reference,
    save_json,
)
from podcast_transcriber.deepseek import (  # noqa: E402, F401
    DEEPSEEK_API_CONFIG_LIMIT,
    DEEPSEEK_API_SEMAPHORE,
    DEEPSEEK_API_SEMAPHORE_LOCK,
    DEEPSEEK_PROMPT_TOKEN_HARD_LIMIT,
    DEEPSEEK_PROMPT_TOKEN_LIMIT,
    DEFAULT_DEEPSEEK_TRANSLATION_BATCH_CHARS,
    DEFAULT_DEEPSEEK_TRANSLATION_BATCH_SEGMENTS,
    DEFAULT_OLLAMA_TRANSLATION_BATCH_CHARS,
    DEFAULT_OLLAMA_TRANSLATION_BATCH_SEGMENTS,
    DeepSeekApiLimiter,
    DeepSeekLengthTruncatedError,
    _dynamic_throttle_deepseek,
    assert_deepseek_prompt_budget,
    deepseek_api_semaphore,
    deepseek_chat_completion,
    deepseek_prompt_limit,
    effective_provider_name,
    estimate_deepseek_cost,
    estimate_text_tokens,
    has_api_entry,
    model_label,
    positive_int,
    provider_name,
    record_deepseek_usage,
    reserve_budget,
    resolve_api_key,
    settle_budget,
    text_hash,
    translation_batch_char_limit,
    translation_batch_segments,
)
from podcast_transcriber.state import (  # noqa: E402, F401
    ACTIVE_TASK_STATUSES,
    MANIFEST_LOCK,
    TERMINAL_TASK_STATUSES,
    TaskHeartbeat,
    _state_is_owned_terminal_on_disk,
    append_lifecycle_log,
    load_manifest,
    mark_task_failed,
    save_task_state_safe,
    touch_task_heartbeat,
    update_manifest_entry,
    update_manifest_processed,
    update_task_state,
)

# DB_PATH removed — SQLite job store is no longer used

DLL_DIRECTORY_HANDLES: list[Any] = []
DLL_DIRECTORY_PATHS: set[str] = set()
MODEL_TRANSCRIBE_LOCK = threading.Lock()
TRANSLATION_SEMAPHORE_LOCK = threading.Lock()
TRANSLATION_SEMAPHORES: dict[int, threading.Semaphore] = {}
OLLAMA_PROCESS: subprocess.Popen[Any] | None = None
RUN_LOCK_ACQUIRED = False


def pipeline_config(config: dict[str, Any]) -> dict[str, Any]:
    return config.get("pipeline") or {}


def max_parallel_audio_files(config: dict[str, Any]) -> int:
    value = pipeline_config(config).get("max_parallel_audio_files", 1)
    try:
        return max(1, min(3, int(value)))
    except (TypeError, ValueError):
        return 1


def max_parallel_translations(config: dict[str, Any]) -> int:
    value = pipeline_config(config).get("max_parallel_translations", 2)
    try:
        return max(1, min(8, int(value)))
    except (TypeError, ValueError):
        return 2


def max_parallel_postprocess_files(config: dict[str, Any]) -> int:
    value = pipeline_config(config).get("max_parallel_postprocess_files", 1)
    try:
        return max(1, min(4, int(value)))
    except (TypeError, ValueError):
        return 1


def transcribe_lock_yield_seconds(config: dict[str, Any]) -> float:
    value = pipeline_config(config).get("transcribe_lock_yield_seconds", 0.35)
    try:
        return max(0.0, min(3.0, float(value)))
    except (TypeError, ValueError):
        return 0.35


def translation_semaphore(config: dict[str, Any]) -> threading.Semaphore:
    limit = max_parallel_translations(config)
    with TRANSLATION_SEMAPHORE_LOCK:
        sem = TRANSLATION_SEMAPHORES.get(limit)
        if sem is None:
            sem = threading.Semaphore(limit)
            TRANSLATION_SEMAPHORES[limit] = sem
        return sem


def resolve_project_path(path_value: str | None, fallback: Path) -> Path:
    if not path_value or not str(path_value).strip():
        return fallback
    candidate = Path(str(path_value))
    if candidate.is_absolute():
        return candidate
    return ROOT / candidate


def should_open_output(config: dict[str, Any], no_open_output: bool = False) -> bool:
    if no_open_output:
        return False
    if os.environ.get("PODCAST_TRANSCRIBER_NO_OPEN_OUTPUT") == "1":
        return False
    launcher = config.get("launcher") or {}
    return bool(launcher.get("open_folder_after_run", True))


def open_output_folder(config: dict[str, Any], logger: logging.Logger | None = None, no_open_output: bool = False) -> None:
    if not should_open_output(config, no_open_output):
        return
    launcher = config.get("launcher") or {}
    folder = resolve_project_path(str(launcher.get("open_folder") or "output"), OUT_FINAL_MARKDOWN)
    if not folder.exists():
        return
    try:
        if sys.platform == "win32":
            subprocess.Popen(["explorer.exe", str(folder)])
        else:
            opener = "open" if sys.platform == "darwin" else "xdg-open"
            subprocess.Popen([opener, str(folder)])
        print(f"Opened output folder: {folder}")
    except Exception as exc:
        if logger:
            logger.warning("Could not open output folder %s: %s", folder, exc)
        else:
            print(f"Could not open output folder {folder}: {exc}")


def sanitize_name(name: str) -> str:
    cleaned = "".join("_" if c in INVALID_FILENAME_CHARS else c for c in name)
    cleaned = re.sub(r"\s+", " ", cleaned).strip().rstrip(".")
    return cleaned or "audio"


def is_file_stable(path: Path, wait: float = 2.0, checks: int = 3) -> bool:
    """Check if file is stable by polling size and mtime.
    
    A file is stable if its size and mtime remain unchanged across
    `checks` polls, each `wait` seconds apart.
    """
    try:
        prev_stat = path.stat()
        for _ in range(checks - 1):
            time.sleep(wait)
            curr_stat = path.stat()
            if prev_stat.st_size != curr_stat.st_size or prev_stat.st_mtime_ns != curr_stat.st_mtime_ns:
                return False
            prev_stat = curr_stat
        return True
    except OSError:
        return False


def cleanup_stale_tmp_files(logger: logging.Logger | None = None) -> None:
    threshold = time.time() - 86400
    for directory in [STATE_DIR, OUT_JSON, OUT_MARKDOWN, OUT_MARKDOWN_BILINGUAL, OUT_SRT, CHUNKS_DIR]:
        if not directory.exists():
            continue
        for path in directory.glob("*.tmp"):
            try:
                if path.stat().st_mtime < threshold:
                    path.unlink()
                    if logger:
                        logger.info("Cleaned stale tmp file: %s", path)
            except OSError:
                pass


def is_durable_path(path: Path) -> bool:
    """Return True for runtime state that must survive worker restarts."""
    try:
        resolved = path.resolve()
        state_root = STATE_DIR.resolve()
        if resolved == state_root or state_root in resolved.parents:
            return True
        lifecycle_log = LIFECYCLE_LOG_PATH.resolve()
        return resolved == lifecycle_log
    except OSError:
        return False


def cleanup_work_artifacts(logger: logging.Logger | None = None, retention_seconds: int = 86400) -> None:
    """Selectively remove stale work artifacts without deleting durable state.

    Worker startup must preserve work/state, jobs.sqlite3, active state JSON files,
    and lifecycle logs. Cleanup removes .tmp files under work immediately, then
    removes non-durable runtime files older than the retention period.
    """
    if not WORK.exists():
        return
    cutoff = time.time() - retention_seconds
    for path in sorted(WORK.rglob("*"), key=lambda item: len(item.parts), reverse=True):
        if is_durable_path(path):
            continue
        try:
            if path.is_dir():
                if path.stat().st_mtime >= cutoff:
                    continue
                try:
                    path.rmdir()
                    if logger:
                        logger.info("Cleaned empty work directory: %s", path)
                except OSError:
                    pass
                continue
            if path.suffix != ".tmp" and path.stat().st_mtime >= cutoff:
                continue
            path.unlink()
            if logger:
                logger.info("Cleaned stale work artifact: %s", path)
        except OSError as exc:
            if logger:
                logger.warning("Could not clean work artifact %s: %s", path, exc)
    ensure_dirs()


def acquire_run_lock(logger: logging.Logger | None = None) -> bool:
    global RUN_LOCK_ACQUIRED
    payload = json.dumps({"pid": os.getpid(), "started_at": now_stamp()}, ensure_ascii=False)
    try:
        fd = os.open(str(RUN_LOCK_PATH), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(payload)
        RUN_LOCK_ACQUIRED = True
        return True
    except (FileExistsError, PermissionError):
        try:
            existing = load_json(RUN_LOCK_PATH, {})
            existing_pid = int(existing.get("pid") or 0) if isinstance(existing, dict) else 0
            age = time.time() - RUN_LOCK_PATH.stat().st_mtime
            if (existing_pid and not process_is_running(existing_pid)) or age > 12 * 3600:
                RUN_LOCK_PATH.unlink()
                return acquire_run_lock(logger)
        except OSError:
            pass
        if logger:
            logger.error("Another PodcastTranscriber run appears active: %s", RUN_LOCK_PATH)
        return False


def process_is_running(pid: int) -> bool:
    if pid <= 0:
        return False
    if sys.platform == "win32":
        try:
            import ctypes  # noqa: E402
            kernel32 = ctypes.windll.kernel32
            PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
            handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
            if not handle:
                return False
            exit_code = ctypes.c_ulong()
            if kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code)):
                kernel32.CloseHandle(handle)
                # STILL_ACTIVE (259) means the process is still running.
                # Any other exit code means the process has already terminated.
                return exit_code.value == 259
            kernel32.CloseHandle(handle)
            return False
        except Exception:
            return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def release_run_lock() -> None:
    global RUN_LOCK_ACQUIRED
    if not RUN_LOCK_ACQUIRED:
        return
    try:
        RUN_LOCK_PATH.unlink(missing_ok=True)
    except OSError:
        pass
    RUN_LOCK_ACQUIRED = False


def cleanup_dll_directory_handles() -> None:
    while DLL_DIRECTORY_HANDLES:
        handle = DLL_DIRECTORY_HANDLES.pop()
        close = getattr(handle, "close", None)
        if close:
            try:
                close()
            except OSError as exc:
                logger = logging.getLogger("cleanup")
                logger.warning("Could not close DLL directory handle: %s", exc)
    DLL_DIRECTORY_PATHS.clear()


def cleanup_child_processes() -> None:
    global OLLAMA_PROCESS
    proc = OLLAMA_PROCESS
    if proc and proc.poll() is None:
        if os.name == "nt":
            try:
                subprocess.run(["taskkill", "/PID", str(proc.pid), "/T"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
                for _ in range(20):
                    if proc.poll() is not None:
                        break
                    time.sleep(0.1)
                if proc.poll() is None:
                    subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            except Exception:
                pass
        else:
            try:
                proc.terminate()
            except OSError:
                pass
    OLLAMA_PROCESS = None
    ollama_pid_path = STATE_DIR / "ollama.pid"
    if ollama_pid_path.exists():
        try:
            ollama_pid_path.unlink()
        except Exception:
            pass


atexit.register(release_run_lock)
atexit.register(cleanup_dll_directory_handles)
atexit.register(cleanup_child_processes)


def validate_chunks_metadata(
    chunk_dir: Path,
    source: Path,
    chunk_seconds: int,
    logger: logging.Logger,
    ffprobe: str | None = None,
) -> bool:
    meta_path = chunk_dir / "chunk_metadata.json"
    if not meta_path.exists():
        logger.info("No chunk metadata found; will regenerate chunks.")
        return False
    try:
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
    except Exception:
        logger.warning("Chunk metadata is corrupted; will regenerate chunks.")
        return False

    expected_fp = file_fingerprint(source)
    if meta.get("source_fingerprint") != expected_fp:
        logger.info("Source fingerprint changed; will regenerate chunks.")
        return False
    if int(meta.get("chunk_seconds", 0)) != chunk_seconds:
        logger.info("chunk_seconds config changed; will regenerate chunks.")
        return False

    chunk_files = sorted(chunk_dir.glob("chunk_*.wav"))
    if not chunk_files:
        logger.info("No chunk files found despite metadata; will regenerate chunks.")
        return False

    actual_count = meta.get("actual_chunks")
    if actual_count is not None and len(chunk_files) != actual_count:
        logger.info("Chunk count mismatch (%s vs %s); will regenerate chunks.", len(chunk_files), actual_count)
        return False

    total_chunk_duration = 0.0
    for i, chunk_path in enumerate(chunk_files):
        try:
            dur = probe_duration(chunk_path, ffprobe, logger)
        except Exception:
            logger.warning("Cannot probe chunk %s; will regenerate chunks.", chunk_path.name)
            return False
        if dur is None or dur <= 0:
            logger.warning("Chunk %s has invalid duration; will regenerate chunks.", chunk_path.name)
            return False
        total_chunk_duration += dur
        if i < len(chunk_files) - 1 and dur < chunk_seconds * 0.5:
            logger.info("Non-last chunk %s unexpectedly short (%ss); will regenerate chunks.", chunk_path.name, dur)
            return False

    source_duration = probe_duration(source, ffprobe, logger)
    if source_duration and source_duration > 0:
        ratio = total_chunk_duration / source_duration
        if ratio < 0.95 or ratio > 1.05:
            logger.info("Total chunk duration ratio %.2f outside tolerance; will regenerate chunks.", ratio)
            return False

    logger.info("Chunk metadata validated; reusing existing chunks.")
    return True


def quarantine_chunks(chunk_dir: Path, task_id: str, logger: logging.Logger) -> None:
    quarantine = CHUNKS_DIR / "quarantine" / f"{task_id}_{int(time.time())}"
    try:
        quarantine.parent.mkdir(parents=True, exist_ok=True)
        chunk_dir.rename(quarantine)
        logger.info("Moved invalid chunks to quarantine: %s", quarantine)
    except OSError as exc:
        logger.warning("Could not quarantine chunks: %s", exc)


def format_hms(seconds: float | int | None) -> str:
    if seconds is None:
        return "未知"
    total = max(0, int(seconds))
    h, rem = divmod(total, 3600)
    m, s = divmod(rem, 60)
    return f"{h:02d}:{m:02d}:{s:02d}"


def format_srt_time(seconds: float) -> str:
    ms_total = max(0, int(round(seconds * 1000)))
    h, rem = divmod(ms_total, 3_600_000)
    m, rem = divmod(rem, 60_000)
    s, ms = divmod(rem, 1000)
    return f"{h:02d}:{m:02d}:{s:02d},{ms:03d}"


def force_translate_override() -> bool | None:
    """TaskSpec.options.translate maps to PODCAST_TRANSCRIBER_FORCE_TRANSLATE=0|1."""
    raw = str(os.environ.get("PODCAST_TRANSCRIBER_FORCE_TRANSLATE", "")).strip().lower()
    if raw in {"0", "false", "no", "off"}:
        return False
    if raw in {"1", "true", "yes", "on"}:
        return True
    return None


def is_translation_enabled(config: dict[str, Any]) -> bool:
    force = force_translate_override()
    if force is False:
        return False
    if force is True:
        return True
    translation = config.get("translation") or {}
    return bool(translation.get("enabled", translation.get("english_required", False)))


def asr_config(config: dict[str, Any]) -> dict[str, Any]:
    merged = dict(config)
    merged.update(config.get("asr") or {})
    if merged.get("model"):
        merged["model"] = resolve_model_reference(merged["model"])
    if merged.get("fallback_models"):
        merged["fallback_models"] = [resolve_model_reference(model) for model in merged["fallback_models"]]
    return merged


def audio_config(config: dict[str, Any]) -> dict[str, Any]:
    merged = {
        "supported_extensions": config.get("supported_extensions", list(SUPPORTED_EXTENSIONS)),
        "normalize_to_wav": config.get("normalize_to_wav", True),
        "sample_rate": config.get("sample_rate", 16000),
        "channels": config.get("channels", 1),
    }
    merged.update(config.get("audio") or {})
    return merged


def normalize_language_code(language: Any) -> str:
    return str(language or "").strip().lower().split("-")[0]


def configured_auto_translate_languages(config: dict[str, Any]) -> set[str]:
    translation = config.get("translation") or {}
    if translation.get("english_required", False):
        value = ["en"]
    else:
        value = translation.get("auto_when_detected_languages", ["en"])
    if value is True:
        return set(DEFAULT_AUTO_TRANSLATE_LANGUAGES)
    if value in (False, None):
        return set()
    if isinstance(value, str):
        return {normalize_language_code(item) for item in re.split(r"[,;\s]+", value) if item.strip()}
    if isinstance(value, list):
        return {normalize_language_code(item) for item in value if normalize_language_code(item)}
    return set(DEFAULT_AUTO_TRANSLATE_LANGUAGES)


def infer_segments_language(segments: list[dict[str, Any]]) -> str | None:
    sample = " ".join(str(segment.get("text", "")) for segment in segments[:80])
    if not sample.strip():
        return None
    cjk_count = len(re.findall(r"[\u3400-\u9fff]", sample))
    latin_count = len(re.findall(r"[A-Za-z]", sample))
    if latin_count >= 80 and latin_count >= cjk_count * 4:
        return "en"
    if cjk_count >= 20 and cjk_count >= latin_count:
        return "zh"
    return None


def should_translate_for_language(config: dict[str, Any], detected_language: Any, segments: list[dict[str, Any]] | None = None) -> bool:
    translation = config.get("translation") or {}
    if not is_translation_enabled(config):
        return False
    backend = str(translation.get("backend", translation.get("provider", "ollama"))).lower()
    if backend == "none":
        return False
    force = force_translate_override()
    if segments is not None:
        from podcast_transcriber.language import assign_language_classes, segment_needs_translation  # noqa: E402

        assign_language_classes(segments, detected_language)
        if any(segment_needs_translation(segment) for segment in segments):
            return True
        # Pure Chinese (or empty) content: skip the translation service.
        return False
    # No segments yet (budget preflight): keep conservative upper bound.
    if force is True:
        return True
    detected = normalize_language_code(detected_language)
    return detected in configured_auto_translate_languages(config)


def has_missing_translations(segments: list[dict[str, Any]]) -> bool:
    from podcast_transcriber.language import assign_language_classes, segment_needs_translation  # noqa: E402

    assign_language_classes(segments, None)
    return any(
        segment_needs_translation(segment) and not segment.get("translation")
        for segment in segments
    )


def has_invalid_translations(segments: list[dict[str, Any]]) -> bool:
    from podcast_transcriber.language import assign_language_classes, segment_needs_translation  # noqa: E402

    assign_language_classes(segments, None)
    return any(
        segment_needs_translation(segment)
        and segment.get("translation")
        and (
            not is_valid_translation_text(str(segment.get("translation", "")))
            or is_controlled_missing_translation(str(segment.get("translation", "")))
        )
        for segment in segments
    )


TRANSLATION_MISSING_PREFIX = "[翻译缺失："


def is_controlled_missing_translation(text: str) -> bool:
    return text.strip().startswith(TRANSLATION_MISSING_PREFIX)


def translation_response_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "translations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer"},
                        "text": {"type": "string"},
                    },
                    "required": ["id", "text"],
                },
            }
        },
        "required": ["translations"],
    }


def ollama_generate(prompt: str, translation_config: dict[str, Any], response_format: Any | None = None) -> str:
    url = str(translation_config.get("ollama_url", "http://127.0.0.1:11434/api/generate"))
    payload = {
        "model": translation_config.get("model", "qwen3.5:9b"),
        "prompt": prompt,
        "stream": False,
        "keep_alive": translation_config.get("keep_alive", "30m"),
        "think": bool(translation_config.get("think", False)),
        "options": {
            "temperature": float(translation_config.get("temperature", 0)),
            "num_ctx": int(translation_config.get("num_ctx", 8192)),
            "num_predict": int(translation_config.get("num_predict", 2048)),
            "num_thread": int(translation_config.get("num_thread", 4)),
        },
    }
    if response_format is not None:
        payload["format"] = response_format
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"}, method="POST")
    timeout = int(translation_config.get("timeout_seconds", 180))
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = json.loads(response.read().decode("utf-8"))
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, OSError) as exc:
        classified = classify_upstream_error(exc, "Ollama")
        if classified:
            raise classified from exc
        raise
    return str(body.get("response", "")).strip()


BAD_TRANSLATION_PATTERNS = (
    r"^\s*\{['\"][^'\"]+['\"]\s*:",
    r"^\s*\{[^}]*['\"](?:text|translation|sponsor)['\"]\s*:",
    r"\b(?:Here is|Here are|The translation|Translated text)\b",
    r"翻译如下",
    r"译文如下",
    r"应保留或译为",
    r"不要解释",
)


def is_valid_translation_text(text: str) -> bool:
    text = text.strip()
    if not text:
        return False
    if is_controlled_missing_translation(text):
        return False
    if text in {"[", "]", "(", ")", '"', "'"}:
        return False
    if any(re.search(pattern, text, flags=re.IGNORECASE) for pattern in BAD_TRANSLATION_PATTERNS):
        return False
    return True


def strip_json_fence(response: str) -> str:
    cleaned = response.strip()
    if cleaned.startswith("```"):
        cleaned = re.sub(r"^```(?:json|text)?\s*", "", cleaned)
        cleaned = re.sub(r"\s*```$", "", cleaned)
    return cleaned.strip()


def extract_json_response(response: str) -> Any:
    cleaned = strip_json_fence(response)
    candidates = [cleaned]

    first_obj = cleaned.find("{")
    last_obj = cleaned.rfind("}")
    if 0 <= first_obj < last_obj:
        candidates.append(cleaned[first_obj : last_obj + 1])

    first_arr = cleaned.find("[")
    last_arr = cleaned.rfind("]")
    if 0 <= first_arr < last_arr:
        candidates.append(cleaned[first_arr : last_arr + 1])

    seen: set[str] = set()
    for candidate in candidates:
        for variant in (
            candidate,
            candidate.replace("“", '"').replace("”", '"').replace("‘", "'").replace("’", "'"),
        ):
            if not variant or variant in seen:
                continue
            seen.add(variant)
            try:
                return json.loads(variant)
            except json.JSONDecodeError:
                continue
    raise RuntimeError("Translation response is not valid JSON.")


class _PartialTranslationError(Exception):
    """Some segments translated successfully but others had empty/invalid text."""

    def __init__(self, partial_results: dict[int, str], failed_ids: list[int]):
        self.partial_results = partial_results
        self.failed_ids = failed_ids
        super().__init__(f"Partial translation: {len(partial_results)} ok, {len(failed_ids)} empty (ids {failed_ids[:5]})")


def validate_translation_payload(payload: Any, batch_segments: list[dict[str, Any]]) -> dict[int, str]:
    expected_ids = [int(segment.get("id") or index + 1) for index, segment in enumerate(batch_segments)]

    if isinstance(payload, dict) and "translations" in payload:
        items = payload.get("translations")
    elif isinstance(payload, dict) and len(batch_segments) == 1:
        items = [payload]
    elif isinstance(payload, list):
        items = payload
    else:
        raise RuntimeError("Translation response must be an object with translations.")

    if not isinstance(items, list):
        raise RuntimeError("Translation response translations must be a list.")

    by_id: dict[int, str] = {}
    empty_ids: list[int] = []
    if items and all(isinstance(item, dict) and "id" in item for item in items):
        for item in items:
            try:
                item_id = int(item.get("id"))
            except (TypeError, ValueError) as exc:
                raise RuntimeError("Translation item has invalid id.") from exc
            if item_id in by_id:
                raise RuntimeError(f"Translation response duplicated id {item_id}.")
            text = clean_translation_text(item)
            if not is_valid_translation_text(text):
                empty_ids.append(item_id)
                continue
            by_id[item_id] = text
        if empty_ids and not by_id:
            raise RuntimeError(f"Invalid translation text for id {empty_ids[0]}:")
        missing = [item_id for item_id in expected_ids if item_id not in by_id]
        extra = [item_id for item_id in by_id if item_id not in expected_ids]
        if not by_id:
            raise RuntimeError(f"Translation ids mismatch. missing={missing[:5]} extra={extra[:5]}")
        if empty_ids:
            raise _PartialTranslationError(by_id, empty_ids)
        if missing or extra:
            raise RuntimeError(f"Translation ids mismatch. missing={missing[:5]} extra={extra[:5]}")
        return by_id

    if len(items) != len(batch_segments):
        raise RuntimeError(f"Expected {len(batch_segments)} translations, got {len(items)}.")
    for item_id, item in zip(expected_ids, items, strict=True):
        text = clean_translation_text(item)
        if not is_valid_translation_text(text):
            empty_ids.append(item_id)
            continue
        by_id[item_id] = text
    if empty_ids and not by_id:
        raise RuntimeError(f"Invalid translation text for id {empty_ids[0]}:")
    if empty_ids:
        raise _PartialTranslationError(by_id, empty_ids)
    return by_id


def parse_translation_lines(response: str, expected_count: int, strict: bool = True) -> list[str]:
    try:
        payload = extract_json_response(response)
        batch = [{"id": index + 1, "text": ""} for index in range(expected_count)]
        by_id = validate_translation_payload(payload, batch)
        return [by_id[index + 1] for index in range(expected_count)]
    except RuntimeError:
        if strict:
            raise

    if strict:
        raise RuntimeError("Translation response must be a JSON array of strings.")

    lines: list[str] = []
    for raw in strip_json_fence(response).splitlines():
        line = raw.strip()
        if not line:
            continue
        line = clean_translation_text(line)
        if is_valid_translation_text(line):
            lines.append(line)
    return lines[:expected_count]


def clean_translation_text(value: Any) -> str:
    if isinstance(value, dict):
        for key in ("translation", "text", "zh", "zh-CN", "target", "output"):
            if key in value:
                return clean_translation_text(value.get(key))
        return ""
    if isinstance(value, list):
        value = " ".join(clean_translation_text(item) for item in value if clean_translation_text(item))
    text = str(value).strip()
    if text in {"[", "]", "(", ")", '"', "'"}:
        return ""
    text = re.sub(r"^\s*(?:[-*]|\d+[.)、:：])\s*", "", text).strip()
    for _ in range(3):
        unwrapped = re.sub(r"""^\s*[\[\("']\s*(.*?)\s*[\]\)"']\s*$""", r"\1", text).strip()
        if unwrapped == text:
            break
        text = unwrapped
        text = re.sub(r"^\s*(?:[-*]|\d+[.)、:：])\s*", "", text).strip()
    return text


def generate_translation_text(
    prompt: str,
    translation_config: dict[str, Any],
    state: dict[str, Any],
    state_path: Path,
    logger: logging.Logger,
    response_format: Any | None = None,
    _job_id: str | None = None,
    _root_config: dict[str, Any] | None = None,
    _state_lock: threading.RLock | None = None,
) -> str:
    backend = effective_provider_name(translation_config)
    if backend == "deepseek":
        reservation = reserve_budget(prompt, translation_config)
        with TaskHeartbeat(state_path, state, status="translating", stage="translating", _job_id=_job_id):
            try:
                response, usage, elapsed = deepseek_chat_completion(
                    prompt,
                    translation_config,
                    response_format=response_format,
                    _root_config=_root_config,
                )
            except Exception:
                settle_budget(reservation, None, translation_config)
                raise
            settle_budget(reservation, usage, translation_config)
        if _state_lock is not None:
            with _state_lock:
                record_deepseek_usage(
                    state,
                    "translation",
                    str(translation_config.get("model") or DEEPSEEK_DEFAULT_MODEL),
                    usage,
                    elapsed,
                    translation_config,
                )
                save_task_state_safe(state_path, state)
        else:
            record_deepseek_usage(state, "translation", str(translation_config.get("model") or DEEPSEEK_DEFAULT_MODEL), usage, elapsed, translation_config)
            save_task_state_safe(state_path, state)
        logger.info(
            "DeepSeek translation request completed in %.2fs, tokens=%s, estimated_cost=$%.6f",
            elapsed,
            usage.get("total_tokens", "?"),
            estimate_deepseek_cost(usage, translation_config),
        )
        return response
    if backend == "ollama":
        with TaskHeartbeat(state_path, state, status="translating", stage="translating", _job_id=_job_id):
            return ollama_generate(prompt, translation_config, response_format)
    raise RuntimeError(f"Unsupported translation backend: {backend}")


def translate_segments_with_llm(
    segments: list[dict[str, Any]],
    config: dict[str, Any],
    state: dict[str, Any],
    state_path: Path,
    logger: logging.Logger,
    _job_id: str | None = None,
) -> list[dict[str, Any]]:
    translation_config = config.get("translation") or {}
    batch_size = translation_batch_segments(translation_config)
    batch_char_limit = translation_batch_char_limit(translation_config)
    target_language = translation_config.get("target_language", "zh-CN")
    source_language = translation_config.get("source_language", "en")
    model_name = model_label(translation_config)
    strict_output = bool(translation_config.get("strict_output", True))
    max_batch_retries = 2
    max_single_retries = 2
    translation_state_lock = threading.RLock()

    from podcast_transcriber.language import assign_language_classes, segment_needs_translation  # noqa: E402

    translated = [dict(segment) for segment in segments]
    assign_language_classes(translated, state.get("detected_language") or (config.get("asr") or {}).get("language"))

    cache_entries = state.get("translation_cache", [])
    cache_map: dict[str, str] = {}
    for entry in cache_entries:
        if (
            entry.get("source_hash")
            and entry.get("translation")
            and entry.get("target_language") == target_language
            and entry.get("model") == model_name
            and (not strict_output or is_valid_translation_text(str(entry.get("translation", ""))))
        ):
            cache_map[entry["source_hash"]] = entry["translation"]

    for segment in translated:
        seg_hash = text_hash(str(segment.get("text", "")))
        if seg_hash in cache_map and segment_needs_translation(segment):
            segment["translation"] = cache_map[seg_hash]

    # Chinese blocks never enter the translation service.
    translatable = [segment for segment in translated if segment_needs_translation(segment)]
    state["translation_total"] = len(translatable)
    state["translation_errors"] = list(state.get("translation_errors") or [])

    def done_count() -> int:
        return sum(1 for segment in translatable if segment.get("translation"))

    def failed_count() -> int:
        return sum(1 for segment in translatable if is_controlled_missing_translation(str(segment.get("translation", ""))))

    def persist_translations() -> None:
        with translation_state_lock:
            state["translation_cache"] = [
                {
                    "source_hash": text_hash(str(segment.get("text", ""))),
                    "translation": segment.get("translation", ""),
                    "target_language": target_language,
                    "model": model_name,
                }
                for segment in translated
                if segment.get("translation") and is_valid_translation_text(str(segment.get("translation", "")))
            ]
            state["translation_done"] = done_count()
            state["translation_failed"] = failed_count()
            state["status"] = "translating"
            state["stage"] = "translating"
            state["updated_at"] = iso_now()
            state["last_update_at"] = state["updated_at"]
            state["last_heartbeat_at"] = state["updated_at"]
            state["worker_pid"] = os.getpid()
            save_task_state_safe(state_path, state)

    pending = [
        segment
        for segment in translated
        if segment_needs_translation(segment)
        and (
            not segment.get("translation")
            or is_controlled_missing_translation(str(segment.get("translation", "")))
        )
    ]
    persist_translations()
    if not pending:
        return translated

    def make_missing_translation(segment: dict[str, Any], reason: str) -> str:
        short_reason = re.sub(r"\s+", " ", reason).strip()[:140] or "翻译失败"
        return f"{TRANSLATION_MISSING_PREFIX}{short_reason}]"

    # 跨批上下文与术语表：批次是并行翻译的，因此上下文只用「源文侧」内容（不依赖其他批的译文输出）。
    ordered_source_ids: list[int] = []
    source_text_by_id: dict[int, str] = {}
    for idx, segment in enumerate(translated):
        if not segment.get("text"):
            continue
        seg_id = int(segment.get("id") or idx + 1)
        ordered_source_ids.append(seg_id)
        source_text_by_id[seg_id] = str(segment.get("text", ""))
    source_pos_by_id = {seg_id: pos for pos, seg_id in enumerate(ordered_source_ids)}
    glossary = translation_config.get("glossary") or {}

    def preceding_source_context(batch_segments: list[dict[str, Any]], count: int = 2, max_chars: int = 600) -> str:
        first_id = int(batch_segments[0].get("id") or 0)
        pos = source_pos_by_id.get(first_id)
        if not pos:
            return ""
        prev_ids = ordered_source_ids[max(0, pos - count):pos]
        context = " ".join(source_text_by_id.get(i, "") for i in prev_ids).strip()
        return context[-max_chars:] if context else ""

    def build_translation_prompt(batch_segments: list[dict[str, Any]]) -> str:
        payload = [
            {"id": int(segment.get("id") or idx + 1), "text": str(segment.get("text", ""))}
            for idx, segment in enumerate(batch_segments)
        ]
        glossary_block = ""
        if isinstance(glossary, dict) and glossary:
            pairs = "; ".join(f"{k} -> {v}" for k, v in list(glossary.items())[:50])
            glossary_block = f"Glossary (always use these exact translations): {pairs}\n"
        context_text = preceding_source_context(batch_segments)
        context_block = (
            f"Preceding transcript context (for continuity only, do NOT translate or include it in output):\n{context_text}\n\n"
            if context_text
            else ""
        )
        prompt = (
            "/no_think\n"
            f"Translate the following {source_language} podcast transcript segments into natural, fluent {target_language} for a readable podcast transcript.\n"
            "Keep technical terms accurate. Preserve names, product names, and acronyms when appropriate.\n"
            "Translate recurring technical terms, product names, and person names the same way every time they appear.\n"
            f"{glossary_block}"
            "Keep one output item for every input id, and do not omit or reorder ids.\n"
            "Inside each item's text, write idiomatic human Chinese instead of machine-translation Chinese.\n"
            "Translate faithfully and completely: do not summarize, omit, or add information. Stylistic cleanup is handled by a later polish step, not here.\n"
            "If an English sentence is cut across adjacent segments, translate the current fragment so it connects naturally in Chinese without inventing missing facts.\n"
            "Do not explain, do not include notes, and do not include thinking.\n"
            "Return only this JSON shape: {\"translations\":[{\"id\":123,\"text\":\"translated text\"}]}.\n"
            "Every returned id must exactly match one input id.\n\n"
            f"{context_block}"
            f"Segments JSON:\n{json.dumps(payload, ensure_ascii=False)}"
        )
        return prompt

    def build_translation_batches(batch_segments: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
        batches: list[list[dict[str, Any]]] = []
        current: list[dict[str, Any]] = []
        current_chars = 0
        for segment in batch_segments:
            text_chars = len(str(segment.get("text", "")))
            would_exceed_count = len(current) >= batch_size
            would_exceed_chars = current_chars + text_chars > batch_char_limit
            if current and (would_exceed_count or would_exceed_chars):
                batches.append(current)
                current = []
                current_chars = 0
            current.append(segment)
            current_chars += text_chars
        if current:
            batches.append(current)
        return batches

    def request_translation_batch(batch_segments: list[dict[str, Any]]) -> dict[int, str]:
        prompt = build_translation_prompt(batch_segments)
        estimated_tokens = estimate_text_tokens(prompt)
        soft_limit = deepseek_prompt_limit(translation_config)
        if estimated_tokens > soft_limit:
            raise PromptBudgetError(f"Translation prompt estimated at {estimated_tokens} tokens, above split limit {soft_limit}.")
        response = generate_translation_text(
            prompt,
            translation_config,
            state,
            state_path,
            logger,
            translation_response_schema(),
            _job_id=_job_id,
            _root_config=config,
            _state_lock=translation_state_lock,
        )
        payload = extract_json_response(response)
        return validate_translation_payload(payload, batch_segments)

    def request_with_retries(batch_segments: list[dict[str, Any]], retries: int) -> dict[int, str]:
        last_exc: Exception | None = None
        for attempt in range(retries + 1):
            try:
                return request_translation_batch(batch_segments)
            except PromptBudgetError:
                raise
            except _PartialTranslationError:
                raise
            except DeepSeekLengthTruncatedError:
                raise
            except PodcastUpstreamError:
                raise
            except Exception as exc:
                last_exc = exc
                if attempt < retries:
                    logger.warning(
                        "Translation request failed, retrying (%s/%s): %s",
                        attempt + 1,
                        retries,
                        exc,
                    )
                    touch_task_heartbeat(state_path, state, status="translating", stage="translating", _job_id=_job_id)
        assert last_exc is not None
        raise RuntimeError(str(last_exc)) from last_exc

    def apply_translation_results(results: dict[int, str]) -> None:
        with translation_state_lock:
            by_id = {int(segment.get("id") or 0): segment for segment in translated}
            for segment_id, translation in results.items():
                if segment_id in by_id:
                    by_id[segment_id]["translation"] = translation
            persist_translations()

    def mark_translation_failure(segment: dict[str, Any], exc: Exception) -> None:
        segment_id = int(segment.get("id") or 0)
        message = str(exc)
        with translation_state_lock:
            segment["translation"] = make_missing_translation(segment, message)
            state.setdefault("translation_errors", []).append(
                {
                    "id": segment_id,
                    "text": str(segment.get("text", ""))[:240],
                    "error": message[:500],
                    "failed_at": now_stamp(),
                }
            )
            persist_translations()

    def translate_resilient(batch_segments: list[dict[str, Any]]) -> None:
        pending_batches = [batch_segments] if batch_segments else []
        while pending_batches:
            current_batch = pending_batches.pop()
            if not current_batch:
                continue
            batch_marker = f"{threading.get_ident()}:{id(current_batch)}"
            batch_state = {
                "id": batch_marker,
                "start_id": int(current_batch[0].get("id") or 0),
                "end_id": int(current_batch[-1].get("id") or 0),
                "size": len(current_batch),
                "started_at": iso_now(),
            }
            with translation_state_lock:
                running = [item for item in state.get("translation_running_batches", []) if isinstance(item, dict)]
                running.append(batch_state)
                state["translation_running_batches"] = running
                state["updated_at"] = iso_now()
                state["last_update_at"] = state["updated_at"]
                state["last_heartbeat_at"] = state["updated_at"]
                state["worker_pid"] = os.getpid()
                save_task_state_safe(state_path, state)
            try:
                try:
                    results = request_with_retries(current_batch, max_batch_retries)
                    apply_translation_results(results)
                    continue
                except _PartialTranslationError as partial_exc:
                    apply_translation_results(partial_exc.partial_results)
                    failed_segments = [s for s in current_batch if int(s.get("id") or 0) in partial_exc.failed_ids]
                    logger.info("Partial translation: %s ok, retrying %s empty segments individually", len(partial_exc.partial_results), len(failed_segments))
                    for segment in failed_segments:
                        try:
                            results = request_with_retries([segment], max_single_retries)
                            apply_translation_results(results)
                        except _PartialTranslationError as single_partial:
                            if single_partial.partial_results:
                                apply_translation_results(single_partial.partial_results)
                            else:
                                mark_translation_failure(segment, single_partial)
                        except PodcastUpstreamError:
                            raise
                        except Exception as single_exc:
                            logger.warning("Single-segment translation failed; marking missing: %s", single_exc)
                            mark_translation_failure(segment, single_exc)
                    continue
                except Exception as exc:
                    if isinstance(exc, PodcastUpstreamError):
                        raise
                    if len(current_batch) > 1:
                        midpoint = len(current_batch) // 2
                        logger.warning("Translation batch failed; splitting %s segments: %s", len(current_batch), exc)
                        pending_batches.append(current_batch[midpoint:])
                        pending_batches.append(current_batch[:midpoint])
                        continue
                    segment = current_batch[0]
                    try:
                        results = request_with_retries([segment], max_single_retries)
                        apply_translation_results(results)
                    except PodcastUpstreamError:
                        raise
                    except Exception as single_exc:
                        logger.warning("Single-segment translation failed; marking missing: %s", single_exc)
                        mark_translation_failure(segment, single_exc)
            finally:
                with translation_state_lock:
                    running = [
                        item
                        for item in state.get("translation_running_batches", [])
                        if not (isinstance(item, dict) and item.get("id") == batch_marker)
                    ]
                    if running:
                        state["translation_running_batches"] = running
                    else:
                        state.pop("translation_running_batches", None)
                    state["updated_at"] = iso_now()
                    state["last_update_at"] = state["updated_at"]
                    state["last_heartbeat_at"] = state["updated_at"]
                    save_task_state_safe(state_path, state)

    max_batch_workers = max(1, min(6, int(translation_config.get("max_batch_workers", 3))))
    batches = build_translation_batches(pending)
    logger.info(
        "Translating %s segments to %s with %s in %s batches (batch_segments=%s, max_batch_chars=%s, max_batch_workers=%s)",
        len(pending),
        target_language,
        model_name,
        len(batches),
        batch_size,
        batch_char_limit,
        max_batch_workers,
    )
    with ThreadPoolExecutor(max_workers=max_batch_workers) as batch_executor:
        futures = [batch_executor.submit(translate_resilient, batch) for batch in batches]
        for future in as_completed(futures):
            try:
                future.result()
            except PodcastUpstreamError:
                raise
            except Exception as exc:
                logger.error("Batch translation worker encountered an unhandled error: %s", exc)
            
            with translation_state_lock:
                pct = (done_count() / max(1, len(translatable))) * 100
                logger.info(
                    "[translate %6.2f%%] processed %s/%s failed=%s",
                    pct,
                    done_count(),
                    len(translatable),
                    failed_count(),
                )
    state.pop("translation_current_batch", None)
    state.pop("translation_running_batches", None)
    persist_translations()
    return translated


def make_clip_timestamps(duration: float | None, clip_seconds: int = 30) -> list[dict[str, float]]:
    if not duration or duration <= 0:
        return [{"start": 0.0, "end": float(clip_seconds)}]
    # Keep only a tiny decoder-rounding margin; larger fixed cuts lose audible tail
    # content on every chunk.
    effective_duration = max(0.1, float(duration) - 0.02)
    clips: list[dict[str, float]] = []
    start = 0.0
    while start < effective_duration:
        end = min(effective_duration, start + clip_seconds)
        clips.append({"start": round(start, 3), "end": round(end, 3)})
        start = end
    return clips


def find_executable(name: str) -> str | None:
    found = shutil.which(name)
    if found:
        return found

    winget_root = Path(os.environ.get("LOCALAPPDATA", "")) / "Microsoft" / "WinGet" / "Packages"
    if winget_root.exists():
        matches = list(winget_root.rglob(f"{name}.exe"))
        if matches:
            bin_dir = str(matches[0].parent)
            os.environ["PATH"] = bin_dir + os.pathsep + os.environ.get("PATH", "")
            return str(matches[0])
    return None


def ollama_tags_url(config: dict[str, Any]) -> str:
    url = str((config.get("translation") or {}).get("ollama_url", "http://127.0.0.1:11434/api/generate"))
    return url.replace("/generate", "/tags")


def check_ollama_ready(config: dict[str, Any], timeout: int = 5) -> bool:
    try:
        req = urllib.request.Request(ollama_tags_url(config), method="GET")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status == 200
    except urllib.error.URLError as exc:
        log = logging.getLogger(__name__)
        if isinstance(exc.reason, ConnectionRefusedError):
            log.info("Ollama not running (connection refused).")
        elif isinstance(exc.reason, TimeoutError):
            log.info("Ollama not ready (timeout after %ss).", timeout)
        else:
            log.debug("Ollama readiness check failed: %s", exc.reason)
        return False
    except Exception as exc:
        logging.getLogger(__name__).debug("Ollama readiness check failed: %s", exc)
        return False


def maybe_start_ollama(config: dict[str, Any], logger: logging.Logger) -> bool:
    global OLLAMA_PROCESS
    translation = config.get("translation") or {}
    polish = (config.get("markdown") or {}).get("llm_polish") or {}
    translation_backend = provider_name(translation)
    polish_backend = provider_name(polish) if bool(polish.get("enabled", True)) else "none"
    if "ollama" not in {translation_backend, polish_backend}:
        logger.info(
            "Skipping Ollama preflight; effective backends are translation=%s, polish=%s.",
            translation_backend,
            polish_backend,
        )
        return True
    if not bool(translation.get("auto_start_ollama", True)):
        logger.info("Ollama auto-start disabled by config.")
        return False
    if check_ollama_ready(config):
        logger.info("Ollama preflight OK")
        return True

    ollama = find_executable("ollama")
    if not ollama:
        logger.warning("Ollama is not running and ollama.exe was not found in PATH.")
        return False

    logger.info("Ollama is not running; starting it automatically: %s serve", ollama)
    try:
        creationflags = 0
        if os.name == "nt":
            creationflags = subprocess.CREATE_NEW_PROCESS_GROUP
        OLLAMA_PROCESS = subprocess.Popen(
            [ollama, "serve"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            stdin=subprocess.DEVNULL,
            creationflags=creationflags,
            close_fds=True,
        )
        # Persist PID so the GUI can target-kill this specific Ollama later
        ollama_pid_path = STATE_DIR / "ollama.pid"
        try:
            ollama_pid_path.write_text(str(OLLAMA_PROCESS.pid), encoding="utf-8")
        except Exception:
            pass
    except Exception as exc:
        logger.warning("Could not start Ollama automatically: %s", exc)
        return False

    wait_seconds = int(translation.get("ollama_start_timeout_seconds", 20))
    deadline = time.monotonic() + max(1, wait_seconds)
    while time.monotonic() < deadline:
        if check_ollama_ready(config, timeout=2):
            logger.info("Ollama started successfully.")
            return True
        time.sleep(1)

    logger.warning("Ollama did not become ready within %s seconds.", wait_seconds)
    return False


def configure_nvidia_dll_paths(logger: logging.Logger | None = None) -> list[str]:
    site_packages = Path(sys.prefix) / "Lib" / "site-packages"
    candidates = [
        site_packages / "nvidia" / "cuda_runtime" / "bin",
        site_packages / "nvidia" / "cuda_nvrtc" / "bin",
        site_packages / "nvidia" / "cublas" / "bin",
        site_packages / "nvidia" / "cudnn" / "bin",
        site_packages / "ctranslate2",
    ]
    existing = [str(path) for path in candidates if path.exists()]
    if existing:
        os.environ["PATH"] = os.pathsep.join(existing + [os.environ.get("PATH", "")])
        if hasattr(os, "add_dll_directory"):
            for path in existing:
                if path in DLL_DIRECTORY_PATHS:
                    continue
                try:
                    DLL_DIRECTORY_HANDLES.append(os.add_dll_directory(path))
                    DLL_DIRECTORY_PATHS.add(path)
                except OSError as exc:
                    if logger:
                        logger.warning("Could not add DLL directory %s: %s", path, exc)
        if logger:
            logger.info("NVIDIA DLL search paths: %s", existing)
    return existing


def nvidia_dll_candidate_paths() -> list[str]:
    site_packages = Path(sys.prefix) / "Lib" / "site-packages"
    candidates = [
        site_packages / "nvidia" / "cuda_runtime" / "bin",
        site_packages / "nvidia" / "cuda_nvrtc" / "bin",
        site_packages / "nvidia" / "cublas" / "bin",
        site_packages / "nvidia" / "cudnn" / "bin",
        site_packages / "ctranslate2",
    ]
    return [str(path) for path in candidates if path.exists()]


def run_command(command: list[str], logger: logging.Logger, timeout: int | None = None) -> subprocess.CompletedProcess[str]:
    logger.info("Running command: %s", " ".join(command))
    try:
        result = subprocess.run(command, text=True, capture_output=True, encoding="utf-8", errors="replace", timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"Command timed out after {timeout}s: {' '.join(command)}") from exc
    if result.stdout:
        logger.info(result.stdout.strip())
    if result.stderr:
        logger.info(result.stderr.strip())
    if result.returncode != 0:
        stderr_tail = (result.stderr or result.stdout or "").strip()[-1200:]
        raise RuntimeError(
            f"Command failed with exit code {result.returncode}: {' '.join(command)}"
            + (f"\nOutput tail:\n{stderr_tail}" if stderr_tail else "")
        )
    return result


def probe_duration(path: Path, ffprobe: str | None, logger: logging.Logger) -> float | None:
    if ffprobe:
        try:
            result = subprocess.run(
                [
                    ffprobe,
                    "-v",
                    "error",
                    "-show_entries",
                    "format=duration",
                    "-of",
                    "json",
                    str(path),
                ],
                text=True,
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=60,
            )
            if result.returncode == 0:
                payload = json.loads(result.stdout)
                duration = payload.get("format", {}).get("duration")
                if duration is not None:
                    return float(duration)
        except Exception as exc:
            logger.warning("ffprobe duration check failed: %s", exc)

    try:
        import av  # noqa: E402

        with av.open(str(path)) as container:
            if container.duration:
                return float(container.duration) / 1_000_000.0
    except Exception as exc:
        logger.warning("PyAV duration check failed: %s", exc)

    if path.suffix.lower() == ".wav":
        try:
            import wave  # noqa: E402

            with wave.open(str(path), "rb") as stream:
                frame_rate = stream.getframerate()
                if frame_rate > 0:
                    return stream.getnframes() / frame_rate
        except Exception as exc:
            logger.warning("WAV duration fallback failed: %s", exc)
    return None


def file_fingerprint(path: Path) -> str:
    stat = path.stat()
    h = hashlib.sha256()
    h.update(path.name.encode("utf-8", errors="replace"))
    h.update(str(stat.st_size).encode())
    h.update(str(stat.st_mtime_ns).encode())
    with path.open("rb") as f:
        h.update(f.read(1024 * 1024))
        if stat.st_size > 1024 * 1024:
            f.seek(max(0, stat.st_size - 1024 * 1024))
            h.update(f.read(1024 * 1024))
    return h.hexdigest()[:16]


def discover_audio_files() -> list[Path]:
    cfg = load_json(CONFIG_PATH, {})
    supported = {str(ext).lower() for ext in audio_config(cfg).get("supported_extensions", SUPPORTED_EXTENSIONS)}
    supported |= {str(ext).lower() for ext in (cfg.get("video") or {}).get("supported_extensions", VIDEO_EXTENSIONS)}
    files = []
    by_stem: dict[str, list[Path]] = {}
    for path in sorted(INBOX.iterdir()):
        if path.is_file() and path.suffix.lower() in supported:
            files.append(path)
            by_stem.setdefault(path.stem.lower(), []).append(path)
    duplicates = {stem: paths for stem, paths in by_stem.items() if len(paths) > 1}
    if duplicates:
        details = "; ".join(", ".join(p.name for p in paths) for paths in duplicates.values())
        raise RuntimeError(f"Duplicate input basename would overwrite one Markdown output. Rename one file: {details}")
    return files


def normalize_audio(source: Path, task_dir: Path, config: dict[str, Any], ffmpeg: str | None, logger: logging.Logger) -> Path:
    target = task_dir / "source.wav"
    task_dir.mkdir(parents=True, exist_ok=True)
    if target.exists() and target.stat().st_size > 0:
        logger.info("Reusing normalized audio: %s", target)
        return target
    if not ffmpeg:
        raise RuntimeError("ffmpeg is required to normalize input audio.")
    cfg = audio_config(config)
    command = [
        ffmpeg,
        "-y",
        "-hide_banner",
        "-i",
        str(source),
        "-vn",
        "-ac",
        str(int(cfg.get("channels", 1))),
        "-ar",
        str(int(cfg.get("sample_rate", 16000))),
        str(target),
    ]
    run_command(command, logger, timeout=600)
    if not target.exists() or target.stat().st_size == 0:
        raise RuntimeError(
            "ffmpeg did not produce normalized source.wav. The input may have no audio track; "
            "for a video without audio, place a same-stem .srt/.ass subtitle next to it."
        )
    return target


def find_sidecar_subtitle(source: Path, config: dict[str, Any]) -> Path | None:
    """Return a same-stem sidecar subtitle next to ``source``, or None.

    Only an exact stem match is accepted (``talk.mp4`` -> ``talk.srt``) so the
    user stays in control of which subtitle is trusted. Embedded subtitle tracks
    are intentionally ignored — only a manually placed sidecar file counts.
    """
    subs_cfg = config.get("subtitles") or {}
    if not subs_cfg.get("use_sidecar", True):
        return None
    exts = [str(e).lower() for e in subs_cfg.get("sidecar_extensions", SIDECAR_SUBTITLE_EXTENSIONS)]
    stem_lower = source.stem.lower()
    try:
        entries = list(source.parent.iterdir())
    except OSError:
        return None
    candidates = [
        path for path in entries
        if path.is_file() and path.stem.lower() == stem_lower and path.suffix.lower() in exts
    ]
    if not candidates:
        return None
    # Prefer the configured extension order (e.g. .srt before .ass).
    candidates.sort(key=lambda p: exts.index(p.suffix.lower()))
    return candidates[0]


def _decode_subtitle_bytes(raw: bytes) -> str:
    """Decode subtitle bytes tolerantly. Chinese .srt files are often GB18030."""
    for enc in ("utf-8-sig", "utf-8", "gb18030"):
        try:
            return raw.decode(enc)
        except UnicodeDecodeError:
            continue
    return raw.decode("utf-8", errors="replace")


_SRT_TIME_RE = re.compile(
    r"(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})"
)
_SUBTITLE_TAG_RE = re.compile(r"<[^>]+>")
# ASS/SSA inline override blocks (e.g. {\an8}, {\i1}) survive ffmpeg's srt
# conversion as literal braces; strip them so they don't pollute the transcript.
_ASS_OVERRIDE_RE = re.compile(r"\{[^}]*\}")


def _srt_timestamp_to_seconds(h: str, m: str, s: str, ms: str) -> float:
    return int(h) * 3600 + int(m) * 60 + int(s) + int(ms.ljust(3, "0")) / 1000.0


def _parse_srt_text(text: str) -> list[dict[str, Any]]:
    """Parse SRT/VTT cue text into [{start, end, text}] (no ids yet)."""
    segments: list[dict[str, Any]] = []
    cleaned = text.replace("﻿", "").replace("\r\n", "\n").replace("\r", "\n").strip()
    for block in re.split(r"\n\s*\n+", cleaned):
        lines = [ln for ln in block.split("\n") if ln.strip() != ""]
        if not lines:
            continue
        timing_idx = None
        match = None
        for i, line in enumerate(lines):
            match = _SRT_TIME_RE.search(line)
            if match:
                timing_idx = i
                break
        if match is None or timing_idx is None:
            continue  # header (e.g. WEBVTT) or malformed block
        start = _srt_timestamp_to_seconds(match.group(1), match.group(2), match.group(3), match.group(4))
        end = _srt_timestamp_to_seconds(match.group(5), match.group(6), match.group(7), match.group(8))
        cue_text = " ".join(
            _SUBTITLE_TAG_RE.sub("", _ASS_OVERRIDE_RE.sub("", ln)) for ln in lines[timing_idx + 1:]
        )
        cue_text = re.sub(r"\s+", " ", cue_text).strip()
        if cue_text:
            segments.append({"start": round(start, 3), "end": round(end, 3), "text": cue_text})
    return segments


def parse_subtitle_to_segments(sub_path: Path, ffmpeg: str | None, logger: logging.Logger) -> list[dict[str, Any]]:
    """Parse a sidecar subtitle into transcription segments [{id,start,end,text}].

    ``.srt``/``.vtt`` are parsed directly. ``.ass``/``.ssa`` (and anything else)
    are converted to SRT with ffmpeg first, which strips styling/positioning.
    """
    suffix = sub_path.suffix.lower()
    if suffix in (".srt", ".vtt"):
        srt_text = _decode_subtitle_bytes(sub_path.read_bytes())
    else:
        if not ffmpeg:
            raise RuntimeError(f"ffmpeg is required to read {suffix} subtitles.")
        fd, tmp_name = tempfile.mkstemp(suffix=".srt")
        os.close(fd)
        tmp_srt = Path(tmp_name)
        try:
            run_command([ffmpeg, "-y", "-hide_banner", "-i", str(sub_path), str(tmp_srt)], logger, timeout=120)
            srt_text = _decode_subtitle_bytes(tmp_srt.read_bytes())
        finally:
            try:
                tmp_srt.unlink()
            except OSError:
                pass
    segments = _parse_srt_text(srt_text)
    for idx, segment in enumerate(segments, 1):
        segment["id"] = idx
    return segments


def setup_file_logger(name: str) -> logging.Logger:
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    logger.handlers.clear()
    log_path = OUT_LOGS / f"{name}.log"
    handler = logging.FileHandler(log_path, encoding="utf-8")
    handler.setFormatter(logging.Formatter("%(asctime)s [%(levelname)s] %(message)s"))
    logger.addHandler(handler)
    console = logging.StreamHandler(sys.stdout)
    console.setFormatter(logging.Formatter("%(message)s"))
    logger.addHandler(console)
    return logger


def detect_silence_points(
    source: Path,
    ffmpeg: str,
    logger: logging.Logger,
    noise: str = "-35dB",
    min_silence_seconds: float = 0.4,
    timeout: int = 900,
) -> list[tuple[float, float]]:
    """用 ffmpeg silencedetect 扫描静音区间，供切块边界对齐。失败返回空列表（调用方回退固定切块）。"""
    command = [
        ffmpeg,
        "-hide_banner",
        "-nostats",
        "-i",
        str(source),
        "-vn",
        "-af",
        f"silencedetect=noise={noise}:d={min_silence_seconds}",
        "-f",
        "null",
        "-",
    ]
    try:
        result = subprocess.run(command, text=True, capture_output=True, encoding="utf-8", errors="replace", timeout=timeout)
    except Exception as exc:
        logger.warning("silencedetect scan failed (%s); falling back to fixed chunk boundaries.", exc)
        return []
    output = result.stderr or ""
    starts = [float(m) for m in re.findall(r"silence_start:\s*([0-9.]+)", output)]
    ends = [float(m) for m in re.findall(r"silence_end:\s*([0-9.]+)", output)]
    pairs = list(zip(starts, ends, strict=True))
    logger.info("silencedetect found %s silence intervals in %s.", len(pairs), source.name)
    return pairs


def compute_silence_split_points(
    duration: float,
    chunk_seconds: int,
    silences: list[tuple[float, float]],
    search_window: float = 90.0,
) -> list[float]:
    """在每个 chunk_seconds 目标点 ±search_window 内取最近的静音中点作为切点；找不到则该点硬切。

    切点间距保持在 chunk_seconds ± search_window，满足 validate_chunks_metadata
    对「非末块时长 >= 0.5 * chunk_seconds」的校验。
    """
    if not silences or duration <= chunk_seconds * 1.25:
        return []
    midpoints = [(start + end) / 2.0 for start, end in silences]
    points: list[float] = []
    target = float(chunk_seconds)
    while target < duration - chunk_seconds * 0.25:
        candidates = [m for m in midpoints if abs(m - target) <= search_window]
        point = min(candidates, key=lambda m: abs(m - target)) if candidates else target
        if points and point <= points[-1] + 1.0:
            point = target
        points.append(round(point, 3))
        target = points[-1] + chunk_seconds
    return points


def prepare_chunks(
    source: Path,
    task_id: str,
    chunk_seconds: int,
    ffmpeg: str | None,
    logger: logging.Logger,
    ffmpeg_threads: int = 2,
    ffprobe: str | None = None,
    force_reprocess: bool = False,
) -> list[Path]:
    chunk_dir = CHUNKS_DIR / task_id
    if force_reprocess and chunk_dir.exists():
        shutil.rmtree(chunk_dir)
        logger.info("Cleared previous chunks for forced reprocessing: %s", chunk_dir)
    chunk_dir.mkdir(parents=True, exist_ok=True)
    existing = sorted(chunk_dir.glob("chunk_*.wav"))
    if existing:
        if validate_chunks_metadata(chunk_dir, source, chunk_seconds, logger, ffprobe):
            logger.info("Reusing existing validated chunks: %s", len(existing))
            return existing
        logger.warning("Existing chunks failed validation; regenerating.")
        quarantine_chunks(chunk_dir, task_id, logger)
        chunk_dir.mkdir(parents=True, exist_ok=True)

    if not ffmpeg:
        raise RuntimeError("ffmpeg is required for resumable chunk processing but was not found.")

    pattern = chunk_dir / "chunk_%05d.wav"

    # 切块边界优先对齐静音点，避免 30 分钟硬切把句子拦腰斩断
    split_points: list[float] = []
    source_duration = probe_duration(source, ffprobe, logger)
    if source_duration and source_duration > chunk_seconds * 1.25:
        silences = detect_silence_points(source, ffmpeg, logger)
        split_points = compute_silence_split_points(source_duration, chunk_seconds, silences)
        if split_points:
            logger.info(
                "Chunking at %s silence-aligned split points: %s",
                len(split_points),
                ", ".join(format_hms(p) for p in split_points),
            )

    command = [
        ffmpeg,
        "-y",
        "-hide_banner",
        "-threads",
        str(max(1, ffmpeg_threads)),
        "-i",
        str(source),
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-f",
        "segment",
    ]
    if split_points:
        command += ["-segment_times", ",".join(f"{p:.3f}" for p in split_points)]
    else:
        command += ["-segment_time", str(chunk_seconds)]
    command += [
        "-reset_timestamps",
        "1",
        str(pattern),
    ]
    run_command(command, logger, timeout=300)
    chunks = sorted(chunk_dir.glob("chunk_*.wav"))
    if not chunks:
        raise RuntimeError("ffmpeg did not produce audio chunks. The file may not contain an audio stream.")

    meta = {
        "source_path": str(source),
        "source_size": source.stat().st_size,
        "source_mtime": source.stat().st_mtime_ns,
        "source_fingerprint": file_fingerprint(source),
        "chunk_seconds": chunk_seconds,
        "split_points": split_points,
        "ffmpeg_command": command,
        "actual_chunks": len(chunks),
        "created_at": now_stamp(),
        "tool_version": "1.0",
    }
    try:
        (chunk_dir / "chunk_metadata.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")
    except OSError as exc:
        logger.warning("Could not write chunk metadata: %s", exc)

    return chunks


def unique_list(items: list[str]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for item in items:
        if item and item not in seen:
            out.append(item)
            seen.add(item)
    return out


def nvidia_smi_available() -> bool:
    return shutil.which("nvidia-smi") is not None


def is_gpu_runtime_error(exc: BaseException) -> bool:
    text = str(exc).lower()
    markers = [
        "cuda",
        "cublas",
        "cudnn",
        "cufft",
        "curand",
        "cusolver",
        "cusparse",
        "nvrtc",
        "dll is not found",
        "cannot be loaded",
    ]
    return any(marker in text for marker in markers)


def load_whisper_model(config: dict[str, Any], logger: logging.Logger):
    configure_nvidia_dll_paths(logger)
    from faster_whisper import BatchedInferencePipeline, WhisperModel  # noqa: E402

    acfg = asr_config(config)
    models = unique_list([acfg.get("model", "large-v3")] + acfg.get("fallback_models", []))
    preference = str(acfg.get("device_preference", "auto")).lower()
    if preference in {"cuda", "gpu"}:
        devices = ["cuda"]
    elif preference == "cpu":
        devices = ["cpu"]
    else:
        devices = ["cuda", "cpu"] if nvidia_smi_available() else ["cpu"]

    gpu_compute = unique_list(list(acfg.get("compute_type_preference", [])) + ["int8"])
    cpu_compute = unique_list(["int8", "float32"])
    failures: list[str] = []

    for device in devices:
        compute_types = gpu_compute if device == "cuda" else cpu_compute
        for model_name in models:
            for compute_type in compute_types:
                try:
                    logger.info("Trying model=%s device=%s compute_type=%s", model_name, device, compute_type)
                    kwargs: dict[str, Any] = {
                        "device": device,
                        "compute_type": compute_type,
                        "download_root": str(MODELS_DIR),
                    }
                    if device == "cpu":
                        kwargs["cpu_threads"] = int(acfg.get("cpu_threads") or max(1, (os.cpu_count() or 4) - 2))
                    else:
                        kwargs["num_workers"] = int(acfg.get("num_workers", 1))
                    base_model = WhisperModel(model_name, **kwargs)
                    use_batched = device == "cuda" and bool(acfg.get("use_batched_inference", True))
                    model = BatchedInferencePipeline(model=base_model) if use_batched else base_model
                    runtime = {
                        "model": model_name,
                        "device": device,
                        "compute_type": compute_type,
                        "batched": str(use_batched).lower(),
                        "batch_size": str(acfg.get("batch_size", 8) if use_batched else 1),
                    }
                    logger.info("Selected runtime: %s", runtime)
                    return model, runtime, failures
                except Exception as exc:
                    message = f"model={model_name} device={device} compute_type={compute_type}: {exc}"
                    logger.warning(message)
                    failures.append(message)
                    if device == "cuda":
                        continue
    raise RuntimeError("No faster-whisper runtime could be loaded.\n" + "\n".join(failures[-10:]))


def transcribe_chunk(
    model: Any,
    chunk_path: Path,
    offset: float,
    state: dict[str, Any],
    chunk_key: str,
    state_path: Path,
    config: dict[str, Any],
    runtime: dict[str, str],
    duration: float | None,
    logger: logging.Logger,
    _job_id: str | None = None,
) -> list[dict[str, Any]]:
    acfg = asr_config(config)
    language = acfg.get("language", "auto")
    language_arg = None if language in (None, "", "auto") else str(language)
    chunk_duration = probe_duration(chunk_path, None, logger)
    state.setdefault("chunks", {})
    state["chunks"][chunk_key] = {"status": "running", "segments": []}
    update_task_state(
        state_path,
        state,
        status="transcribing",
        stage="transcribing",
        current_chunk=int(chunk_key) + 1,
        _job_id=_job_id,
    )

    transcribe_kwargs: dict[str, Any] = {
        "beam_size": int(acfg.get("beam_size", config.get("beam_size", 5))),
        "language": language_arg,
        "vad_filter": bool(acfg.get("vad_filter", config.get("vad_filter", True))),
        "without_timestamps": False,
    }
    if runtime.get("batched") == "true":
        transcribe_kwargs["batch_size"] = int(acfg.get("batch_size", 8))
        transcribe_kwargs["log_progress"] = True
        if not transcribe_kwargs["vad_filter"]:
            transcribe_kwargs["clip_timestamps"] = make_clip_timestamps(chunk_duration)

    chunk_segments: list[dict[str, Any]] = []
    save_every = max(1, int(config.get("state_save_every_segments", 25)))
    save_interval = max(1.0, float(config.get("state_save_interval_seconds", 5)))
    last_save = time.monotonic()
    last_cancel_check = time.monotonic()
    cancel_check_interval = max(1.0, min(save_interval, 3.0))  # Check at least every 3s
    cancelled = False
    lock_acquired = False
    try:
        while not lock_acquired:
            lock_acquired = MODEL_TRANSCRIBE_LOCK.acquire(timeout=5.0)
            if not lock_acquired:
                touch_task_heartbeat(state_path, state, status="transcribing", stage="transcribe_waiting", _job_id=_job_id)

        with TaskHeartbeat(state_path, state, status="transcribing", stage="transcribing", _job_id=_job_id):
            segments_iter, info = model.transcribe(str(chunk_path), **transcribe_kwargs)
            state["detected_language"] = getattr(info, "language", None)
            state["language_probability"] = getattr(info, "language_probability", None)

            for idx, segment in enumerate(segments_iter, 1):
                item = {
                    "id": len(state.get("segments", [])) + len(chunk_segments) + 1,
                    "start": round(float(segment.start) + offset, 3),
                    "end": round(float(segment.end) + offset, 3),
                    "text": segment.text.strip(),
                }
                for attr in ("avg_logprob", "compression_ratio", "no_speech_prob"):
                    value = getattr(segment, attr, None)
                    if value is not None:
                        item[attr] = value
                chunk_segments.append(item)
                state["chunks"][chunk_key]["segments"] = chunk_segments
                now = time.monotonic()
                # Check for cancellation frequently (every segment or every cancel_check_interval)
                if now - last_cancel_check >= cancel_check_interval or idx % save_every == 0:
                    last_cancel_check = now
                    if _job_cancelled_or_deleted(_job_id):
                        logger.info("Job cancelled during segment loop at segment %d, aborting chunk.", idx)
                        cancelled = True
                        break
                if idx % save_every == 0 or now - last_save >= save_interval:
                    if duration:
                        pct = min(98.0, max(float(state.get("progress_percent") or 8), (item["end"] / duration) * 98.0))
                    else:
                        pct = float(state.get("progress_percent") or 8)
                    update_task_state(
                        state_path,
                        state,
                        status="transcribing",
                        stage="transcribing",
                        progress_percent=pct,
                        current_chunk=int(chunk_key) + 1,
                        _job_id=_job_id,
                    )
                    last_save = now

                if duration:
                    pct = min(100.0, (item["end"] / duration) * 100)
                    logger.info("[%6.2f%%] %s %s", pct, format_hms(item["start"]), item["text"][:90])
                else:
                    logger.info("[segment %s] %s %s", idx, format_hms(item["start"]), item["text"][:90])
    finally:
        if lock_acquired:
            MODEL_TRANSCRIBE_LOCK.release()
            delay = transcribe_lock_yield_seconds(config)
            if delay > 0:
                time.sleep(delay)

    if cancelled:
        state["chunks"][chunk_key] = {"status": "cancelled", "segments": chunk_segments}
        return chunk_segments  # Caller must check _job_cancelled_or_deleted
    state["chunks"][chunk_key] = {"status": "done", "segments": chunk_segments}
    update_task_state(state_path, state, status="transcribing", stage="transcribing", current_chunk=int(chunk_key) + 1, _job_id=_job_id)
    return chunk_segments


def write_outputs(
    source: Path,
    safe_stem: str,
    task_id: str,
    duration: float | None,
    segments: list[dict[str, Any]],
    runtime: dict[str, str],
    started_at: str,
    bilingual: bool = False,
) -> dict[str, str]:
    payload = {
        "task_id": task_id,
        "source_file": source.name,
        "source_path": str(source),
        "duration_seconds": duration,
        "model": runtime.get("model"),
        "device": runtime.get("device"),
        "compute_type": runtime.get("compute_type"),
        "detected_language": runtime.get("detected_language"),
        "language_probability": runtime.get("language_probability"),
        "transcribed_at": now_stamp(),
        "segments": segments,
    }
    from podcast_transcriber.language import assign_language_classes  # noqa: E402

    assign_language_classes(segments, runtime.get("detected_language"))
    payload["segments"] = segments
    file_stem = f"{safe_stem}.{task_id}"
    json_path = OUT_JSON / f"{file_stem}.segments.json"
    save_json(json_path, payload)

    srt_lines: list[str] = []
    for i, segment in enumerate(segments, 1):
        srt_lines.append(str(i))
        srt_lines.append(f"{format_srt_time(segment['start'])} --> {format_srt_time(segment['end'])}")
        srt_lines.append(segment["text"])
        srt_lines.append("")
    srt_path = OUT_SRT / f"{file_stem}.srt"
    srt_path.write_text("\n".join(srt_lines), encoding="utf-8")

    title = source.stem
    md_lines = [
        f"# {title}",
        "",
        "## 音频信息",
        "",
        f"- 原文件：{source.name}",
        f"- 文件路径：{source}",
        f"- 时长：{format_hms(duration)}",
        f"- 模型：{runtime.get('model')}",
        f"- 设备：{runtime.get('device')} / {runtime.get('compute_type')}",
        f"- 转写时间：{started_at} - {now_stamp()}",
        "",
        "## 正文",
        "",
    ]
    for segment in segments:
        md_lines.append(f"### {format_hms(segment['start'])}")
        md_lines.append("")
        md_lines.append(segment["text"])
        md_lines.append("")
    md_path = OUT_MARKDOWN / f"{file_stem}.md"
    md_path.write_text("\n".join(md_lines), encoding="utf-8")

    outputs = {"json": str(json_path), "srt": str(srt_path), "markdown": str(md_path)}

    if bilingual:
        bilingual_lines = [
            f"# {title}",
            "",
            "## 音频信息",
            "",
            f"- 原文件：{source.name}",
            f"- 文件路径：{source}",
            f"- 时长：{format_hms(duration)}",
            f"- 模型：{runtime.get('model')}",
            f"- 设备：{runtime.get('device')} / {runtime.get('compute_type')}",
            f"- 转写时间：{started_at} - {now_stamp()}",
            f"- 翻译：{runtime.get('translation_model', 'ollama')}",
            "",
            "## 正文",
            "",
        ]
        for segment in segments:
            bilingual_lines.append(f"### {format_hms(segment['start'])}")
            bilingual_lines.append("")
            bilingual_lines.append("Original:")
            bilingual_lines.append(segment["text"])
            bilingual_lines.append("")
            bilingual_lines.append("Translation:")
            if segment.get("translation"):
                bilingual_lines.append(segment["translation"])
            else:
                bilingual_lines.append("[TRANSLATION_MISSING]")
            bilingual_lines.append("")
        bilingual_path = OUT_MARKDOWN_BILINGUAL / f"{file_stem}.bilingual.md"
        bilingual_path.write_text("\n".join(bilingual_lines), encoding="utf-8")
        outputs["markdown_bilingual"] = str(bilingual_path)

    return outputs


def write_final_markdown_from_json(json_path: str, logger: logging.Logger, config: dict[str, Any]) -> str | None:
    try:
        import importlib.util  # noqa: E402

        scripts_dir = ROOT / "scripts"
        scripts_dir_text = str(scripts_dir)
        if scripts_dir_text in sys.path:
            sys.path.remove(scripts_dir_text)
        sys.path.insert(0, scripts_dir_text)
        importlib.invalidate_caches()
        cached_pricing = sys.modules.get("deepseek_pricing")
        cached_path = Path(str(getattr(cached_pricing, "__file__", ""))).resolve() if cached_pricing else None
        expected_path = (scripts_dir / "deepseek_pricing.py").resolve()
        if cached_pricing and cached_path == expected_path and not hasattr(cached_pricing, "deepseek_chat_completions_url"):
            sys.modules.pop("deepseek_pricing", None)

        script_path = ROOT / "scripts" / "polish_interview_markdown.py"
        spec = importlib.util.spec_from_file_location("polish_interview_markdown", script_path)
        if spec is None or spec.loader is None:
            raise RuntimeError(f"Could not load {script_path}")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        if hasattr(module, "set_deepseek_semaphore"):
            module.set_deepseek_semaphore(deepseek_api_semaphore(config))

        state_file = Path(json_path)

        def report_polish_progress(done: int, total: int) -> None:
            try:
                state = load_json(state_file) or {}
                state["stage"] = "polishing"
                state["polish_total"] = int(total)
                state["polish_done"] = int(done)
                state["updated_at"] = iso_now()
                state["last_update_at"] = state["updated_at"]
                state["last_heartbeat_at"] = state["updated_at"]
                save_task_state_safe(state_file, state)
            except Exception:
                pass

        if hasattr(module, "set_polish_progress_reporter"):
            module.set_polish_progress_reporter(report_polish_progress)

        out_path = module.process_json(Path(json_path), final_only=True)

        summary = getattr(module, "LAST_POLISH_SUMMARY", None)
        if summary:
            logger.info(
                "润色小结: 总段落 %s | 送润 %s | 批量成功 %s | 串行处理 %s | 覆盖率 %s%%%s",
                summary.get("blocks_total"),
                summary.get("polish_targets"),
                summary.get("polished_batch"),
                summary.get("polished_serial"),
                summary.get("coverage_percent"),
                "（警告：连续错误导致润色中途停用，建议检查 API 后重跑）" if summary.get("disabled_after_errors") else "",
            )
            try:
                state = load_json(state_file) or {}
                state["polish_summary"] = summary
                state["updated_at"] = iso_now()
                save_task_state_safe(state_file, state)
            except Exception:
                pass
        return str(out_path)
    except Exception as exc:
        message = f"Final Markdown generation failed for {json_path}: {exc}"
        logger.exception(message)
        raise RuntimeError(message) from exc


def is_readable_markdown(path_value: str | Path | None) -> bool:
    if not path_value:
        return False
    try:
        path = Path(path_value)
        return path.exists() and path.is_file() and path.suffix.lower() == ".md" and path.stat().st_size > 0
    except OSError:
        return False


def require_final_markdown(outputs: dict[str, str], source_name: str) -> str:
    final_markdown = outputs.get("final_markdown")
    if is_readable_markdown(final_markdown):
        return str(final_markdown)

    expected = OUT_FINAL_MARKDOWN / f"{sanitize_name(Path(source_name).stem)}.md"
    if is_readable_markdown(expected):
        outputs["final_markdown"] = str(expected)
        return str(expected)

    raise RuntimeError(f"Final Markdown was not generated under {OUT_FINAL_MARKDOWN}")


def translate_json_outputs_if_needed(
    json_path: str,
    config: dict[str, Any],
    state: dict[str, Any],
    state_path: Path,
    logger: logging.Logger,
    _job_id: str | None = None,
) -> dict[str, str] | None:
    data = load_json(Path(json_path), None)
    if not data:
        return None

    segments = data.get("segments", [])
    detected_language = data.get("detected_language") or state.get("detected_language") or infer_segments_language(segments)
    if not should_translate_for_language(config, detected_language, segments):
        return None
    strict_output = bool((config.get("translation") or {}).get("strict_output", True))
    if not has_missing_translations(segments) and not (strict_output and has_invalid_translations(segments)):
        return None
    if strict_output:
        for segment in segments:
            if segment.get("translation") and not is_valid_translation_text(str(segment.get("translation", ""))):
                segment.pop("translation", None)

    logger.info("Detected language '%s'; starting automatic translation.", detected_language or "unknown")
    translated_segments = translate_segments_with_llm(segments, config, state, state_path, logger)
    translation_config = config.get("translation") or {}
    runtime = {
        "model": data.get("model", ""),
        "device": data.get("device", ""),
        "compute_type": data.get("compute_type", ""),
        "detected_language": detected_language,
        "language_probability": data.get("language_probability") or state.get("language_probability"),
        "translation_model": model_label(translation_config),
    }
    source_path = Path(data.get("source_path") or data.get("source_file") or Path(json_path).stem)
    safe_stem = sanitize_name(Path(data.get("source_file") or source_path.name).stem)
    return write_outputs(
        source_path,
        safe_stem,
        data.get("task_id") or state.get("task_id") or hashlib.sha256(str(json_path).encode("utf-8")).hexdigest()[:16],
        data.get("duration_seconds"),
        translated_segments,
        runtime,
        data.get("transcribed_at", now_stamp()),
        bilingual=bool((config.get("translation") or {}).get("output_bilingual_markdown", True)),
    )


def is_task_complete(task_id: str, manifest: dict[str, Any], required_outputs: list[str] | None = None) -> bool:
    if manifest.get("processed", {}).get(task_id, {}).get("status") != "success":
        return False
    outputs = manifest.get("processed", {}).get(task_id, {}).get("outputs", {})
    required = required_outputs or ["json"]
    for key in required:
        if key == "final_markdown":
            if not is_readable_markdown(outputs.get(key)):
                return False
            continue
        if not outputs.get(key) or not Path(outputs[key]).exists():
            return False
    return True


def process_file(
    source: Path,
    config: dict[str, Any],
    manifest: dict[str, Any],
    model: Any,
    runtime: dict[str, str],
    ffmpeg: str | None,
    ffprobe: str | None,
    force_reprocess: bool = False,
    _return_context: bool = False,
    _job_id: str | None = None,
) -> dict[str, Any]:
    task_id = file_fingerprint(source)
    safe_stem = sanitize_name(source.stem)
    logger = setup_file_logger(safe_stem)
    task_work_dir = WORK / safe_stem
    state_path = STATE_DIR / f"{task_id}.json"
    started_at = now_stamp()
    log_path = OUT_LOGS / f"{safe_stem}.log"
    state = load_json(
        state_path,
        {
            "task_id": task_id,
            "source_path": str(source),
            "source_file": source.name,
            "created_at": iso_now(),
            "started_at": iso_now(),
            "chunks": {},
            "segments": [],
        },
    )

    # ------------------------------------------------------------------
    # Recovery action handling: skip stages based on recovery_action
    # ------------------------------------------------------------------
    recovery_action = str(state.get("recovery_action") or "").strip()
    if recovery_action == "write_markdown_only":
        # Skip transcription and translation; generate Markdown from existing data
        logger.info("Recovery action: write_markdown_only — skipping to output stage for %s", source.name)
        state.pop("recovery_action", None)
        update_task_state(
            state_path, state,
            status="postprocess_queued", stage="postprocess_queued",
            progress_percent=98,
            log_path=str(log_path),
            can_resume=False, can_retry=True,
            _job_id=_job_id,
        )
        # Load existing segments from state or JSON output
        all_segments = state.get("segments") or []
        if not all_segments:
            json_path = OUT_JSON / f"{safe_stem}.{task_id}.segments.json"
            if json_path.exists():
                data = load_json(json_path, {})
                all_segments = data.get("segments") or []
                state["segments"] = all_segments
        detected_language = state.get("detected_language") or infer_segments_language(all_segments)
        needs_translation = should_translate_for_language(config, detected_language, all_segments)
        runtime.setdefault("detected_language", detected_language or "")
        runtime.setdefault("language_probability", state.get("language_probability"))
        context = {
            "_type": "audio_context",
            "source": source,
            "config": config,
            "task_id": task_id,
            "safe_stem": safe_stem,
            "state_path": state_path,
            "state": state,
            "all_segments": all_segments,
            "runtime": runtime,
            "duration": state.get("duration_seconds") or probe_duration(source, ffprobe, logger) or 0,
            "started_at": started_at,
            "detected_language": detected_language,
            "needs_translation": needs_translation,
        }
        result, failures = process_postprocess_stage(context, config, _job_id=_job_id)
        return result

    if recovery_action == "resume_translation":
        # Skip transcription; use existing segments for translation + output
        logger.info("Recovery action: resume_translation — skipping to translation stage for %s", source.name)
        state.pop("recovery_action", None)
        update_task_state(
            state_path, state,
            status="translating", stage="translating",
            progress_percent=98,
            log_path=str(log_path),
            can_resume=False, can_retry=True,
            _job_id=_job_id,
        )
        # Load existing segments from state or JSON output
        all_segments = state.get("segments") or []
        if not all_segments:
            json_path = OUT_JSON / f"{safe_stem}.{task_id}.segments.json"
            if json_path.exists():
                data = load_json(json_path, {})
                all_segments = data.get("segments") or []
                state["segments"] = all_segments
        detected_language = state.get("detected_language") or infer_segments_language(all_segments)
        runtime.setdefault("detected_language", detected_language or "")
        runtime.setdefault("language_probability", state.get("language_probability"))
        needs_translation = should_translate_for_language(config, detected_language, all_segments)
        context = {
            "_type": "audio_context",
            "source": source,
            "config": config,
            "task_id": task_id,
            "safe_stem": safe_stem,
            "state_path": state_path,
            "state": state,
            "all_segments": all_segments,
            "runtime": runtime,
            "duration": state.get("duration_seconds") or probe_duration(source, ffprobe, logger) or 0,
            "started_at": started_at,
            "detected_language": detected_language,
            "needs_translation": needs_translation,
        }
        result, failures = process_postprocess_stage(context, config, _job_id=_job_id)
        return result

    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before preparing.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    _transition_job_db(_job_id, "preparing", "preparing", progress_percent=0)
    update_task_state(
        state_path,
        state,
        status="preparing",
        stage="preparing",
        progress_percent=0,
        log_path=str(log_path),
        can_resume=False,
        can_retry=True,
        _job_id=_job_id,
    )

    if not is_file_stable(source, wait=2.0):
        logger.warning("Input file appears unstable (still changing); skipping: %s", source.name)
        update_task_state(
            state_path,
            state,
            status="failed",
            stage="preparing",
            error_message="input file unstable",
            error_type="InputFileUnstable",
            can_retry=True,
            heartbeat=False,
            _job_id=_job_id,
        )
        _transition_job_db(_job_id, "failed", "preparing", error_message="input file unstable", event_type="job_failed")
        return {"file": source.name, "status": "failed", "task_id": task_id, "error": "input file unstable"}

    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before normalizing.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    # ------------------------------------------------------------------
    # Sidecar subtitle shortcut: if a same-stem .srt/.ass sits next to the
    # input, use it as the transcript source and skip ASR entirely. The parsed
    # segments feed the exact same postprocess stage (translate -> polish ->
    # final Markdown) as Whisper output, so output conventions are unchanged.
    # Falls back to normal audio transcription if parsing fails.
    # ------------------------------------------------------------------
    sidecar = find_sidecar_subtitle(source, config)
    if sidecar is not None:
        logger.info("Using sidecar subtitle as transcript source (skipping ASR): %s", sidecar.name)
        # Mirror the normal ASR path's forced-reprocess clearing: drop a stale
        # terminal state file so state/GUI updates below are not blocked.
        if force_reprocess and state_path.exists():
            try:
                state_path.unlink()
                logger.info("Cleared previous state for forced reprocessing: %s", state_path)
            except OSError as exc:
                logger.warning("Could not clear previous state: %s", exc)
            state["chunks"] = {}
            state["segments"] = []
            state.pop("terminal_by", None)
            state.pop("outputs", None)
        update_task_state(state_path, state, status="preparing", stage="parsing_subtitle",
                          progress_percent=5, log_path=str(log_path), _job_id=_job_id)
        try:
            subtitle_segments = parse_subtitle_to_segments(sidecar, ffmpeg, logger)
        except Exception as exc:
            logger.warning("Failed to parse sidecar subtitle %s (%s); falling back to audio transcription.",
                           sidecar.name, exc)
            subtitle_segments = []
        if subtitle_segments:
            duration = (subtitle_segments[-1].get("end") or 0) or probe_duration(source, ffprobe, logger) or 0
            detected_language = state.get("detected_language") or infer_segments_language(subtitle_segments)
            state["segments"] = subtitle_segments
            state["duration_seconds"] = duration
            state["detected_language"] = detected_language
            state["subtitle_source"] = sidecar.name
            runtime["detected_language"] = detected_language or ""
            runtime["language_probability"] = state.get("language_probability")
            runtime["model"] = f"subtitle:{sidecar.name}"
            runtime["device"] = "subtitle"
            runtime["compute_type"] = "none"
            context = {
                "_type": "audio_context",
                "source": source,
                "config": config,
                "task_id": task_id,
                "safe_stem": safe_stem,
                "state_path": state_path,
                "state": state,
                "all_segments": subtitle_segments,
                "runtime": runtime,
                "duration": duration,
                "started_at": started_at,
                "detected_language": detected_language,
                "needs_translation": should_translate_for_language(config, detected_language, subtitle_segments),
            }
            if _return_context:
                update_task_state(state_path, state, status="postprocess_queued", stage="postprocess_queued",
                                  progress_percent=98, _job_id=_job_id)
                save_task_state_safe(state_path, state)
                return context
            save_task_state_safe(state_path, state)
            result, _ = process_postprocess_stage(context, config, _job_id=_job_id)
            return result

    _transition_job_db(_job_id, "normalizing", "normalizing", progress_percent=1)
    update_task_state(state_path, state, status="normalizing", stage="normalizing", progress_percent=1, _job_id=_job_id)
    try:
        normalized_source = normalize_audio(source, task_work_dir, config, ffmpeg, logger)
    except Exception as exc:
        mark_task_failed(state_path, state, exc, stage="normalizing", logger=logger, _job_id=_job_id)
        _transition_job_db(_job_id, "failed", "normalizing", error_message=str(exc), event_type="job_failed")
        raise
    state["normalized_source"] = str(normalized_source)
    update_task_state(state_path, state, status="normalizing", stage="normalizing", progress_percent=3, _job_id=_job_id)

    required = ["json", "final_markdown"]
    manifest = load_manifest()
    if not force_reprocess and config.get("skip_processed_files", True) and is_task_complete(task_id, manifest, required):
        logger.info("Skipping already processed file: %s", source.name)
        outputs = dict(manifest.get("processed", {}).get(task_id, {}).get("outputs", {}))
        if outputs.get("json") and Path(outputs["json"]).exists():
            try:
                translated_outputs = translate_json_outputs_if_needed(outputs["json"], config, load_json(state_path, {}), state_path, logger, _job_id=_job_id)
                if translated_outputs:
                    outputs.update(translated_outputs)
                    trans_state = load_json(state_path, {})
                    translation_status = "partial_success" if int(trans_state.get("translation_failed") or 0) else "success"
                    updates = {"outputs": outputs, "translation_status": translation_status}
                    if translation_status == "partial_success":
                        updates["status"] = "partial_success"
                    update_manifest_entry(task_id, updates)
            except Exception as exc:
                logger.warning("Automatic translation skipped after failure: %s", exc)
                update_manifest_entry(task_id, {"translation_status": "failed"})
            final_markdown = write_final_markdown_from_json(outputs["json"], logger, config)
            if final_markdown:
                outputs["final_markdown"] = final_markdown
            require_final_markdown(outputs, source.name)
            update_manifest_entry(task_id, {"outputs": outputs})
            state["outputs"] = outputs
            update_task_state(state_path, state, status="success", stage="completed", progress_percent=100, heartbeat=False, _job_id=_job_id)
        return {"file": source.name, "status": "skipped", "task_id": task_id, "outputs": outputs}

    logger.info("Processing %s", source)
    duration = probe_duration(normalized_source, ffprobe, logger)
    if force_reprocess and state_path.exists():
        try:
            state_path.unlink()
            logger.info("Cleared previous state for forced reprocessing: %s", state_path)
        except OSError as exc:
            logger.warning("Could not clear previous state: %s", exc)
        state["chunks"] = {}
        state["segments"] = []
        state.pop("translation_cache", None)
        state.pop("translation_errors", None)
        state.pop("translation_current_batch", None)
        state.pop("translation_running_batches", None)
        state.pop("outputs", None)
    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before chunking.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    chunk_seconds = int(asr_config(config).get("chunk_seconds", config.get("chunk_seconds", 1800)))
    state["duration_seconds"] = duration
    _transition_job_db(_job_id, "chunking", "chunking", progress_percent=5)
    update_task_state(state_path, state, status="chunking", stage="chunking", progress_percent=5, _job_id=_job_id)
    try:
        chunks = prepare_chunks(
            normalized_source,
            task_id,
            chunk_seconds,
            ffmpeg,
            logger,
            int(config.get("ffmpeg_threads", 2)),
            ffprobe,
            force_reprocess=force_reprocess,
        )
    except Exception as exc:
        mark_task_failed(state_path, state, exc, stage="chunking", logger=logger, _job_id=_job_id)
        _transition_job_db(_job_id, "failed", "chunking", error_message=str(exc), event_type="job_failed")
        raise
    update_task_state(
        state_path,
        state,
        status="chunking",
        stage="chunking",
        progress_percent=8,
        current_chunk=0,
        total_chunks=len(chunks),
        _job_id=_job_id,
    )
    all_segments: list[dict[str, Any]] = []
    _transition_job_db(_job_id, "transcribing", "transcribing",
                       progress_percent=8, current_chunk=0, total_chunks=len(chunks))

    for index, chunk_path in enumerate(chunks):
        if _job_cancelled_or_deleted(_job_id):
            logger.info("Job %s was cancelled or deleted by user; aborting during chunk %s.", _job_id, index)
            return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

        chunk_key = f"{index:05d}"
        chunk_state = state.get("chunks", {}).get(chunk_key, {})
        if chunk_state.get("status") == "done":
            logger.info("Skipping completed chunk %s", chunk_key)
            all_segments.extend(chunk_state.get("segments", []))
            continue

        offset = index * chunk_seconds
        logger.info("Transcribing chunk %s/%s: %s", index + 1, len(chunks), chunk_path.name)
        update_task_state(
            state_path,
            state,
            status="transcribing",
            stage="transcribing",
            progress_percent=float(state.get("progress_percent") or 8),
            current_chunk=index + 1,
            total_chunks=len(chunks),
            _job_id=_job_id,
        )
        chunk_segments = transcribe_chunk(
            model,
            chunk_path,
            offset,
            state,
            chunk_key,
            state_path,
            config,
            runtime,
            duration,
            logger,
            _job_id=_job_id,
        )
        # If job was cancelled during chunk transcription, abort the chunk loop
        if _job_cancelled_or_deleted(_job_id):
            logger.info("Job cancelled after chunk %s, aborting transcription loop.", chunk_key)
            break
        all_segments.extend(chunk_segments)
        state["segments"] = all_segments
        chunk_progress = ((index + 1) / max(1, len(chunks))) * 90.0
        update_task_state(
            state_path,
            state,
            status="transcribing",
            stage="transcribing",
            progress_percent=min(98.0, 8.0 + chunk_progress),
            current_chunk=index + 1,
            total_chunks=len(chunks),
            _job_id=_job_id,
        )

    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job cancelled after transcription, skipping postprocess.")
        return {"status": "cancelled"}

    all_segments = sorted(all_segments, key=lambda item: (item["start"], item["end"]))
    for idx, segment in enumerate(all_segments, 1):
        segment["id"] = idx

    detected_language = state.get("detected_language") or infer_segments_language(all_segments)
    runtime["detected_language"] = detected_language or ""
    runtime["language_probability"] = state.get("language_probability")

    if _return_context:
        needs_translation = should_translate_for_language(config, detected_language, all_segments)
        next_status = "postprocess_queued"
        next_stage = "postprocess_queued"
        _transition_job_db(_job_id, next_status, next_stage, progress_percent=98)
        update_task_state(
            state_path, state,
            status=next_status, stage=next_stage,
            progress_percent=98,
            _job_id=_job_id,
        )
        state["translation_status"] = "queued"
        state["translation_done"] = 0
        state["translation_total"] = 0
        save_task_state_safe(state_path, state)
        return {
            "_type": "audio_context",
            "source": source,
            "config": config,
            "task_id": task_id,
            "safe_stem": safe_stem,
            "state_path": state_path,
            "state": state,
            "all_segments": all_segments,
            "runtime": runtime,
            "duration": duration,
            "started_at": started_at,
            "detected_language": detected_language,
            "needs_translation": needs_translation,
        }

    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before postprocess.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    # Build context for postprocess stage and delegate
    context = {
        "_type": "audio_context",
        "source": source,
        "config": config,
        "task_id": task_id,
        "safe_stem": safe_stem,
        "state_path": state_path,
        "state": state,
        "all_segments": all_segments,
        "runtime": runtime,
        "duration": duration,
        "started_at": started_at,
        "detected_language": detected_language,
        "needs_translation": should_translate_for_language(config, detected_language, all_segments),
    }
    result, _ = process_postprocess_stage(context, config)
    return result


def process_postprocess_stage(
    context: dict,
    config: dict,
    run_logger: logging.Logger | None = None,
    _job_id: str | None = None,
) -> tuple[dict[str, Any], list[str]]:
    """Run postprocess for a completed audio transcription: translate, write output, finalize.

    Called either from the postprocess executor (parallel mode) or inline
    from process_file() (sequential mode).  # noqa: E402

    Args:
        context: AudioContext dict from process_file() with _return_context=True.
        config: Full config dict.
        run_logger: Optional run-level logger for error reporting.

    Returns:
        (result_dict, local_failures) tuple, same shape as process_source_with_fallback.
    """
    source: Path = context["source"]
    task_id: str = context["task_id"]
    safe_stem: str = context["safe_stem"]
    state_path: Path = context["state_path"]
    # Reload state fresh — the audio stage may have continued writing after context capture
    state: dict = load_json(state_path, context["state"])
    all_segments: list[dict] = context["all_segments"]
    runtime: dict = context["runtime"]
    duration: float = context["duration"]
    started_at: str = context["started_at"]
    detected_language: str | None = context.get("detected_language")
    needs_translation: bool = context.get("needs_translation", True)
    logger = setup_file_logger(safe_stem)

    try:
        return _run_postprocess_body(source, task_id, safe_stem, state_path, state,
                                     all_segments, runtime, duration, started_at,
                                     detected_language, needs_translation, config, logger, run_logger,
                                     _job_id)
    except PodcastUpstreamError:
        raise
    except Exception as exc:
        logger.exception("Unexpected postprocess failure for %s", source.name)
        state["status"] = "failed"
        state["stage"] = "failed"
        state["error"] = str(exc)
        save_task_state_safe(state_path, state)
        _transition_job_db(_job_id, "failed", "failed", error_message=str(exc), event_type="postprocess_failed")
        manifest_entry = {
            "status": "failed",
            "source_path": str(source),
            "source_file": source.name,
            "completed_at": now_stamp(),
            "error": str(exc),
        }
        update_manifest_processed(task_id, manifest_entry)
        return {"file": source.name, "status": "failed", "task_id": task_id, "error": str(exc)}, []


def _run_postprocess_body(
    source, task_id, safe_stem, state_path, state,
    all_segments, runtime, duration, started_at,
    detected_language, needs_translation, config, logger, run_logger,
    _job_id=None,
):
    """Internal: core postprocess logic wrapped by process_postprocess_stage for error safety."""
    translation_status: str = "skipped"
    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before postprocess.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    if needs_translation:
        translation_config = config.get("translation") or {}
        try:
            if _job_cancelled_or_deleted(_job_id):
                logger.info("Job %s was cancelled or deleted by user; aborting before translation.", _job_id)
                return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}
            logger.info("Detected language '%s'; starting automatic translation.", detected_language or "unknown")
            _transition_job_db(_job_id, "translating", "translating", progress_percent=98)
            update_task_state(state_path, state, status="translating", stage="translating", progress_percent=98, _job_id=_job_id)
            state["translation_status"] = "running"
            save_task_state_safe(state_path, state)
            with translation_semaphore(config):
                all_segments = translate_segments_with_llm(all_segments, config, state, state_path, logger, _job_id=_job_id)
            runtime["translation_model"] = model_label(translation_config)
            translation_status = "partial_success" if int(state.get("translation_failed") or 0) else "success"
        except PodcastUpstreamError:
            raise
        except Exception as exc:
            logger.error("Translation failed: %s", exc)
            state["translation_error"] = str(exc)
            translation_status = "failed"
            _transition_job_db(_job_id, "translating", "translating",
                                error_message=str(exc), error_type=type(exc).__name__)
            update_task_state(
                state_path,
                state,
                status="translating",
                stage="translating",
                error_message=str(exc),
                error_type=type(exc).__name__,
                _job_id=_job_id,
            )

    if _job_cancelled_or_deleted(_job_id):
        logger.info("Job %s was cancelled or deleted by user; aborting before writing output.", _job_id)
        return {"file": source.name, "status": "cancelled", "task_id": task_id, "error": "cancelled by user"}

    _transition_job_db(_job_id, "writing_output", "writing_output", progress_percent=99)
    update_task_state(state_path, state, status="writing_output", stage="writing_output", progress_percent=99, _job_id=_job_id)
    outputs = write_outputs(
        source,
        safe_stem,
        task_id,
        duration,
        all_segments,
        runtime,
        started_at,
        bilingual=bool((config.get("translation") or {}).get("output_bilingual_markdown", True)),
    )
    final_markdown = write_final_markdown_from_json(outputs["json"], logger, config)
    if final_markdown:
        outputs["final_markdown"] = final_markdown
    try:
        require_final_markdown(outputs, source.name)
    except RuntimeError as exc:
        state["status"] = "failed"
        state["stage"] = "failed"
        state["translation_status"] = translation_status
        state["error"] = str(exc)
        state["segments"] = all_segments
        state["outputs"] = outputs
        state["completed_at"] = now_stamp()
        state["last_update_at"] = now_stamp()
        save_task_state_safe(state_path, state)
        _transition_job_db(_job_id, "failed", "failed",
                           error_message=str(exc), event_type="job_failed")
        manifest_entry = {
            "status": "failed",
            "source_path": str(source),
            "source_file": source.name,
            "completed_at": now_stamp(),
            "outputs": outputs,
            "translation_status": translation_status,
            "error": str(exc),
        }
        update_manifest_processed(task_id, manifest_entry)
        logger.error("%s", exc)
        return {"file": source.name, "status": "failed", "task_id": task_id, "outputs": outputs, "error": str(exc)}, []

    overall_status = "success"
    if translation_status in {"failed", "partial_success"}:
        overall_status = "partial_success"

    state["status"] = overall_status
    state["stage"] = "completed"
    state["progress_percent"] = 100
    state["translation_status"] = translation_status
    state["segments"] = all_segments
    state["outputs"] = outputs
    state["completed_at"] = now_stamp()
    _transition_job_db(_job_id, "completed", "completed", progress_percent=100, event_type="job_completed")
    update_task_state(state_path, state, status=overall_status, stage="completed", progress_percent=100, heartbeat=False, _job_id=_job_id)

    manifest_entry = {
        "status": overall_status,
        "source_path": str(source),
        "source_file": source.name,
        "completed_at": now_stamp(),
        "outputs": outputs,
        "translation_status": translation_status,
    }
    update_manifest_processed(task_id, manifest_entry)
    logger.info("Completed: %s (status=%s, translation=%s)", source.name, overall_status, translation_status)
    return {"file": source.name, "status": overall_status, "task_id": task_id, "outputs": outputs, "translation_status": translation_status}, []


def translate_existing_outputs(config: dict[str, Any], force: bool = False, no_open_output: bool = False) -> int:
    json_files = sorted(OUT_JSON.glob("*.segments.json"))
    if not json_files:
        print(f"No segments JSON files found in {OUT_JSON}")
        return 0

    results: list[dict[str, Any]] = []
    for json_path in json_files:
        data = load_json(json_path, None)
        if not data:
            results.append({"file": json_path.name, "status": "failed", "error": "invalid json"})
            continue

        task_id = data.get("task_id") or hashlib.sha256(str(json_path).encode("utf-8")).hexdigest()[:16]
        source_file = data.get("source_file") or json_path.name.replace(".segments.json", "")
        source_path = Path(data.get("source_path") or source_file)
        safe_stem = sanitize_name(Path(source_file).stem)
        state_path = STATE_DIR / f"{task_id}.translation.json"
        state = load_json(state_path, {"task_id": task_id, "source_file": source_file, "translation_cache": []})
        logger = setup_file_logger(f"{safe_stem}.translation")

        if not force:
            existing_bilingual = OUT_MARKDOWN_BILINGUAL / f"{safe_stem}.{task_id}.bilingual.md"
            if existing_bilingual.exists():
                logger.info("Skipping %s: bilingual output already exists.", source_file)
                results.append({"file": source_file, "status": "skipped", "task_id": task_id, "error": "bilingual output already exists"})
                continue

        try:
            segments = data.get("segments", [])
            detected_language = data.get("detected_language") or state.get("detected_language") or infer_segments_language(segments)
            if not should_translate_for_language(config, detected_language, segments):
                results.append({"file": source_file, "status": "skipped", "task_id": task_id, "error": f"language {detected_language or 'unknown'} is not configured for translation"})
                continue
            if bool((config.get("translation") or {}).get("strict_output", True)):
                for segment in segments:
                    if segment.get("translation") and not is_valid_translation_text(str(segment.get("translation", ""))):
                        segment.pop("translation", None)
            with translation_semaphore(config):
                translated_segments = translate_segments_with_llm(segments, config, state, state_path, logger, _job_id=None)
            runtime = {
                "model": data.get("model", ""),
                "device": data.get("device", ""),
                "compute_type": data.get("compute_type", ""),
                "detected_language": detected_language,
                "language_probability": data.get("language_probability") or state.get("language_probability"),
                "translation_model": model_label(config.get("translation") or {}),
            }
            outputs = write_outputs(
                source_path,
                safe_stem,
                task_id,
                data.get("duration_seconds"),
                translated_segments,
                runtime,
                data.get("transcribed_at", now_stamp()),
                bilingual=True,
            )
            final_markdown = write_final_markdown_from_json(outputs["json"], logger, config)
            if final_markdown:
                outputs["final_markdown"] = final_markdown
            require_final_markdown(outputs, source_file)
            translation_status = "partial_success" if int(state.get("translation_failed") or 0) else "success"
            overall_status = "partial_success" if translation_status == "partial_success" else "success"
            update_manifest_processed(task_id, {
                "status": overall_status,
                "source_path": str(source_path),
                "source_file": source_file,
                "completed_at": now_stamp(),
                "outputs": outputs,
                "translation_status": translation_status,
            })
            results.append({"file": source_file, "status": overall_status, "task_id": task_id, "outputs": outputs, "translation_status": translation_status})
        except Exception as exc:
            logger.exception("Translation failed for %s", source_file)
            failure_status = "failed" if "Final Markdown was not generated" in str(exc) else "partial_success"
            # Update manifest to reflect translation failure
            existing = load_manifest().get("processed", {}).get(task_id, {})
            if failure_status == "failed":
                update_manifest_processed(task_id, {
                    **existing,
                    "status": "failed",
                    "source_path": str(source_path),
                    "source_file": source_file,
                    "completed_at": now_stamp(),
                    "translation_status": "failed",
                    "error": str(exc),
                })
            elif existing.get("status") == "success":
                update_manifest_entry(task_id, {"translation_status": "failed", "status": "partial_success"})
            results.append({"file": source_file, "status": failure_status, "task_id": task_id, "error": str(exc), "translation_status": "failed"})

    summary = write_run_summary(results, None, [])
    print(f"Translation summary: {summary}")
    open_output_folder(config, no_open_output=no_open_output)
    return 0 if not any(item["status"] in ("failed", "partial_success") for item in results) else 1


def write_run_summary(results: list[dict[str, Any]], runtime: dict[str, str] | None, failures: list[str]) -> Path:
    success = sum(1 for item in results if item["status"] == "success")
    skipped = sum(1 for item in results if item["status"] == "skipped")
    failed = sum(1 for item in results if item["status"] == "failed")
    trans_success = sum(1 for item in results if item.get("translation_status") == "success")
    trans_partial = sum(1 for item in results if item.get("translation_status") == "partial_success")
    trans_failed = sum(1 for item in results if item.get("translation_status") == "failed")
    path = OUT_REPORTS / "run_summary.md"
    lines = [
        "# 本轮转写摘要",
        "",
        f"- 运行时间：{now_stamp()}",
        f"- 转写成功（无翻译请求）：{success - trans_success}",
        f"- 转写成功且翻译成功：{trans_success}",
        f"- 转写成功但部分翻译缺失：{trans_partial}",
        f"- 转写成功但翻译失败：{trans_failed}",
        f"- 跳过：{skipped}",
        f"- 失败：{failed}",
        "",
        "## 各阶段状态",
        "",
    ]
    for item in results:
        t_status = item.get("translation_status", "skipped")
        # Categorize status label for clarity
        if item["status"] == "partial_success":
            status_label = "转录成功，部分翻译缺失" if t_status == "partial_success" else "转录成功，翻译失败"
        elif t_status == "success" and item["status"] == "success":
            status_label = "转录+翻译均成功"
        elif item["status"] == "success":
            status_label = "转写成功（未请求翻译）"
        elif item["status"] == "skipped":
            status_label = "跳过（已处理）"
        elif item["status"] == "failed":
            status_label = "失败"
        else:
            status_label = item["status"]
        lines.append(f"- {item['file']}: {status_label}")
    if runtime:
        lines.append("")
        lines.append(f"- 实际运行：{runtime.get('model')} / {runtime.get('device')} / {runtime.get('compute_type')}")
    lines.append("")
    lines.append("## 文件结果")
    lines.append("")
    if results:
        for item in results:
            line = f"- {item['status']}: {item['file']} ({item.get('task_id', '-')})"
            if item.get("error"):
                line += f" - {item['error']}"
            if item.get("fallback_from"):
                line += f" - fallback from {item['fallback_from']}"
            lines.append(line)
    else:
        lines.append("- inbox 中没有待处理音频。")
    if failures:
        lines.extend(["", "## 模型或 GPU 尝试失败记录", ""])
        for failure in failures[-20:]:
            lines.append(f"- {failure}")
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def dry_run(config: dict[str, Any]) -> int:
    files = discover_audio_files()
    print("PodcastTranscriber dry-run")
    print(f"Workspace: {ROOT}")
    print(f"Input: {INBOX}")
    print(f"Final markdown output: {OUT_FINAL_MARKDOWN}")
    print(f"Internal artifacts: {INTERNAL}")
    print(f"Config model: {asr_config(config).get('model')}")
    print(f"Parallel audio files: {max_parallel_audio_files(config)}")
    print(f"Parallel translations: {max_parallel_translations(config)}")
    print(f"Audio files found: {len(files)}")
    for path in files:
        print(f" - {path.name}")
    ffmpeg = find_executable("ffmpeg")
    ffprobe = find_executable("ffprobe")
    print(f"ffmpeg: {ffmpeg or 'not found'}")
    print(f"ffprobe: {ffprobe or 'not found'}")
    print("faster-whisper environment: checked at transcription time")
    print("NVIDIA DLL dirs:")
    for path in nvidia_dll_candidate_paths():
        print(f" - {path}")
    return 0


def preflight_checks(config: dict[str, Any], logger: logging.Logger) -> bool:
    try:
        free = shutil.disk_usage(str(ROOT)).free
        min_free = 1 * 1024 * 1024 * 1024
        if free < min_free:
            logger.warning("Disk space low: %s GB available (recommended >=1 GB)", free / (1024**3))
        else:
            logger.info("Disk space OK: %s GB available", free / (1024**3))
    except Exception as exc:
        logger.warning("Could not check disk space: %s", exc)

    if is_translation_enabled(config) or bool(((config.get("markdown") or {}).get("llm_polish") or {}).get("enabled", True)):
        try:
            if not maybe_start_ollama(config, logger):
                logger.warning("Ollama preflight failed (Ollama-backed translation or polish may not work).")
        except RuntimeError as exc:
            # DeepSeek backend raises RuntimeError from effective_provider_name
            # when API key is not yet validated. This is not fatal at preflight —
            # actual validation happens at translation time. Log and continue.
            logger.warning("Backend preflight note: %s", exc)
    return True


def _transition_job_db(job_id: str | None, status: str, stage: str, **kwargs: Any) -> None:
    """No-op stub — SQLite job store removed in v2 refactor."""
    pass


def _job_cancelled_or_deleted(job_id: str | None) -> bool:
    """No-op stub — always returns False (no DB to check)."""
    return False


def process_source_with_fallback(
    source: Path,
    config: dict[str, Any],
    manifest: dict,
    model: Any,
    runtime: dict[str, str],
    ffmpeg: str | None,
    ffprobe: str | None,
    force_reprocess: bool,
    run_logger: logging.Logger | None,
    _audio_only: bool = False,
    _job_id: str | None = None,
) -> dict[str, Any]:
    local_failures: list[str] = []
    task_id = file_fingerprint(source)
    safe_stem = sanitize_name(source.stem)
    logger = setup_file_logger(safe_stem)
    state_path = STATE_DIR / f"{task_id}.json"
    log_path = OUT_LOGS / f"{safe_stem}.log"
    # Write log_path into state early so the GUI can read per-task logs
    # even before process_file() sets it during the preparing stage.
    try:
        early_state = load_json(state_path, {})
        if not early_state.get("log_path"):
            early_state["log_path"] = str(log_path)
            early_state.setdefault("task_id", task_id)
            early_state.setdefault("source_file", source.name)
            early_state.setdefault("source_path", str(source))
            early_state.setdefault("status", "preparing")
            save_json(state_path, early_state)
    except Exception:
        pass
    try:
        state = load_json(
            state_path,
            {
                "task_id": task_id,
                "source_path": str(source),
                "source_file": source.name,
                "created_at": iso_now(),
                "started_at": iso_now(),
                "chunks": {},
                "segments": [],
            },
        )
    except Exception as exc:
        name = sanitize_name(source.stem)
        logger = setup_file_logger(name)
        logger.exception("Failed to process %s", source.name)
        task_id = file_fingerprint(source)
        state_path = STATE_DIR / f"{task_id}.json"
        state = load_json(state_path, {"task_id": task_id, "source_path": str(source), "source_file": source.name})
        if state.get("status") not in TERMINAL_TASK_STATUSES:
            mark_task_failed(state_path, state, exc, stage=str(state.get("stage") or "failed"), logger=logger, _job_id=_job_id)
        _transition_job_db(_job_id, "failed", str(state.get("stage") or "failed"),
                           error_message=str(exc), event_type="job_failed")

        if runtime.get("device") == "cuda" and is_gpu_runtime_error(exc):
            failure = f"CUDA runtime failed during transcription; switching this file to CPU. Error: {exc}"
            run_logger.warning(failure)
            local_failures.append(failure)
            cpu_config = copy.deepcopy(config)
            cpu_config["asr"] = dict(asr_config(config))
            cpu_config["asr"]["device_preference"] = "cpu"
            try:
                cpu_model, cpu_runtime, cpu_failures = load_whisper_model(cpu_config, run_logger)
                local_failures.extend(cpu_failures)
                result = process_file(
                    source,
                    cpu_config,
                    manifest,
                    cpu_model,
                    dict(cpu_runtime),
                    ffmpeg,
                    ffprobe,
                    force_reprocess=force_reprocess,
                    _return_context=_audio_only,
                    _job_id=_job_id,
                )
                if _audio_only and isinstance(result, dict) and result.get("_type") == "audio_context":
                    result["fallback_from"] = "cuda"
                    return result, local_failures
                if _audio_only:
                    return result, local_failures
                # Sequential fallback: run postprocess inline
                if isinstance(result, dict) and result.get("_type") == "audio_context":
                    pp_result, pp_failures = process_postprocess_stage(result, config, run_logger, _job_id=_job_id)
                    local_failures.extend(pp_failures)
                    pp_result["fallback_from"] = "cuda"
                    return pp_result, local_failures
                return result, local_failures
            except Exception as cpu_exc:
                logger.exception("CPU fallback also failed for %s", source.name)
                local_failures.append(f"CPU fallback failed: {cpu_exc}")
                _transition_job_db(_job_id, "failed", "failed",
                                   error_message=f"CUDA failed: {exc}; CPU fallback failed: {cpu_exc}",
                                   event_type="job_failed")
                return (
                    {
                        "file": source.name,
                        "status": "failed",
                        "error": f"CUDA failed: {exc}; CPU fallback failed: {cpu_exc}",
                        "task_id": file_fingerprint(source),
                    },
                    local_failures,
                )

        return ({"file": source.name, "status": "failed", "error": str(exc), "task_id": file_fingerprint(source)}, local_failures)

    # Normal path: call process_file
    try:
        result = process_file(
            source,
            config,
            manifest,
            model,
            runtime,
            ffmpeg,
            ffprobe,
            force_reprocess=force_reprocess,
            _return_context=_audio_only,
            _job_id=_job_id,
        )
    except Exception as exc:
        logger.exception("Failed to process %s", source.name)
        _transition_job_db(_job_id, "failed", "failed", error_message=str(exc), event_type="job_failed")
        return {"file": source.name, "status": "failed", "error": str(exc), "task_id": file_fingerprint(source)}, []

    # No exception path: handle result based on mode
    if _audio_only:
        # Return AudioContext or terminal result as-is for the dispatcher
        return result, local_failures

    # Sequential mode: run postprocess if audio succeeded
    if isinstance(result, dict) and result.get("_type") == "audio_context":
        return process_postprocess_stage(result, config, run_logger, _job_id=_job_id)

    # Terminal result (skipped, already processed, etc.)
    return result, local_failures


def main() -> int:
    parser = argparse.ArgumentParser(description="Scan inbox and transcribe podcast audio files to Markdown.")
    parser.add_argument("--dry-run", action="store_true", help="Check directories, ffmpeg, disk space, and pending files. Does not load Whisper model.")
    parser.add_argument("--check-model", action="store_true", help="Load the configured model/runtime, then exit.")
    parser.add_argument("--translate-existing", action="store_true", help="Translate existing work/internal/json/*.segments.json files and generate bilingual Markdown.")
    parser.add_argument("--force", action="store_true", help="Ignore previous run state and re-transcribe every supported audio file in input/.")
    parser.add_argument("--force-transcribe", action="store_true", help="Ignore previous run state and re-transcribe every supported audio file in input/.")
    parser.add_argument("--force-translate", action="store_true", help="When used with --translate-existing, overwrite existing bilingual translation output.")
    parser.add_argument("--no-open-output", action="store_true", help="Do not open the output folder after a normal transcription or translation run.")
    args = parser.parse_args()

    ensure_dirs()
    config = load_json(CONFIG_PATH, {})
    if args.dry_run:
        return dry_run(config)
    if args.translate_existing:
        return translate_existing_outputs(config, force=args.force_translate, no_open_output=args.no_open_output)

    if not acquire_run_lock():
        print(f"Another PodcastTranscriber run appears active: {RUN_LOCK_PATH}")
        return 4
    cleanup_work_artifacts()
    run_logger = setup_file_logger("run")
    cleanup_stale_tmp_files(run_logger)
    ffmpeg = find_executable("ffmpeg")
    ffprobe = find_executable("ffprobe")
    if not ffmpeg:
        run_logger.error("ffmpeg was not found. Install FFmpeg or rerun setup.")
        write_run_summary([], None, ["ffmpeg not found"])
        return 2

    preflight_checks(config, run_logger)

    if args.check_model:
        _, runtime, failures = load_whisper_model(config, run_logger)
        write_run_summary([], runtime, failures)
        print(f"Model check OK: {runtime}")
        return 0

    # Discover audio files
    audio_files = discover_audio_files()
    if not audio_files:
        print("No audio files found in input/.")
        return 0

    print(f"Found {len(audio_files)} audio file(s) to process.")
    for f in audio_files:
        print(f"  - {f.name}")

    force_reprocess = bool(
        args.force or args.force_transcribe or config.get("always_reprocess_inputs", False)
        or not config.get("skip_processed_files", True)
    )

    # Load model
    results: list[dict[str, Any]] = []
    runtime: dict[str, str] | None = None
    failures: list[str] = []
    model = None

    try:
        model, runtime, load_failures = load_whisper_model(config, run_logger)
        failures.extend(load_failures)
    except Exception as exc:
        run_logger.exception("Could not load any model runtime.")
        results.append({"file": "model", "status": "failed", "error": str(exc)})
        write_run_summary(results, None, failures)
        return 3

    # Process files with two-phase pipeline:
    # Phase 1: Audio transcription (GPU-bound, serialized by MODEL_TRANSCRIBE_LOCK)
    # Phase 2: Postprocess (API-bound translation + polish, runs in parallel)
    parallel_files = max_parallel_audio_files(config)
    pp_workers = max_parallel_postprocess_files(config)
    run_logger.info("Audio file concurrency: %s, Postprocess concurrency: %s", parallel_files, pp_workers)

    pp_executor = ThreadPoolExecutor(max_workers=pp_workers)
    pp_futures: dict[Future, str] = {}

    with ThreadPoolExecutor(max_workers=parallel_files) as audio_executor:
        future_to_source: dict[Future, Path] = {}
        for source in audio_files:
            task_id = file_fingerprint(source)
            future = audio_executor.submit(
                process_source_with_fallback,
                source, config, load_manifest(), model, runtime,
                ffmpeg, ffprobe, force_reprocess, run_logger,
                True,  # _audio_only — return context for postprocess
                task_id,  # _job_id
            )
            future_to_source[future] = source

        for future in as_completed(future_to_source):
            source = future_to_source[future]
            try:
                result_or_pair = future.result()
                if isinstance(result_or_pair, tuple):
                    result, local_failures = result_or_pair
                    failures.extend(local_failures)
                else:
                    result = result_or_pair
                if isinstance(result, dict) and result.get("_type") == "audio_context":
                    pp_f = pp_executor.submit(
                        process_postprocess_stage, result, config, run_logger, result.get("task_id")
                    )
                    pp_futures[pp_f] = source.name
                    run_logger.info("Audio done for %s, queued postprocess.", source.name)
                else:
                    results.append(result)
                    run_logger.info("Finished %s: %s", source.name, result.get("status", "unknown") if isinstance(result, dict) else "unknown")
            except Exception as exc:
                run_logger.exception("Failed to process %s", source.name)
                results.append({"file": source.name, "status": "failed", "error": str(exc)})

    for future in as_completed(pp_futures):
        name = pp_futures[future]
        try:
            pp_result, pp_failures = future.result()
            failures.extend(pp_failures)
            results.append(pp_result)
            run_logger.info("Postprocess finished %s: %s", name, pp_result.get("status", "unknown") if isinstance(pp_result, dict) else "unknown")
        except PodcastUpstreamError:
            raise
        except Exception as exc:
            run_logger.exception("Postprocess failed for %s", name)
            results.append({"file": name, "status": "failed", "error": str(exc)})

    pp_executor.shutdown(wait=False)

    summary_path = write_run_summary(results, runtime, failures)
    print(f"Run summary: {summary_path}")
    open_output_folder(config, run_logger, no_open_output=args.no_open_output)
    has_failed = any(item.get("status") == "failed" for item in results)
    if not has_failed:
        logging.shutdown()
        cleanup_work_artifacts()
    return 0 if not has_failed else 1


if __name__ == "__main__":
    raise SystemExit(main())

