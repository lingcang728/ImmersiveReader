# AGENT_HANDOFF.md

**Key:** `PODCAST_TRANSCRIBER_HANDOFF_KEY`
**Last Updated:** 2026-06-22

### 2026-06-22 Video Input + Sidecar Subtitle Support

- **Goal**: accept video files (`.mp4`/`.mkv`/...) and reuse a manually placed sidecar subtitle as the transcript when present. Transcription conventions are unchanged (EN→中英双语, ZH→直接转写, both polished).
- **Audio extraction was already free**: `normalize_audio()`/`prepare_chunks()` already run ffmpeg with `-vn`, so a video just needed its extension allow-listed. Video extensions live in a **separate** config key `video.supported_extensions` (NOT `audio.supported_extensions`, which `quick_validate.py` still pins to exactly mp3/m4a/wav). Constants `VIDEO_EXTENSIONS` / `SIDECAR_SUBTITLE_EXTENSIONS` added in `podcast_transcriber/common.py`.
- **Discovery/GUI**: `discover_audio_files()` now unions `video.supported_extensions`; `run_with_gui.py` lists video too via `ACCEPTED_INPUT_EXTENSIONS`.
- **Sidecar subtitle path** (decision: trust only same-stem external `.srt/.ass/.vtt`, never embedded tracks, never use a foreign-language sub as the translation): `process_file()` checks `find_sidecar_subtitle()` before `normalize_audio`; if found, `parse_subtitle_to_segments()` builds `{id,start,end,text}` and the branch **mirrors the `resume_translation` recovery path** — builds an `audio_context` and calls `process_postprocess_stage()`, skipping ASR. Parse failure falls back to normal transcription. `.srt/.vtt` parsed directly (tolerant decode: utf-8-sig→utf-8→gb18030 for Chinese subs); `.ass/.ssa` converted via ffmpeg to srt first. Both `<...>` and ASS `{\...}` override blocks are stripped.
- **Validation**: `quick_validate.py` now also requires `video.supported_extensions` contains `.mp4`. `py_compile` of all changed scripts passed; `quick_validate OK`; `--dry-run` discovers a generated test MP4; unit tests cover SRT/VTT/GB18030/ASS-via-ffmpeg parsing and the sidecar finder. The live ASR+DeepSeek path was NOT auto-run (GPU + paid API); it is unchanged and shared with the audio path.
- **Note for users**: place the sidecar in the content's ORIGINAL language; a translated (e.g. Chinese) sidecar would be treated as the transcript and yield a Chinese-only doc. Burned-in/hardcoded subtitles can't be extracted → always transcribed.

### 2026-05-21 Comprehensive Risk Audit & Bug Fix Batch

- **Scope**: Full codebase risk audit across `transcribe_podcasts.py`, `polish_interview_markdown.py`, `run_with_gui.py`, `quick_validate.py`, and `podcast-transcriber-v2.html`. Identified ~50 risks/bugs, fixed 11 across all files.
- **Critical/High fixes applied**:
  - `transcribe_podcasts.py` H1: `av.time_base` → `av.TIME_BASE` (PyAV constant was lowercase, causing AttributeError that silently disabled PyAV duration probing)
  - `transcribe_podcasts.py` M8: Removed duplicate `state["completed_at"] = now_stamp()` assignment
  - `polish_interview_markdown.py` H7: Added `_LLM_CONFIG_LOCK` (threading.Lock) to protect `_consecutive_errors` and `_disabled_after_error` reads/writes from race conditions in multithreaded polish
  - `run_with_gui.py` C3: `do_POST` now protects against empty/missing Content-Length
  - `run_with_gui.py` H3: `worker_is_running()` and `stop_worker()` now use `PROCESS_LOCK` for all PROCESS access
  - `run_with_gui.py` H4: `stop_worker()` now resets `PROCESS = None` after taskkill
  - `run_with_gui.py` H9: Retry operation now polls `worker_is_running()` up to 5s instead of fixed `time.sleep(1)`
  - `run_with_gui.py` M5: Added `CONFIG_LOCK` to protect concurrent `save_config()` calls
  - `quick_validate.py` M6: Relaxed extension validation to include all supported formats
  - `quick_validate.py` M7: Binary files filtered from script content scans
  - `podcast-transcriber-v2.html` M12: Fixed `escapeAttr` to properly escape double quotes
