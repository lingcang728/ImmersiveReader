# ImmersiveReader 1.1.0 QA Report

## Automated and local QA

- `scripts/verify.ps1`: final result is recorded after the release-version build; it covers contracts, desktop tests/Svelte/Rust, Zhihu tests/build/reader compile, and Podcast tests/quick validation.
- Bookshelf detail: Playwright isolated mock QA passed; source link, provenance revision, task status/revision, and detail screenshot were verified.
- Migration preview: read-only deterministic Rust preview test passed; no target data was created.
- Podcast short samples: two real 30-second clips completed locally; one Chinese transcription and one English transcription plus local Ollama translation succeeded, with no paid API request. Evidence: `.omo/ulw-loop/evidence/podcast-short-qa-20260713.md`.

## Isolated Zhihu QA

The isolated run for `xiao-xue-shi-46-24` used `ImmersiveReader-QA-zhihu-v3-20260713`. The managed profile had no login session; both target routes returned a 404/empty index with `logged=false`. The merged Top 5 result was `0`, no task or Library output was written, CAPTCHA was not reached, and publish was not attempted. The result is a human login/target-reachability blocker, not a success claim.

## Authorization boundaries

Full original-audio execution and possible API charges were not performed. Real production data migration, deletion of legacy frontends, formal `ship:local`, Markdown association changes, and remote history rewriting were not performed.

## Installed artifact

The final isolated development EXE timestamp, SHA-256, and process smoke result are recorded in `release-manifest.json` after `ship:dev`. Production installation remains a separate gate.
