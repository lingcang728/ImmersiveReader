# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

沉浸阅读 (ImmersiveReader) is a local-first Windows desktop app for long-form reading. It unifies three content sources — Zhihu archives, podcast transcriptions, and Markdown folders — into a single local Library. Current release: 1.1.0.

It is a polyglot monorepo with **no root `package.json` and no npm workspaces** — each package is built and tested independently and stitched together by the Rust backend at runtime.

## Common commands

All commands assume PowerShell from the repo root. Prefer `npm.cmd` / `cargo.exe` explicit forms in scripts (the verify harness does).

### Development

```powershell
.\scripts\start.ps1 desktop          # tauri dev (sets IMMERSIVE_RUNTIME_ROOT, dev URL http://localhost:1420)
```

### Verify (run for any cross-package or shared-contract change)

```powershell
.\scripts\verify.ps1                 # runs contracts + desktop (TS + Rust) + zhihu + podcast checks
```

`verify.ps1` also asserts no source file references retired legacy project paths (`MMbook`, `Zhihu_packer`, `PodcastTranscriber`) and that dependency dirs are not junctions.

### Production install (required for desktop UI/Rust/Tauri/capability/install changes)

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
```

`ship:local` builds the NSIS installer, installs it, and refreshes the Markdown handler (without forging `UserChoice`). After installing, report the commit, production EXE timestamp, and SHA-256. Documentation-only changes do not require a new install.

### Per-package commands

| Package | Test | Typecheck/Build |
|---|---|---|
| `apps/desktop` (frontend) | `npm.cmd --prefix .\apps\desktop test` (vitest run) | `npm.cmd --prefix .\apps\desktop run check` (svelte-check) |
| `apps/desktop/src-tauri` (Rust) | `cargo.exe test --manifest-path .\apps\desktop\src-tauri\Cargo.toml` | `cargo.exe check --manifest-path .\apps\desktop\src-tauri\Cargo.toml` |
| `packages/contracts` | `node --test tests/*.test.ts` | `tsc -p tsconfig.json` |
| `tools/zhihu-packer` | `npm.cmd --prefix .\tools\zhihu-packer test` | `npm.cmd --prefix .\tools\zhihu-packer run build` (+ `run compile-reader`) |
| `tools/podcast-transcriber` | `pytest -q` (uses managed `runtime\podcast\python\python.exe` via `Get-PodcastPython`) | `python scripts\quick_validate.py` |

### Running a single test

- Vitest: `npm.cmd --prefix .\apps\desktop test -- src/lib/focus/scroll.test.ts`
- Rust: `cargo.exe test --manifest-path .\apps\desktop\src-tauri\Cargo.toml recent_cleanup_keeps_existing_files_only`
- Node test runner: `node --test packages/contracts/tests/contracts.test.ts`
- Pytest: `pytest tools/podcast-transcriber/tests/test_transcribe_task.py::test_name`

## Architecture

### Package layout

- **`apps/desktop`** — the product. Svelte 5 + SvelteKit (SPA, `ssr = false`, `adapter-static` with `index.html` fallback) on a Tauri 2 shell. Frontend dev port `1420`.
- **`apps/desktop/src-tauri`** — the Rust backend. This is the system's core, not a thin IPC layer.
- **`tools/zhihu-packer`** — TypeScript/Node archival tool. Runs as a managed sidecar (Express server on a sidecar HTTP protocol).
- **`tools/podcast-transcriber`** — Python transcription tool (faster-whisper + DeepSeek). Runs as a managed worker.
- **`packages/contracts`** — standalone TypeScript schema + validators. See "Shared contracts" below.
- **`runtime/`** — vendored, gitignored production runtimes (Node, Chromium/msedge, Python, FFmpeg, Whisper models). Described by `runtime/manifest.json` (path + bytes + SHA-256 per artifact). Located at runtime via `IMMERSIVE_RUNTIME_ROOT`.

### The Rust backend is the core

`apps/desktop/src-tauri/src/lib.rs` registers all `#[tauri::command]`s and wires the `tauri::Builder`. The backend owns:

- **Library** (`library.rs`, `importer.rs`, `trash/`) — book manifests, chapter content, reading progress, soft-delete trash with idempotent restore/delete.
- **Control DB** (`control/`, `tasks.rs`) — SQLite (`rusqlite` bundled) at `Data\App\control.db`. Tracks acquisition tasks, events, idempotent command claims (`claim_command`/`complete_command`), migration runs, stale-engine recovery.
- **Tools** (`tools/`, `zhihu.rs`, `podcast/`) — launches and supervises the Zhihu/Podcast sidecars as managed child processes. On Windows, child processes are tied to the app lifetime via **Job Objects** (`job_object.rs`); the `tools/tool_manager.rs` tracks process health and `tools/sidecar_http.rs` speaks the sidecar protocol.
- **Reader server** (`reader_server.rs`, `reader_http.rs`) — a `tiny_http` server on `127.0.0.1` that serves the Zhihu reader template for 连读 (continuous reading). This is a *separate* reading surface from the 精读 (close reading) Svelte `ReaderWorkspace` — both share the same reading semantics, do not create a second reading core.
- **Storage & secrets** (`storage.rs`, `secrets.rs`, `settings.rs`) — canonical storage roots under `%LOCALAPPDATA%\ImmersiveReader`; DeepSeek API keys live in Windows Credential Manager, never on disk.
- **Migration & publish** (`migration/`, `publish/`) — legacy-location migration with preview/reconciliation; publish transactions with commit/rollback.
- **Atomic writes** — all persistent writes go through `atomic_file.rs`.

Storage roots (computed by `storage::StorageLocations::current()`, library root overridable via settings): `Data\` (control.db, settings.json), `Cache\`, `Logs\`, `Backups\`, runtime state. Default Library: `Documents\沉浸阅读\Library`.

### Shared contracts

`packages/contracts` is **not a dependency of the desktop app** — the frontend does not import it. It is a standalone spec library. The actual shared contract between Rust and TypeScript is the **JSON schema + fixtures**:

- `packages/contracts/fixtures/manifest.valid.json` and `reading.valid.json` are the source of truth.
- Rust `contracts.rs` `include_str!`s these fixtures and validates against them.
- TS `packages/contracts/src/index.ts` validates the same shape.

When you change `BookManifest` / `ReadingState` / `Chapter` fields, update **both** `contracts.rs` and `packages/contracts/src/index.ts`, plus the fixtures, and run `verify.ps1`. `is_safe_relative_path` (Rust) and `requireRelativePath` (TS) must stay in lockstep — chapter paths must be forward-slash relative paths with no `..`/`\`/drive prefixes.

### Frontend structure

Single SPA route (`src/routes/+page.svelte`) orchestrates the whole app; logic lives in `src/lib/`:

- `components/` — `Bookshelf` (single entry workbench), `ReaderWorkspace`, `ZhihuWorkflow`, `PodcastWorkflow`, `TrashPanel`, `SettingsPanel`, `WindowChrome` (custom decorations — `decorations: false` in tauri.conf), `WorkflowDialogShell`.
- `focus/` — Focus Mode: `segment.ts` (sentence splitting), `scroll.ts` (viewport-anchor restore). **Locked behavior — see below.**
- `render/` — Markdown pipeline (`unified` + remark/rehype, KaTeX, Shiki, Mermaid) with a web worker (`markdown.worker.ts`).
- `stores/app.ts` — central Svelte stores (theme, font scale, focus mode, current file, task sync).
- `tasks/sync.ts` — consumes `acquisition://task-event` and `TaskSnapshot` events from the backend.
- `theme/themes.ts` — the only source of theme variables. MMbook monochrome palette; link blue (`--link`) is used for focus rings, status, secondary actions, progress. No gold/copper/purple gradients.

## Locked behaviors — do not regress

From `DESIGN.md` and `AGENTS.md`:

- **Focus Mode** visuals (spotlight, progressive blur, particles, windowed style) and the **viewport-anchor** scroll restoration on focus toggle / font reflow. Do not reuse raw `scrollTop`; long jumps use instant scroll, only short distances smooth-scroll.
- All file access must go through controlled Rust `#[tauri::command]`s — the frontend does not touch the filesystem directly.
- External links must be explicit `http://`/`https://` only.
- Keep production and QA data roots separate. **Never** use a real user database, profile, credential, audio, or output as a test fixture.
- Markdown `.md`/`.markdown` association: `ship:local` refreshes the `ImmersiveReader.Markdown` handler so the latest build opens `.md`/`.markdown`, but the protected Windows `UserChoice` hash is never forged - the final default-app choice stays a Windows UI gate (`register:markdown` opens Default Apps for first-time confirmation).

## Conventions

- UI copy is Chinese; code identifiers are English.
- One logical change per commit, with verification evidence in the commit or the release QA report (`docs/release/<version>/QA_REPORT.md`).
- Never commit Library content, browser profiles, databases, `config.json`, `.env`, credentials, models, inputs, outputs, caches, or logs (see `.gitignore`).

## Runtime & tooling reuse

Per the machine's global rules: before installing any tool, CLI, browser, SDK, or runtime, first search the machine for an existing usable install (`where.exe`, `npm root -g`, `pip show`, `PLAYWRIGHT_BROWSERS_PATH`, etc.). Reuse the managed `runtime/` and the global Playwright installation rather than installing second copies into the repo. `runtime/` is gitignored and provisioned by `scripts/prepare-runtime.ps1`.