- **Issues confirmed already fixed in current code**: C1 (postprocess cancelled return tuples), C2 (run_logger guard), H2 (save_json atomic fallback), H5 (inferred_language consistency), H6 (ollama retry), H8 (semaphore set_limit), M1 (save_text atomic), M2 (main loop try/except), M3 (usage JSON corruption), M4 (Chinese comma regex), M11 (pp_executor wait=True), M13 (AGA regex boundary), L1 (chain replacement), L7 (acquire_run_lock loop)
- **Line ending normalization**: Both `transcribe_podcasts.py` and `polish_interview_markdown.py` had mixed `\r\n` / `\n` line endings causing editing tool issues; normalized to `\n`
- **Validation**: `py_compile` all 4 scripts passed, `quick_validate.py` passed, `--dry-run` passed

### 2026-05-20 Chinese Podcast Polish Quality Fix

- **Root cause found**: the two Chinese outputs looked barely polished because `only_suspect_blocks=true` relied mostly on `needs_polish`, but Chinese turn building rarely set that flag. The original GUI run finished postprocess in about 2 seconds after audio completion, and the task logs had no `DeepSeek batch polish completed` / `DeepSeek polish request completed` entries, so most Chinese text never reached the LLM polish stage.
- **Current polish behavior**: `scripts/polish_interview_markdown.py` now scores Chinese blocks across the whole file for high-risk ASR/polish issues, selects the top suspect blocks instead of only the earliest flagged blocks, passes the source title as a global proper-noun anchor, and caps Chinese batch size to avoid JSON truncation/fallback storms.
- **Name/term cleanup**: deterministic cleanup now covers the errors seen in the two fresh outputs, including `姚顺雨`, `谢赛宁`, `张小珺`, `OpenAI`, `Anthropic`, `DeepMind`, `ChatGPT`, `GPT-1/2/3`, `普林斯顿`, `姚班`, `word2vec`, `deep learning`, `world model`, and common simplified-Chinese conversion before final Markdown is written.
- **Speaker/paragraph cleanup**: Chinese opener heuristics now split mixed dialogue segments such as host prompts followed by guest self-introductions, classify intro host monologues more reliably, and use shorter Chinese merge/paragraph limits (`zh_merge_chars=420`, `zh_max_paragraph_chars=420`) for less overlong paragraphing.
- **Regenerated outputs**: the two current final Markdown files in `output/` were regenerated from existing `work/internal/json/*.segments.json` with the new logic. A targeted grep for the observed bad terms and common residual traditional characters returned no matches after the final regeneration.
- **Observed DeepSeek polish cost/time**: `work/state/deepseek_polish_usage.json` is cumulative for this repair session and ended at 114 requests, 1051.257 elapsed seconds, 246277 total tokens, estimated `$0.04148711` on `deepseek-v4-flash`. Earlier failed/over-broad attempts are included in that cumulative file; the final all-file regeneration completed in about 174 seconds after the batch/limit fixes.
- **Validation**: `python -m py_compile scripts/polish_interview_markdown.py` passed; `python scripts/quick_validate.py` passed (`quick_validate OK`); direct `python scripts/polish_interview_markdown.py --final-only` regenerated both current Markdown files successfully.
- **Risk to watch**: this is still heuristic speaker attribution, not true diarization. Mixed-speaker ASR segments can be improved with targeted split markers, but strict speaker separation still needs a real diarization layer. `config.json` remains ignored and may contain a real API key; do not force-add it.

### 2026-05-20 Turbo ASR and Final Markdown Failure Fix

