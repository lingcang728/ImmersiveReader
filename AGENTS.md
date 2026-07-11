# ImmersiveReader repository rules

- Preserve the original MMbook, PodcastTranscriber, and Zhihu_packer repositories. This repository is the integration target.
- Never commit Library content, browser profiles, databases, local configuration, API credentials, models, inputs, outputs, or logs.
- Reuse installed Windows runtimes and the global Playwright installation before installing anything.
- Keep the MMbook focus-mode visuals and viewport-anchor behavior unchanged.
- Run `scripts/verify.ps1` when the change spans multiple packages or touches shared contracts.

## Mandatory close-out (every agent task)

After any task that changes **desktop app** code, config, UI, Rust backend, install scripts, or anything that affects the running product:

1. **Git commit** (required for rollback). Do not leave a finished task uncommitted unless the user explicitly says not to commit.
2. **Ship local install** (required so the desktop shortcut and `.md` / `.markdown` double-click use the new build):

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
```

This builds the NSIS package, silently installs into the monorepo root
(`C:\Users\15pro\Desktop\MyProject\ImmersiveReader\immersive-reader.exe`),
refreshes Desktop/Start Menu shortcuts, and re-registers Markdown associations.

3. Report the **commit hash**, installed **EXE timestamp**, and **SHA-256** (at least the first 16 chars).

Do not stop after `tauri build` alone. An installer that is not installed leaves the user on an old EXE.

If only docs/scripts unrelated to the desktop binary changed, still **git commit**; skip ship only when the installed app binary cannot be affected.
