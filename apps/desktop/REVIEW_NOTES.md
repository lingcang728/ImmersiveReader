# REVIEW_NOTES

## Phase 1 — Reconnaissance (Read-Only Baseline)

### 1) Repo structure (top 3 levels)

```text
MMbook/
├─ .claude/
├─ .github/
│  └─ workflows/
├─ .vscode/
├─ src/
│  ├─ lib/
│  │  ├─ render/
│  │  ├─ stores/
│  │  └─ theme/
│  └─ routes/
├─ src-tauri/
│  ├─ capabilities/
│  ├─ gen/
│  │  └─ schemas/
│  ├─ icons/
│  │  ├─ android/
│  │  └─ ios/
│  ├─ keys/
│  └─ src/
└─ static/
```

- Languages: TypeScript + Svelte 5 (frontend), Rust (backend), JSON/TOML/YAML config.
- Framework/runtime: SvelteKit SPA + Tauri v2 desktop shell.
- Build system/package managers:
  - Frontend: npm + Vite + SvelteKit (`package.json`).
  - Backend: Cargo (`src-tauri/Cargo.toml`).

#### Entry points
- Frontend entry surface:
  - `src/routes/+page.svelte` (single-page UI and interaction logic)
  - `src/routes/+layout.ts` sets `ssr = false`
- Backend entry surface:
  - `src-tauri/src/main.rs` -> `mmbook_lib::run()`
  - `src-tauri/src/lib.rs` builds Tauri app and registers commands

#### Tests
- JS test framework: none configured (`npm test` -> missing script).
- Rust tests: default cargo test harness available, but no custom tests existed initially.
- How to run:
  - JS: n/a (no script)
  - Rust: `cd src-tauri && cargo test`

#### Lint / type-check / format tooling
- Type check: `npm run check` (`svelte-check`) configured.
- Lint: none configured (`npm run lint` -> missing script).
- Formatter: none explicitly configured in repo scripts.

### 2) Docs review summary (README / CLAUDE / AGENTS / CONTRIBUTING / docs / ADR)

Reviewed files:
- `README.md`
- `CLAUDE.md`
- `AGENTS.md`

Not found in repo root: `CONTRIBUTING*`, `docs/`, ADR files.

Project purpose summary (3–5 sentences):
MMbook is a desktop Markdown reader focused on immersive reading and light editing. It uses a Svelte 5 frontend and Rust/Tauri backend for native packaging, file I/O, and persisted reading state. The UI is intentionally concentrated in a single Svelte page component and emphasizes focus mode, TOC navigation, search highlighting, theme switching, and inline edits. Markdown rendering is done through unified/remark/rehype and code highlighting via Shiki. The app supports `.md`/`.markdown` file association and opening through window args/events.

### 3) High-level module/dependency graph

#### Frontend mental model (TypeScript/Svelte)
- **Core domain behavior**
  - `src/routes/+page.svelte`
    - file open/load/save flow
    - inline paragraph editing and source patching
    - focus-mode block detection/navigation
    - search highlight navigation
    - TOC display/jump
- **Rendering pipeline**
  - `src/lib/render/markdown.ts`
    - markdown parse + GFM + rehype stringify
    - code block replacement with Shiki
    - heading ID generation + TOC extraction
- **Global state/theming**
  - `src/lib/stores/app.ts` (Svelte stores)
  - `src/lib/theme/themes.ts` (theme definitions + CSS var application)
- **Glue/integration**
  - Tauri JS APIs: invoke/events/dialog/updater

#### Backend mental model (Rust/Tauri)
- **Core domain behavior**
  - `src-tauri/src/lib.rs`
    - markdown read/save commands with multi-encoding support
    - reading state load/save command pair
- **Infrastructure**
  - Tauri builder/plugins (dialog/fs/opener/updater)
  - app data state directory management
- **Glue/integration**
  - startup arg handling for initial file open
  - macOS opened-file event forwarding to frontend