- **Final Markdown bug fixed**: both recent failed jobs hit `NameError: name 'config' is not defined` in `write_final_markdown_from_json()`. The function now accepts the active full config and injects the shared DeepSeek limiter into `polish_interview_markdown.py` without relying on an out-of-scope local.
- **Fast ASR profile**: `config.example.json` now defaults `asr.model` to `large-v3-turbo`. Local `config.json` was also updated, but remains ignored because it may contain a real API key. Keep the old `models--Systran--faster-whisper-large-v3` cache for rollback.
- **Turbo prewarm**: run `.\.venv\Scripts\python.exe -c "from faster_whisper import WhisperModel; WhisperModel('large-v3-turbo', device='cuda', compute_type='int8_float16', download_root='models')"` before the first real run if you want the model downloaded outside the GUI worker.
- **Translation throughput profile**: DeepSeek API concurrency is now configured at `pipeline.max_deepseek_api_requests=6`, outer translation concurrency at `pipeline.max_parallel_translations=6`, per-file translation workers at `translation.max_batch_workers=6`, and translation batches at `32` segments / `12,000` chars to avoid the observed 48-segment truncation and JSON/ID retries.
- **Polish throughput profile**: long files now default to suspect-block-only polish with `max_blocks_per_file=24`, `batch_blocks=12`, and `max_batch_chars=12,000`, preserving final readability without full-file LLM polish becoming the bottleneck.
- **Validation**: run `python -m py_compile scripts/transcribe_podcasts.py scripts/polish_interview_markdown.py scripts/run_with_gui.py`, `python scripts/quick_validate.py`, `python scripts/transcribe_podcasts.py --dry-run`, `python -m pytest tests`, and direct `polish_interview_markdown.py --final-only` against any failed `work/internal/json/*.segments.json` that should be recovered.
- **Risk to watch**: if DeepSeek returns 429 or latency spikes, reduce `pipeline.max_deepseek_api_requests` and `translation.max_batch_workers` together. If Turbo quality is insufficient for a specific audio, roll `asr.model` back to `large-v3`.

### 2026-05-20 DeepSeek API Concurrency Fix Follow-up

- **Default API direction**: `config.example.json` defaults translation and polish to DeepSeek API (`deepseek-v4-flash`). The current fast profile uses translation batches at `32` segments / `12,000` chars / `8,192` max output tokens. Local Ollama remains supported but is no longer the example default.
- **Shared API limiter repaired**: `transcribe_podcasts.py` now uses an adjustable process-local DeepSeek limiter object. Translation and final polish share the same injected limiter, and 429 throttling lowers the shared limiter capacity instead of replacing the semaphore instance.
- **Translation state safety**: Translation API usage aggregation and task-state writes now run under the translation state lock. Concurrent translation batches now publish `translation_running_batches` instead of overwriting the single stale `translation_current_batch` field.
- **Polish safety**: Disabled DeepSeek polish no longer requires an API key. Batch polish is covered for both English transcripts (polishing Chinese translations while preserving quoted English original) and Chinese transcripts (polishing original Chinese text).
- **Conflict boundary**: A translating English file and a Chinese-only polish job can run at the same time; both contend only for the shared DeepSeek limiter. They do not share per-task translation state. Polish usage writes are protected with a process/thread file lock around `work/state/deepseek_polish_usage.json`.
- **Validation**: run `python -m py_compile scripts/transcribe_podcasts.py scripts/polish_interview_markdown.py`, `python scripts/quick_validate.py`, `python scripts/transcribe_podcasts.py --dry-run`, and `python -m pytest tests`.
- **Risk to watch**: `config.json` is intentionally ignored because it may contain a real API key; do not force-add it. Keep future default changes in `config.example.json` and GUI config write paths.

### 2026-05-20 DeepSeek Translation & Polish Throughput Optimization

- **DeepSeek Translation Concurrency & Batching**:
  - Implemented `ThreadPoolExecutor` for parallel translation batches (up to `max_parallel_translations=3` workers).
  - Optimally tuned default configuration parameters: `batch_segments` increased to 48, `max_batch_chars` to 18,000 for translation; `batch_blocks` increased to 8, `max_batch_chars` to 12,000 for markdown polish.
- **DeepSeek API Concurrency Semaphore Limit**:
  - Introduced a global `DEEPSEEK_API_SEMAPHORE` (configured via `pipeline.max_deepseek_api_requests`, defaulting to `4`) to prevent HTTP 429 Rate Limit errors by limiting active DeepSeek API connections globally across both translation and polish phases.
  - Dynamically injected the global semaphore from the orchestrator `transcribe_podcasts.py` into the modularly-loaded `polish_interview_markdown.py`.
- **DeepSeek Resilience & Error Handling**:
  - Refactored `deepseek_chat_completion` with retry mechanisms: parses HTTP 429 `Retry-After` headers for smart backoff, fallback standard delays, and auto-throttles global concurrency to `1` when repeated 429s are encountered.
  - Detects `finish_reason == "length"` indicating truncation, raising a specific `DeepSeekLengthTruncatedError` to trigger fallback mechanisms.
- **Batch Polish with Fallback**:
  - Implemented `polish_blocks_batch()` using DeepSeek JSON response format to batch polish up to 8 blocks per API request.
  - Seamlessly fallbacks to single block-by-block polish if JSON structure parsing fails.
