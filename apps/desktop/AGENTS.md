# AGENTS.md

This file provides guidance to agents working in `apps/desktop`.

## Build & Development Commands

```bash
npm install                  # Install frontend dependencies
npm run tauri dev            # Start dev server (Vite + Tauri)
npm run tauri build          # Production build only (not enough for user-facing install)
npm run ship:local           # REQUIRED after app changes: build + install + shortcuts + .md associations
npm run install:latest       # Install newest existing NSIS without rebuilding
npm run check                # TypeScript/Svelte type checking
npm run test                 # Vitest unit tests
npm run dev                  # Frontend-only (no Tauri shell)
```

Rust backend is in `src-tauri/`. Direct: `cd src-tauri && cargo build` / `cargo test` / `cargo check`.

## Mandatory close-out after every change

**The user runs the product only via the installed EXE** (desktop shortcut or double-clicking `.md` / `.markdown`). Dev server is not their daily path.

Whenever this task changes app code, UI, Rust, Tauri config, themes, install scripts, or capabilities:

### 1. Git commit (always)

Unless the user explicitly says not to commit:

1. `git status` / review the diff.
2. Stage only intentional project files (never Library, secrets, `*.exe` if gitignored, local logs).
3. Create a concise commit with a clear message.
4. Report the commit hash.

Every completed change set must be one commit (or a small stack of commits) so the user can roll back with `git log` / `git checkout`.

### 2. Ship local package (always for binary-affecting work)

```powershell
# From apps/desktop OR monorepo root:
npm.cmd --prefix .\apps\desktop run ship:local
```

`ship:local` must:

- Build NSIS (`tauri build --no-sign --bundles nsis`)
- Silent-install into monorepo root:  
  `C:\Users\15pro\Desktop\MyProject\ImmersiveReader\immersive-reader.exe`  
  (not `%LOCALAPPDATA%`)
- Refresh Desktop + Start Menu shortcuts named `沉浸阅读`
- Register `.md` / `.markdown` → that EXE (`-RegisterMarkdownAssociations`)

Then verify:

```powershell
$exe = "C:\Users\15pro\Desktop\MyProject\ImmersiveReader\immersive-reader.exe"
(Get-Item $exe).LastWriteTime
(Get-FileHash $exe -Algorithm SHA256).Hash
(Get-Item 'HKCU:\Software\Classes\ImmersiveReader.Markdown\shell\open\command').GetValue('')
```

**Stopping after `npm run tauri build` without install is a task failure.** The user will still open the old installed binary.

If only non-binary docs in this folder change, commit is still required; ship can be skipped only when the installed EXE cannot change.

## Architecture (short)

**Tauri v2 + SvelteKit** (adapter-static SPA). Main UI: `src/routes/+page.svelte` + bookshelf components. Library/progress/reader server in `src-tauri/src/`. Themes: `src/lib/theme/themes.ts` CSS variables.

## Key conventions

- UI copy in Chinese; code identifiers in English.
- Reading/focus visuals: do not change locked spotlight algorithms unless asked.
- Theme colors via CSS variables from the theme system for reading surfaces.
- Install target is monorepo root; keep Markdown associations on that path after ship.
