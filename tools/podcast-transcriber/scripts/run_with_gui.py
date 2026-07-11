"""Minimal GUI HTTP server for PodcastTranscriber.

Serves the HTML frontend and a few JSON API endpoints.
The transcription worker (transcribe_podcasts.py) is launched as a subprocess;
its progress is read from work/state/*.json files that the worker writes.
"""

import json
import os
import shutil
import subprocess
import sys
import threading
import time
from collections import deque
from datetime import datetime
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

APP_ROOT = Path(__file__).resolve().parents[1]
if str(APP_ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(APP_ROOT / "scripts"))

from podcast_transcriber.common import CONFIG_PATH, INBOX, OUT_LOGS, OUTPUT, STATE_DIR, WORK  # noqa: E402

ROOT = Path(os.environ.get("IMMERSIVE_PODCAST_DATA_ROOT", APP_ROOT)).resolve()
PYTHON_EXE = Path(os.environ.get("IMMERSIVE_PODCAST_PYTHON", sys.executable)).resolve()
TRANSCRIBE_SCRIPT = APP_ROOT / "scripts" / "transcribe_podcasts.py"
SETUP_SCRIPT = APP_ROOT / "scripts" / "setup.ps1"
TRAY_SCRIPT = APP_ROOT / "scripts" / "tray_icon.ps1"
OUTPUT_DIR = OUTPUT
INPUT_DIR = INBOX
LOG_DIR = OUT_LOGS
WORK_DIR = WORK
SUPPORTED_EXTENSIONS = {".mp3", ".m4a", ".wav"}
VIDEO_EXTENSIONS = {".mp4", ".mkv", ".mov", ".webm", ".avi", ".m4v", ".ts", ".flv", ".wmv"}
# Audio + video the GUI will list from input/. Video audio tracks are extracted
# by the worker (ffmpeg) before transcription.
ACCEPTED_INPUT_EXTENSIONS = SUPPORTED_EXTENSIONS | VIDEO_EXTENSIONS
DEFAULT_PORT = 8765
DEEPSEEK_BASE_URL = "https://api.deepseek.com"
DEEPSEEK_DEFAULT_MODEL = "deepseek-v4-flash"

# ── Global state ──
LOG_LINES: deque[str] = deque(maxlen=200)
LOG_LOCK = threading.Lock()
PROCESS: subprocess.Popen | None = None
PROCESS_LOCK = threading.Lock()
CONFIG_LOCK = threading.Lock()
SERVER: ThreadingHTTPServer | None = None
TRAY_PROCESS: subprocess.Popen | None = None
SHUTTING_DOWN = threading.Event()
WORKER_EXIT_CODE: int | None = None
APP_URL: str = ""
APP_WINDOW = None
APP_WINDOW_LOCK = threading.Lock()


def iso_now() -> str:
    return datetime.now().isoformat(timespec="seconds")


def append_log(msg: str) -> None:
    with LOG_LOCK:
        LOG_LINES.append(msg)


def tail_text_file(path: Path, max_lines: int = 80) -> list[str]:
    """Read a short tail from a UTF-8 log file without failing snapshot polling."""
    if not path.exists() or not path.is_file():
        return []
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []
    return lines[-max_lines:]


def load_config() -> dict:
    if not CONFIG_PATH.exists():
        return {}
    try:
        return json.loads(CONFIG_PATH.read_text(encoding="utf-8-sig"))
    except Exception:
        return {}


def save_config(config: dict) -> None:
    """Write config.json atomically."""
    with CONFIG_LOCK:
        payload = json.dumps(config, ensure_ascii=False, indent=2)
        tmp = CONFIG_PATH.with_suffix(".tmp")
        tmp.write_text(payload, encoding="utf-8")
        os.replace(tmp, CONFIG_PATH)


def provider_name(config: dict) -> str:
    return str(config.get("backend", config.get("provider", "ollama"))).strip().lower()


def has_api_entry(config: dict) -> bool:
    if not str(config.get("base_url") or "").strip():
        return False
    env_name = str(config.get("api_key_env") or "").strip()
    return bool(env_name or str(config.get("api_key") or "").strip())