#### FFI/IPC boundary (explicit map)
Frontend calls Rust commands via Tauri `invoke`:
- `read_markdown_file(path)` -> `{ content, encoding }`
- `save_markdown_file(path, content, encoding)` -> `()`
- `load_reading_state(path)` -> `{ scroll_position, bookmarks }`
- `save_reading_state(path, state)` -> `()`

Backend emits event to frontend:
- `open-file` (macOS open-with flow)

### 4) Test baseline

Commands executed and results:
- `npm test` -> **fails** (missing script)
- `cd src-tauri && cargo test` -> **pass**, 0 tests executed

Coverage:
- No coverage tooling configured; no baseline coverage metric available.

Flaky tests:
- None observed (no actual test suite present except empty Rust harness).

### 5) Lint/type-check baseline

Commands executed and results:
- `npm run lint` -> **fails** (missing script)
- `npm run check` -> **pass**, `svelte-check found 0 errors and 0 warnings`
- `cd src-tauri && cargo check` -> **pass**

### Dependency security baseline

- `npm audit --json`: reports high vulnerabilities in dependency tree including `vite` and `@sveltejs/kit` chain.
- `cd src-tauri && cargo audit`: reports multiple advisories including vulnerabilities in transitive updater-related path.

---

## ISSUES

### High

1) **Untrusted HTML can execute in rendered Markdown**
- File/line evidence:
  - `src/lib/render/markdown.ts:57` (`remarkRehype(... allowDangerousHtml: true)`)
  - `src/lib/render/markdown.ts:59` (`rehypeStringify(... allowDangerousHtml: true)`)
  - `src/routes/+page.svelte:1292` (`{@html $renderedHtml}`)
  - `src-tauri/tauri.conf.json:25` (`"csp": null`)
- Category: Security (XSS/script execution risk in desktop webview context).
- Severity: High.
- Proposed fix: Add HTML sanitization stage to renderer output with explicit allow-list that preserves required app attributes (e.g., heading IDs, source position data attributes used by inline editing).

2) **Dependency vulnerabilities (JS + Rust trees) include high severity items**
- File evidence:
  - `package.json`, `package-lock.json` resolved to vulnerable ranges (via `npm audit`).
  - `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` vulnerable transitive paths (via `cargo audit`).
- Category: Security.
- Severity: High.
- Proposed fix: Minimal direct dependency upgrades targeting critical/high findings only, then re-audit and record remaining deferred advisories.

### Medium

1) **Failed file-open can corrupt visible state model**
- File/line evidence:
  - `src/routes/+page.svelte:349-350` sets `$currentFilePath`/`fileName` before read succeeds.
  - `src/routes/+page.svelte:376-378` on read failure only logs; previous rendered content may remain while path points to failed target.
- Category: Correctness / API contract consistency.
- Severity: Medium.
- Proposed fix: Make open-file transition atomic: commit path/name/content only after successful read/render; preserve previous state on failure.

2) **Markdown extension checks are case-sensitive**
- File/line evidence:
  - `src-tauri/src/lib.rs:100-102` (`ends_with(".md") || ends_with(".markdown")`)
  - `src/routes/+page.svelte:284` drag-drop filter checks lower-case suffixes only.
- Category: Correctness.
- Severity: Medium.
- Proposed fix: Normalize extension case before checks in both frontend and backend; add Rust unit tests for mixed/upper-case extensions.

### Low

- No high-confidence low-severity style-only/code-smell-only findings recorded as fix targets in this pass (out of scope unless tied to correctness/security).

## Phase 3 pre-fix ordering (severity then blast radius)

1. High: Markdown sanitization (+ tests, includes Vitest introduction)
2. Medium: Failed-open atomicity
3. Medium: Case-insensitive extension checks (+ Rust tests)
4. High: Dependency upgrades for critical/high advisories

## Working tree note

- Baseline environment already had:
  - `src-tauri/Cargo.lock` modified (`mmbook` lock entry 0.1.0 -> 0.1.1)
  - untracked `AGENTS.md`
- These were preserved as requested.