# ImmersiveReader repository rules

- This repository is the integration target for the Markdown reader, Podcast and Zhihu tools. Imported source history is preserved in Git; the standalone Zhihu archive is private and archived at `https://github.com/lingcang728/Zhihu_packer`.
- Never commit Library content, browser profiles, databases, local configuration, API credentials, models, inputs, outputs, caches or logs.
- Reuse installed Windows runtimes and the global Playwright installation before installing anything.
- Keep the MMbook focus-mode visuals and viewport-anchor behavior unchanged.
- Run `scripts\verify.ps1` when a change spans multiple packages or touches shared contracts.

## Change close-out

- Make one logical change per commit and keep its verification evidence in the same commit or the release QA report.
- Any desktop app, UI, Rust backend, Tauri configuration, capability or install-script change requires a commit and:

  ```powershell
  npm.cmd --prefix .\apps\desktop run ship:dev
  ```

  Report the commit, development EXE timestamp and SHA-256. Documentation-only changes do not require a new install.
- `ship:local` is a production-install gate. Markdown association registration is a separate Windows UI gate.
- Keep production, development and QA data roots separate. Never use a real user database, profile, credential, audio or output as a test fixture.