def api_key_available(config: dict) -> bool:
    env_name = str(config.get("api_key_env") or "").strip()
    if env_name and os.environ.get(env_name):
        return True
    return bool(str(config.get("api_key") or "").strip())


def effective_backend(config: dict) -> str:
    backend = provider_name(config)
    if backend == "deepseek" and not has_api_entry(config):
        return "unconfigured_api"
    return backend


def get_translation_config() -> dict:
    """Return translation-related settings for the frontend."""
    config = load_config()
    translation = config.get("translation") or {}
    polish = (config.get("markdown") or {}).get("llm_polish") or {}
    raw_key = str(translation.get("api_key") or "").strip()
    masked_key = ""
    if raw_key and len(raw_key) > 6:
        masked_key = raw_key[:3] + "*" * (len(raw_key) - 6) + raw_key[-3:]
    elif raw_key:
        masked_key = "***"
    return {
        "translation_backend": translation.get("backend", "ollama"),
        "effective_translation_backend": effective_backend(translation),
        "translation_model": translation.get("model", "qwen3.5:9b"),
        "deepseek_base_url": translation.get("base_url") or DEEPSEEK_BASE_URL,
        "translation_enabled": bool(translation.get("english_required", False)),
        "translation_api_configured": has_api_entry(translation),
        "translation_api_key_available": api_key_available(translation),
        "polish_backend": polish.get("backend", "ollama"),
        "effective_polish_backend": effective_backend(polish),
        "polish_model": polish.get("model", "qwen3.5:9b"),
        "polish_enabled": bool(polish.get("enabled", True)),
        "polish_api_configured": has_api_entry(polish),
        "polish_api_key_available": api_key_available(polish),
        "masked_api_key": masked_key,
    }


def configure_deepseek_entry(section: dict, api_key: str | None = None) -> list[str]:
    changed = []
    old_url = str(section.get("base_url") or "")
    if old_url != DEEPSEEK_BASE_URL:
        section["base_url"] = DEEPSEEK_BASE_URL
        changed.append(f"DeepSeek base_url: {old_url or '<empty>'} → {DEEPSEEK_BASE_URL}")
    old_model = str(section.get("model") or "")
    if old_model not in ("deepseek-v4-flash", "deepseek-v4-pro"):
        section["model"] = DEEPSEEK_DEFAULT_MODEL
        changed.append(f"DeepSeek model: {old_model or '<empty>'} → {DEEPSEEK_DEFAULT_MODEL}")
    if api_key is not None:
        old_key = str(section.get("api_key") or "")
        section["api_key"] = api_key
        section["api_key_env"] = ""
        if old_key != api_key:
            changed.append("DeepSeek API key updated")
    return changed