- **Usage Record Thread-Safety**:
  - Secured `record_usage` and `record_deepseek_polish_usage` with thread reentrant locks (`threading.Lock`) to prevent corruption during parallel writes.

### 2026-05-19 Moved-Path Shortcut and Branch Cleanup

- **Historical checkout retired**: Podcast Transcriber now lives under `tools\podcast-transcriber` inside ImmersiveReader.
- **Historical shortcut retired**: ImmersiveReader launches the managed Python sidecar from its internal `runtime\podcast` directory.
- **Branch consolidation**: Local development branch is now only `main` plus `origin/main`. Deleted confusing local branches after comparison:
  - `codex/open-source-snapshot` was an exact duplicate of `main` at `345f05f6416db3af182a7331d3f8e35524124441`.
  - `master` pointed at old orphan/open-source prep commit `55b7cff54cd02de474568a734acf73fa293a0c75`; merging it into `main` would have reverted current app/icon/code state.
  - `codex/recovered-two-stage-stash` pointed at old recovered stash commit `ac443e480f4eb6c47096df89597798d19a522748`; merging it into `main` would have reintroduced stale code and removed the current icon file.
- **Current rule**: Make future edits on `main`. If an old branch state is ever needed, recover it by the commit IDs above instead of recreating multiple active local branches.

### 2026-05-18 README Rewrite and Icon Generation

- **Icon Generation**: Created a beautiful, modern, and transparent icon (`assets/icon.png`) using an AI image generation model, matching the project's glassmorphism and modern UI aesthetic. Removed the fake transparency grid and generated an `assets/icon.ico` file for Windows shortcuts.
- **System Tray Icon**: Updated `scripts/tray_icon.ps1` to use the generated `assets/icon.ico` instead of the default application icon, giving the app a branded system tray appearance when minimized.
- **README Update**: Rewrote `README.md` to be engaging, user-friendly, and visually appealing, incorporating the new icon. Added clear "Quick Start" steps and a dedicated section for "DeepSeek" advanced configuration.

### 2026-05-18 Four-Bug Fix and UI Overhaul

- **Bug 1 — DeepSeek translation not starting**: `preflight_checks()` called `maybe_start_ollama()` which used `effective_provider_name()` that raises `RuntimeError` when DeepSeek is selected. Changed to `provider_name()` for Ollama-need check, and `preflight_checks` now catches RuntimeError as warning instead of fatal crash.
- **Bug 2 — Parallelism verification**: `ThreadPoolExecutor(max_workers=3)` is correct. Whisper inference is serialized by `MODEL_TRANSCRIBE_LOCK` (RTX 4060 8GB GPU limitation). File-level parallelism covers preparation/chunking/translation/output writing. No code bug — hardware-correct behavior.
- **Bug 3 — Log window showing placeholder**: `process_source_with_fallback` now writes `log_path` into state JSON early before `process_file()` begins. Frontend `renderLogs` now shows contextual status hints when log is empty instead of generic "等待启动".
- **Bug 4 — API key disappearing on backend switch**: Backend `/api/config` GET now returns `masked_api_key` (e.g. `sk-****3b9`). Frontend preserves masked key in input, supports show/hide toggle, clears on focus for paste, restores masked key on blur.
- **UI Overhaul**: Inter font, glassmorphism cards/panels with `backdrop-filter: blur()`, gradient background, gradient accent buttons/progress bars, card hover lift animation, fade-in app animation, gradient top border on log panel.

---

## Current Architecture (v2 Refactored)

### What changed
Deep refactoring following First Principles + Occam's Razor:
- **`run_with_gui.py`**: 3569 → ~530 lines. Deleted: SQLite DB, scanner, heartbeat monitor, 14-phase state machine, lifecycle logging, event store. Added: config read/write API for translation/polish backend selection.
- **`podcast-transcriber-v2.html`**: 2084 → ~640 lines. Kept CSS theme system, rewrote JS to simple poll→render. Added: collapsible settings panel for translation/polish backend (ollama vs deepseek vs none).
- **`transcribe_podcasts.py`**: Removed DB claim loop (`job_store.claim_next_queued_job`), restored direct file discovery + ThreadPoolExecutor. `_transition_job_db` and `_job_cancelled_or_deleted` stubbed as no-ops. Cleaned dead `job_store`/`DB_PATH` references from `update_task_state` and `touch_task_heartbeat`.
- **Deleted files**: `job_store.py`, `lifecycle_verification.py`, `_gui_lifecycle_test*.py`, `_lifecycle_smoke.ps1`, `_smoke_test.ps1`, `_minimal_lifecycle_test.py`, `gui_smoke_test.py`, `run_with_gui_tkinter.py`, `tests/test_job_store.py`, `tests/test_repair.py`

