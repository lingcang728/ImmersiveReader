# ImmersiveReader repository rules

- Preserve the original MMbook, PodcastTranscriber, and Zhihu_packer repositories. This repository is the integration target.
- Never commit Library content, browser profiles, databases, local configuration, API credentials, models, inputs, outputs, or logs.
- Reuse installed Windows runtimes and the global Playwright installation before installing anything.
- Keep the MMbook focus-mode visuals and viewport-anchor behavior unchanged.
- Run `scripts/verify.ps1` when the change spans multiple packages or touches shared contracts.

## Mandatory close-out (every agent task)

Before starting V3 implementation work, read the repository-root
`V3-TODO.md`. Treat it as the current cross-session handoff and execution
order. After each independently verified logical step:

1. Move the item from `未完成` to `已完成` and mark it `[x]`.
2. Update its verification/commit/build evidence when applicable.
3. Commit the checklist update so a new conversation can resume without the
   previous chat history.

After any task that changes **desktop app** code, config, UI, Rust backend, install scripts, or anything that affects the running product:

1. **Git commit** (required for rollback). Do not leave a finished task uncommitted unless the user explicitly says not to commit.
2. **Ship the isolated development install**:

```powershell
npm.cmd --prefix .\apps\desktop run ship:dev
```

This installs `immersive-reader-dev.exe` into the ignored `.dev-install` directory,
refreshes only the development shortcuts, and uses development-only AppData and
Library roots. It must never overwrite the production executable, production data,
or `.md` / `.markdown` associations.

3. Report the **commit hash**, development **EXE timestamp**, and **SHA-256** (at least the first 16 chars).

`ship:local` is a production-install authorization gate. Run it only after the user
has explicitly approved that gate with the current build and QA evidence. Markdown
association registration is a separate authorization gate and is never implied by
approval to run `ship:local`.

If only docs/scripts unrelated to the desktop binary changed, still **git commit**; skip ship only when the installed app binary cannot be affected.
