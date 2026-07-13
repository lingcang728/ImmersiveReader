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

The isolated development EXE was built by `ship:dev` at `2026-07-13 11:16:37`, size `19151872` bytes, SHA-256 `C302561A94048DA27C8937E4F5D23036BCE5A75093BE16988F2D2D1D748E4F97`; Markdown associations were not registered. The exact EXE started and stopped; its WebView child was gone afterward. A CLI close request kept the process alive, but the native visibility check did not prove the expected hidden state, so tray hide/restore remains open in `V3-TODO.md`.