def update_translation_config(data: dict) -> dict:
    """Update translation backend settings in config.json."""
    with CONFIG_LOCK:
        config = load_config()
        translation = config.setdefault("translation", {})
        polish = config.setdefault("markdown", {}).setdefault("llm_polish", {})

        changed = []

        # Translation backend
        new_backend = data.get("translation_backend")
        if new_backend and new_backend in ("ollama", "deepseek", "none"):
            old = translation.get("backend", "ollama")
            if old != new_backend:
                translation["backend"] = new_backend
                changed.append(f"translation backend: {old} → {new_backend}")
            if new_backend == "deepseek":
                changed.extend(f"translation {item}" for item in configure_deepseek_entry(translation))

        # Translation model
        new_model = data.get("translation_model")
        if new_model and isinstance(new_model, str) and new_model.strip():
            old = translation.get("model", "qwen3.5:9b")
            if old != new_model.strip():
                translation["model"] = new_model.strip()
                changed.append(f"translation model: {old} → {new_model.strip()}")

        # Translation enabled
        if "translation_enabled" in data:
            new_val = bool(data["translation_enabled"])
            old_val = bool(translation.get("english_required", False))
            if old_val != new_val:
                translation["english_required"] = new_val
                changed.append(f"translation enabled: {old_val} → {new_val}")

        # Polish backend
        new_polish_backend = data.get("polish_backend")
        if new_polish_backend and new_polish_backend in ("ollama", "deepseek"):
            old = polish.get("backend", "ollama")
            if old != new_polish_backend:
                polish["backend"] = new_polish_backend
                changed.append(f"polish backend: {old} → {new_polish_backend}")
            if new_polish_backend == "deepseek":
                changed.extend(f"polish {item}" for item in configure_deepseek_entry(polish))

        # Polish model
        new_polish_model = data.get("polish_model")
        if new_polish_model and isinstance(new_polish_model, str) and new_polish_model.strip():
            old = polish.get("model", "qwen3.5:9b")
            if old != new_polish_model.strip():
                polish["model"] = new_polish_model.strip()
                changed.append(f"polish model: {old} → {new_polish_model.strip()}")

        # Polish enabled
        if "polish_enabled" in data:
            new_val = bool(data["polish_enabled"])
            old_val = bool(polish.get("enabled", True))
            if old_val != new_val:
                polish["enabled"] = new_val
                changed.append(f"polish enabled: {old_val} → {new_val}")

        if "deepseek_api_key" in data:
            api_key = str(data.get("deepseek_api_key") or "").strip()
            if api_key:
                changed.extend(f"translation {item}" for item in configure_deepseek_entry(translation, api_key))
                changed.extend(f"polish {item}" for item in configure_deepseek_entry(polish, api_key))

        if changed:
            save_config(config)
            for c in changed:
                append_log(f"[Config] {c}")
    current = get_translation_config()
    current.update({"ok": True, "changed": changed})
    return current


# ── Worker management ──

def worker_is_running() -> bool:
    with PROCESS_LOCK:
        return PROCESS is not None and PROCESS.poll() is None