### Data Flow
```
input/*.mp3,m4a,wav
  → transcribe_podcasts.py discovers files, processes in parallel
  → writes state to work/state/<fingerprint>.json
  → writes output to output/<stem>.md

run_with_gui.py
  → starts HTTP server on :8765
  → reads work/state/*.json for /api/snapshot
  → GET /api/config returns translation/polish backend settings
  → POST /api/config updates config.json (translation_backend, polish_backend, etc.)
  → spawns transcribe_podcasts.py as subprocess
  → serves podcast-transcriber-v2.html
```

### Settings UI
The frontend now has a collapsible settings panel (⚙ button in header) that allows switching:
- **Translation backend**: `ollama` (local), `deepseek` (API), `none` (disabled)
- **Polish backend**: `ollama` (local), `deepseek` (API)

Changes are written directly to `config.json` and take effect on the next worker run.

### 2026-05-18 Backend and Markdown Fix

- Effective backend resolution is shared between GUI config reporting and worker startup. If `translation.backend` or `markdown.llm_polish.backend` is `deepseek` and an API entry is configured, shortcut launches keep using DeepSeek and skip Ollama preflight unless another enabled stage still uses Ollama.
- If DeepSeek is selected but the API entry/key is missing, the worker now fails with a clear API configuration error instead of silently falling back to Ollama.
- Default pipeline concurrency is now file-level transcription `3` and translation `3`. Whisper inference remains serialized by `MODEL_TRANSCRIBE_LOCK`; final Markdown postprocess remains `1`.
- Chinese final Markdown generation no longer fails just because speaker inference finds fewer than two roles. That quality gate still applies to English interview-style output where missing speaker separation is more likely to break the bilingual transcript.
- Existing `output/` semantics remain unchanged: only final readable Markdown belongs there; intermediate JSON/SRT/raw Markdown remain under `work/internal/`.

### Verification Commands
```powershell
# Syntax check
.\.venv\Scripts\python.exe -m py_compile scripts\run_with_gui.py
.\.venv\Scripts\python.exe -m py_compile scripts\transcribe_podcasts.py
.\.venv\Scripts\python.exe -m py_compile scripts\polish_interview_markdown.py

# Structure check
.\.venv\Scripts\python.exe scripts\quick_validate.py

# Dry run (no model load)
.\.venv\Scripts\python.exe scripts\transcribe_podcasts.py --dry-run

# Focused Markdown regression tests
.\.venv\Scripts\python.exe -m pytest tests\test_polish_interview_markdown.py -q

# Launch GUI
.\.venv\Scripts\python.exe scripts\run_with_gui.py

# Verify final Markdown generation from an existing segments JSON
.\.venv\Scripts\python.exe scripts\polish_interview_markdown.py <work\internal\json\file.segments.json> --final-only
```

### Risks
1. **No cancel support**: Since `_job_cancelled_or_deleted` is a no-op, the "stop" button in GUI kills the entire worker process. Individual task cancellation is not supported.
2. **State file cleanup**: After successful runs, `cleanup_work_artifacts()` removes state files. If interrupted, stale state files may persist.
3. **Config write safety**: `save_config()` now uses `CONFIG_LOCK` + atomic write (tmp + os.replace) for concurrent safety.
4. **API key hygiene**: do not commit real API keys. GUI saves the local key into `config.json`; before committing, verify `translation.api_key` and `markdown.llm_polish.api_key` are empty or intentionally unstaged.
5. **Manual worker start required**: GUI shortcut launch is intentionally idle even when `input/` contains audio. Users must click "启动" to begin processing.
6. **Stop is destructive by design**: clicking "停止" clears current outputs and intermediate state so the next run is treated as a fresh task. Do not use Stop if preserving partial work matters.
7. **WebView dependency**: the unified native window path requires `pywebview` from `requirements.txt`; if it is missing or WebView2 is unavailable, launch falls back to browser app-mode and tray reopen cannot preserve a live native window instance.
