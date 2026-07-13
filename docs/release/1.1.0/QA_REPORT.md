# ImmersiveReader 1.1.0 QA Report

## Automated and local QA

- `scripts/verify.ps1`: passed on the 1.1.0 tree: contracts 5, desktop Vitest 38, Svelte 0 errors/0 warnings, Rust 88, Zhihu 25, Podcast 27, and Podcast quick validation.
- Bookshelf detail: Playwright isolated mock QA passed; source link, provenance revision, task status/revision, and detail screenshot were verified.
- Migration preview: read-only deterministic Rust preview test passed; no target data was created.
- Podcast short samples: two real 30-second clips completed locally; one Chinese transcription and one English transcription plus local Ollama translation succeeded, with no paid API request. Evidence: `.omo/ulw-loop/evidence/podcast-short-qa-20260713.md`.

## Isolated Zhihu QA

The isolated run for `xiao-xue-shi-46-24` used `ImmersiveReader-QA-zhihu-v3-20260713`. The managed profile had no login session; both target routes returned a 404/empty index with `logged=false`. The merged Top 5 result was `0`, no task or Library output was written, CAPTCHA was not reached, and publish was not attempted. The result is a human login/target-reachability blocker, not a success claim.

## Authorization boundaries

Full original-audio execution and possible API charges were not performed. Real production data migration, deletion of legacy frontends, formal `ship:local`, Markdown association changes, and remote history rewriting were not performed.

## Installed artifact

The isolated development EXE was built by `ship:dev` at `2026-07-13 13:41:30`, size `19143680` bytes, SHA-256 `5350B8E9DF2EBFD5253F13A12B33335A9D39E8678A2C2E662AA5F7898BCB1602`; Markdown associations were not registered. The exact EXE started with title `沉浸阅读 · 开发版`. Using the uniquely attributable `沉浸阅读 Dev` tray icon, UI automation verified hide, restore, and `退出（保留任务）`; the process and development WebView child were both gone after exit. The separate CLI `CloseMainWindow()` path is not used as tray-menu evidence.
