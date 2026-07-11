# REVIEW_REPORT

Date: 2026-04-22

## Summary of findings by severity
- High: 2
  - Untrusted HTML execution risk in markdown rendering pipeline (fixed).
  - Dependency advisories in JS/Rust trees with high-severity items in baseline (partially fixed).
- Medium: 2
  - Non-atomic state transition on file open failure (fixed).
  - Case-sensitive markdown extension checks (fixed).
- Low: 0 fix-target findings in code; dependency residuals remain in audit outputs (deferred).

## What was fixed
1. Markdown HTML sanitization hardening
- Status: Fixed
- Scope: `src/lib/render/markdown.ts` + new tests `src/lib/render/markdown.test.ts`
- Result: malicious HTML payloads are sanitized before `{@html}` render while required app attributes remain available.

2. Atomic file-open failure path
- Status: Fixed
- Scope: `src/routes/+page.svelte`
- Result: file path/name/content state now commits only after a successful read; failed reads preserve prior active document state.

3. Case-insensitive markdown extension handling
- Status: Fixed
- Scope: `src-tauri/src/lib.rs`, `src/routes/+page.svelte`
- Result: mixed/upper-case markdown extensions are accepted consistently in backend checks and drag-drop filtering.

4. Dependency upgrades for critical/high path
- Status: Fixed for high/critical target, with deferred non-high/upstream advisories.
- Scope: `package.json`, `package-lock.json`, `src-tauri/Cargo.lock`
- Result:
  - `npm audit` now reports `high: 0`, `critical: 0`.
  - `cargo audit` high-severity `rustls-webpki` path was eliminated; remaining vulnerabilities are medium `tar` advisories via `tauri-plugin-updater` transitive path.

## What was deferred
- Rust medium advisories:
  - `RUSTSEC-2026-0067` and `RUSTSEC-2026-0068` in `tar 0.4.44` via `tauri-plugin-updater 2.10.1`.
- Rust warnings from transitive platform stack (unmaintained/unsound/yanked) in Tauri GTK/WebKit dependency chain.
- NPM low advisories:
  - `cookie` (via `@sveltejs/kit` / `@sveltejs/adapter-static`).

## Needs human decision
- Whether to temporarily disable or replace updater-related functionality to force removal of `tar` transitive risk now, versus waiting for upstream `tauri-plugin-updater` release to consume `tar >= 0.4.45`.
- Whether to accept Tauri GTK/WebKit transitive warning profile for current release target platforms.

## Test count and coverage delta
- Baseline:
  - JS tests: none configured.
  - Rust tests: 0.
- Current:
  - JS tests: Vitest configured, 1 test file, 3 tests passing.
  - Rust tests: 2 unit tests passing in `src-tauri/src/lib.rs`.
- Coverage:
  - No coverage tooling configured at baseline or after fixes; numerical coverage delta unavailable.

## Lint/type-check delta vs baseline
- Lint:
  - Baseline: not configured.
  - Current: unchanged (not configured).
- Type check:
  - Baseline: `npm run check` -> 0 errors, 0 warnings.
  - Current: unchanged -> 0 errors, 0 warnings.

## Validation results (final)
- `npm test`: pass (3/3 tests)
- `npm run check`: pass (0 errors, 0 warnings)
- `cargo test`: pass (2 unit tests)
- `cargo check`: pass
- `npm audit --json`: `high: 0`, `critical: 0` (3 low remain)
- `cargo audit`: 2 vulnerabilities remain (both medium) + transitive warnings

## Risks introduced and staging verification
- Markdown sanitization may strip previously-rendered raw HTML constructs that were not explicitly allowed.
- Verify in staging with a representative markdown corpus:
  1. Confirm headings/TOC anchors and inline editing source-position behavior still work.
  2. Confirm fenced code highlighting and language classes still render correctly.
  3. Confirm malicious payloads (script/event-handler/javascript URL cases) render inert.
- File-open path changes should be verified with forced read failures:
  1. Open a valid file, then attempt opening a missing/denied file.
  2. Confirm previous content/path UI remains consistent and save operations still target the last successful file.