def start_worker() -> dict:
    """Start the transcription worker subprocess."""
    global PROCESS, WORKER_EXIT_CODE

    if worker_is_running():
        return {"ok": False, "error": "Worker already running"}

    if not PYTHON_EXE.exists():
        append_log("Setting up .venv ...")
        try:
            subprocess.run(
                ["powershell", "-File", str(SETUP_SCRIPT)],
                check=True, capture_output=True,
            )
        except Exception as exc:
            append_log(f"Setup failed: {exc}")
            return {"ok": False, "error": f"Setup failed: {exc}"}

    env = os.environ.copy()
    env["PYTHONUTF8"] = "1"
    env["PODCAST_TRANSCRIBER_NO_OPEN_OUTPUT"] = "1"

    try:
        with PROCESS_LOCK:
            PROCESS = subprocess.Popen(
                [str(PYTHON_EXE), "-u", str(TRANSCRIBE_SCRIPT), "--force", "--no-open-output"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
            WORKER_EXIT_CODE = None

        # Read stdout in a background thread
        def _read_stdout():
            global WORKER_EXIT_CODE
            try:
                for line in PROCESS.stdout:
                    line = line.strip()
                    if line:
                        append_log(line)
            except (ValueError, AttributeError):
                pass
            PROCESS.wait()
            WORKER_EXIT_CODE = PROCESS.returncode
            append_log(f"Worker exited with code {WORKER_EXIT_CODE}")

        threading.Thread(target=_read_stdout, daemon=True).start()
        append_log(f"Worker started (PID {PROCESS.pid})")
        return {"ok": True}

    except Exception as exc:
        append_log(f"Failed to start worker: {exc}")
        return {"ok": False, "error": str(exc)}


def stop_worker() -> dict:
    """Stop the worker subprocess."""
    global PROCESS
    if not worker_is_running():
        return {"ok": True, "message": "Worker not running"}

    with PROCESS_LOCK:
        if PROCESS is None:
            return {"ok": True, "message": "Worker not running"}
        pid = PROCESS.pid
        # Close stdout pipe first to unblock the reader thread
        if PROCESS.stdout:
            try:
                PROCESS.stdout.close()
            except Exception:
                pass

    append_log(f"Stopping worker (PID {pid})...")

    try:
        if os.name == "nt":
            # Use taskkill /T to kill the entire process tree
            subprocess.run(
                ["taskkill", "/PID", str(pid), "/T", "/F"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                timeout=10,
            )
        else:
            with PROCESS_LOCK:
                if PROCESS is not None:
                    PROCESS.terminate()
                    PROCESS.wait(timeout=5)

        with PROCESS_LOCK:
            PROCESS = None
        append_log("Worker stopped.")
        return {"ok": True}
    except Exception as exc:
        append_log(f"Error stopping worker: {exc}")
        return {"ok": False, "error": str(exc)}


def clear_directory_contents(path: Path, keep_names: set[str] | None = None) -> int:
    keep_names = keep_names or set()
    if not path.exists():
        path.mkdir(parents=True, exist_ok=True)
        return 0
    removed = 0
    for item in path.iterdir():
        if item.name in keep_names:
            continue
        try:
            if item.is_dir():
                shutil.rmtree(item)
            else:
                item.unlink()
            removed += 1
        except OSError as exc:
            append_log(f"Could not remove {item}: {exc}")
    return removed


def clear_run_artifacts() -> dict:
    """Remove outputs and all intermediate runtime artifacts after Stop."""
    removed_output = clear_directory_contents(OUTPUT_DIR, keep_names={".gitkeep"})
    removed_work = clear_directory_contents(WORK_DIR)
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    append_log(f"Cleared stop artifacts: output items={removed_output}, work items={removed_work}")
    return {"output_items_removed": removed_output, "work_items_removed": removed_work}


def quit_application() -> dict:
    result = stop_worker()
    cleanup = clear_run_artifacts()
    # Full cleanup on tray "退出并清理": also wipe the input audio files (keep
    # .gitkeep). Stop only clears outputs/work; exit clears everything.
    removed_input = clear_directory_contents(INPUT_DIR, keep_names={".gitkeep"})
    cleanup["input_items_removed"] = removed_input
    append_log(f"Cleared input on exit: input items={removed_input}")
    result["cleanup"] = cleanup
    SHUTTING_DOWN.set()
    append_log("Tray exit requested; shutting down GUI service.")
    with APP_WINDOW_LOCK:
        if APP_WINDOW is not None:
            try:
                APP_WINDOW.destroy()
            except Exception as exc:
                append_log(f"Could not destroy WebView window: {exc}")
    if SERVER is not None:
        threading.Thread(target=SERVER.shutdown, daemon=True).start()
    return result


# ── API data ──

def get_tasks() -> list[dict]:
    """Read work/state/*.json files and build a task list for the frontend."""
    tasks = []
    if not STATE_DIR.exists():
        return tasks

    for path in STATE_DIR.glob("*.json"):
        if path.name in ("manifest.json", "gui_server.json", "deepseek_polish_usage.json") or path.name.endswith((".interview.json", ".translation.json", "_usage.json")):
            continue
        try:
            state = json.loads(path.read_text(encoding="utf-8-sig"))
        except Exception:
            continue
        if not state.get("task_id") and not state.get("source_file"):
            continue

        source = state.get("source_file") or path.stem
        status = str(state.get("status") or "queued").lower()
        stage = str(state.get("stage") or status)
        progress = float(state.get("progress_percent") or 0)
        error = state.get("error_message") or state.get("error") or ""
        detected_lang = str(state.get("detected_language") or "")
        current_chunk = int(state.get("current_chunk") or 0)
        total_chunks = int(state.get("total_chunks") or 0)
        updated_at = state.get("updated_at") or state.get("last_update_at") or ""
        log_path_value = state.get("log_path") or ""
        log_path = Path(log_path_value) if log_path_value else LOG_DIR / f"{Path(source).stem}.log"
        if not log_path.is_absolute():
            log_path = ROOT / log_path

        # Determine card state for the frontend
        if status in ("success", "completed", "partial_success"):
            card_state = "done"
        elif status in ("failed", "stalled", "interrupted", "cancelled", "stalled_recoverable"):
            card_state = "error"
        elif status in ("transcribing", "normalizing", "chunking", "preparing",
                        "translating", "polishing", "writing_output", "postprocess_queued"):
            card_state = "running"
        else:
            card_state = "waiting"

        # Stage label (Chinese)
        stage_labels = {
            "queued": "排队中",
            "preparing": "准备中",
            "normalizing": "音频规范化",
            "chunking": "音频切分",
            "transcribe_waiting": "等待转写通道",
            "transcribing": "正在语音转写",
            "postprocess_queued": "等待后处理",
            "translating": "正在翻译",
            "polishing": "正在润色",
            "writing_output": "正在写入 Markdown",
            "success": "已完成",
            "completed": "已完成",
            "partial_success": "部分完成",
            "failed": "失败",
            "interrupted": "已中断",
            "cancelled": "已取消",
            "stalled": "已停滞",
            "stalled_recoverable": "已停滞（可恢复）",
        }
        stage_label = stage_labels.get(stage, stage_labels.get(status, stage))

        # Check if output markdown exists
        source_stem = Path(source).stem if source else path.stem
        final_md = OUTPUT_DIR / f"{source_stem}.md"
        has_output = final_md.exists()

        # Translation info
        trans_total = int(state.get("translation_total") or 0)
        trans_done = int(state.get("translation_done") or 0)
        trans_pct = round(trans_done / trans_total * 100, 1) if trans_total > 0 else 0.0

        # Polish info
        polish_total = int(state.get("polish_total") or 0)
        polish_done = int(state.get("polish_done") or 0)
        polish_pct = round(polish_done / polish_total * 100, 1) if polish_total > 0 else 0.0
        polish_summary = state.get("polish_summary") or None

        tasks.append({
            "task_id": path.stem,
            "name": source,
            "state": card_state,
            "status": status,
            "stage": stage,
            "stage_label": stage_label,
            "progress": round(progress, 1),
            "error": error,
            "detected_language": detected_lang,
            "current_chunk": current_chunk,
            "total_chunks": total_chunks,
            "translation_pct": trans_pct,
            "polish_pct": polish_pct,
            "polish_done": polish_done,
            "polish_total": polish_total,
            "polish_summary": polish_summary,
            "has_output": has_output,
            "updated_at": updated_at,
            "log_path": str(log_path),
            "log_tail": tail_text_file(log_path, max_lines=80),
        })

    # Sort: running first, then waiting, then done/error
    order = {"running": 0, "waiting": 1, "done": 2, "error": 3}
    tasks.sort(key=lambda t: (order.get(t["state"], 9), t["name"]))
    return tasks


def get_snapshot() -> dict:
    """Build the full snapshot for the frontend."""
    tasks = get_tasks()
    total = len(tasks)
    active = sum(1 for t in tasks if t["state"] == "running")
    waiting = sum(1 for t in tasks if t["state"] == "waiting")
    done = sum(1 for t in tasks if t["state"] == "done")
    failed = sum(1 for t in tasks if t["state"] == "error")

    return {
        "tasks": tasks,
        "stats": {
            "total": total,
            "active": active,
            "waiting": waiting,
            "done": done,
            "failed": failed,
        },
        "worker": {
            "running": worker_is_running(),
            "pid": PROCESS.pid if worker_is_running() else None,
            "exit_code": WORKER_EXIT_CODE,
        },
        "logs": list(LOG_LINES)[-30:],
    }


def get_health() -> dict:
    return {"ok": True, "gui_service": "ok"}


def input_files() -> list[str]:
    """List audio/video files in input/."""
    if not INPUT_DIR.exists():
        return []
    return [
        f.name for f in INPUT_DIR.iterdir()
        if f.is_file() and f.suffix.lower() in ACCEPTED_INPUT_EXTENSIONS
    ]


def handle_action(data: dict) -> dict:
    action = data.get("action", "")

    if action == "start":
        return start_worker()
    elif action == "stop":
        result = stop_worker()
        cleanup = clear_run_artifacts()
        result["cleanup"] = cleanup
        return result
    elif action in ("quit", "exit"):
        return quit_application()
    elif action == "retry":
        # Stop worker if running, clear state files, then restart
        if worker_is_running():
            stop_worker()
            # Wait for the process to fully exit (max 5 seconds)
            for _ in range(10):
                if not worker_is_running():
                    break
                time.sleep(0.5)
        # Clear state files
        if STATE_DIR.exists():
            for p in STATE_DIR.glob("*.json"):
                if p.name != "manifest.json":
                    try:
                        p.unlink()
                    except OSError:
                        pass
        return start_worker()
    elif action == "open_output":
        try:
            subprocess.Popen(["explorer.exe", str(OUTPUT_DIR)])
            return {"ok": True}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}
    elif action == "heartbeat":
        return {"ok": True}
    else:
        return {"ok": False, "error": f"Unknown action: {action}"}


def show_app_window() -> dict:
    """Restore the single GUI window, or open the app-mode browser fallback."""
    global APP_WINDOW
    with APP_WINDOW_LOCK:
        if APP_WINDOW is not None:
            try:
                APP_WINDOW.show()
                try:
                    APP_WINDOW.restore()
                except Exception:
                    pass
                try:
                    APP_WINDOW.on_top = True
                    APP_WINDOW.on_top = False
                except Exception:
                    pass
                append_log("Window restored from tray.")
                return {"ok": True, "mode": "webview"}
            except Exception as exc:
                append_log(f"Could not restore WebView window: {exc}")
        if APP_URL and open_app_window(APP_URL):
            append_log("App-mode browser window opened from tray.")
            return {"ok": True, "mode": "browser"}
    return {"ok": False, "error": "Could not open window"}


def handle_window_action(data: dict) -> dict:
    action = data.get("action", "show")
    if action in ("show", "open", "restore"):
        return show_app_window()
    return {"ok": False, "error": f"Unknown window action: {action}"}


# ── HTTP Server ──

class APIServer(ThreadingHTTPServer):
    # HTTPServer defaults allow_reuse_address=True (SO_REUSEADDR). On Windows that
    # lets us silently bind a port another process is already listening on, so the
    # OSError-based "port in use" fallback never fires and our requests get routed
    # to the foreign server (producing a bogus 404). Disable it so a busy port
    # raises OSError and we fail over to a free one.
    allow_reuse_address = False
    daemon_threads = True


class APIHandler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(APP_ROOT), **kwargs)

    def do_OPTIONS(self):
        self.send_response(200)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path

        if path == "/api/snapshot":
            self._json(get_snapshot())
        elif path == "/api/health":
            self._json(get_health())
        elif path == "/api/inputs":
            self._json({"files": input_files()})
        elif path == "/api/config":
            self._json(get_translation_config())
        else:
            super().do_GET()

    def do_POST(self):
        parsed = urlparse(self.path)
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length > 0 else b""
        body = raw.decode("utf-8", errors="replace") if raw else "{}"
        try:
            data = json.loads(body)
        except Exception:
            self._json({"ok": False, "error": "Invalid JSON"})
            return
        if parsed.path == "/api/action":
            self._json(handle_action(data))
        elif parsed.path == "/api/window":
            self._json(handle_window_action(data))
        elif parsed.path == "/api/config":
            self._json(update_translation_config(data))
        else:
            self.send_error(404)

    def _json(self, data: dict):
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(json.dumps(data, ensure_ascii=False).encode("utf-8"))
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass

    def log_message(self, format, *args):
        pass  # Suppress default logging


def open_app_window(url: str) -> bool:
    """Open Chrome or Edge in app mode."""
    candidates = [
        Path(os.environ.get("PROGRAMFILES", "")) / "Google" / "Chrome" / "Application" / "chrome.exe",
        Path(os.environ.get("PROGRAMFILES(X86)", "")) / "Google" / "Chrome" / "Application" / "chrome.exe",
        Path(os.environ.get("PROGRAMFILES", "")) / "Microsoft" / "Edge" / "Application" / "msedge.exe",
        Path(os.environ.get("PROGRAMFILES(X86)", "")) / "Microsoft" / "Edge" / "Application" / "msedge.exe",
    ]
    for exe in candidates:
        if exe.exists():
            subprocess.Popen(
                [str(exe), f"--app={url}", "--new-window", "--window-size=1240,820",
                 "--disable-extensions"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
            return True
    return False


def start_tray_icon(url: str, port: int) -> None:
    """Start a Windows tray menu for reopening or fully exiting the GUI."""
    global TRAY_PROCESS
    if os.name != "nt" or not TRAY_SCRIPT.exists():
        return
    exit_endpoint = f"http://127.0.0.1:{port}/api/action"
    open_endpoint = f"http://127.0.0.1:{port}/api/window"
    try:
        TRAY_PROCESS = subprocess.Popen(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(TRAY_SCRIPT),
                "-Url",
                url,
                "-ExitEndpoint",
                exit_endpoint,
                "-OpenEndpoint",
                open_endpoint,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        append_log("Tray icon started.")
    except Exception as exc:
        TRAY_PROCESS = None
        append_log(f"Could not start tray icon: {exc}")


def start_webview_window(url: str) -> bool:
    """Start one native WebView window; closing it hides to tray."""
    global APP_WINDOW
    try:
        import webview
    except Exception as exc:
        append_log(f"pywebview unavailable; using browser fallback: {exc}")
        return False

    try:
        window = webview.create_window(
            "Podcast Transcriber",
            url,
            width=1240,
            height=820,
            resizable=True,
            confirm_close=False,
        )
        APP_WINDOW = window

        def _hide_to_tray():
            if SHUTTING_DOWN.is_set():
                return True
            append_log("Window closed; hidden to tray.")
            window.hide()
            return False

        window.events.closing += _hide_to_tray
        webview.start(gui="edgechromium", debug=False)
        return True
    except Exception as exc:
        append_log(f"Could not start WebView window: {exc}")
        APP_WINDOW = None
        return False


def stop_tray_icon() -> None:
    global TRAY_PROCESS
    if TRAY_PROCESS is None or TRAY_PROCESS.poll() is not None:
        TRAY_PROCESS = None
        return
    try:
        TRAY_PROCESS.terminate()
        TRAY_PROCESS.wait(timeout=3)
    except Exception:
        try:
            TRAY_PROCESS.kill()
        except Exception:
            pass
    TRAY_PROCESS = None


def main():
    global APP_URL
    print("=" * 50)
    print("Podcast Transcriber v2")
    print("=" * 50)

    # Start HTTP server. Try the preferred port and a few neighbours; if all are
    # taken (e.g. another local app already holds 8765), fall back to a random
    # free port. APIServer disables SO_REUSEADDR so a busy port raises OSError
    # here instead of silently sharing it with the other server.
    server = None
    for candidate in [DEFAULT_PORT + offset for offset in range(10)] + [0]:
        try:
            server = APIServer(("127.0.0.1", candidate), APIHandler)
            break
        except OSError:
            continue
    if server is None:
        print("Could not bind any local port for the GUI server.")
        return
    port = server.server_address[1]

    global SERVER
    SERVER = server
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    url = f"http://127.0.0.1:{port}/podcast-transcriber-v2.html"
    APP_URL = url
    print(f"\nUI: {url}")

    # Keep shortcut launches idle. The worker should only start from the UI so
    # the visible Start/Stop state always matches the real subprocess state.
    files = input_files()
    if files:
        print(f"Found {len(files)} audio/video file(s) in input/. Click 'Start' in the UI to begin.")
    else:
        print("No audio/video files in input/. Add files and click 'Start' in the UI.")

    start_tray_icon(url, port)

    print("\nPress Ctrl+C to stop.\n")

    try:
        if not start_webview_window(url):
            if not open_app_window(url):
                print(f"Could not open browser. Please open:\n  {url}")
            while not SHUTTING_DOWN.is_set():
                time.sleep(1)
    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        if worker_is_running():
            stop_worker()
        stop_tray_icon()
        server.shutdown()
        server.server_close()
        print("Goodbye.")


if __name__ == "__main__":
    main()
