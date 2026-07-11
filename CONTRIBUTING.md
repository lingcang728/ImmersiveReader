# Contributing to ImmersiveReader

ImmersiveReader integrates the Markdown reader, Podcast Transcriber, and Zhihu
Packer into one Windows desktop product. Keep changes reviewable: one logical
change per commit, with its tests and rollback boundary in the same commit.

## Development channel

Desktop changes are exercised through the isolated development channel:

```powershell
npm.cmd --prefix .\apps\desktop run ship:dev
```

The development executable, shortcuts, application identifier, AppData, cache,
logs, task data, and default Library are separate from production. Development and
QA builds must not read production stores or register Markdown associations.

Production installation is intentionally separate:

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
```

Run it only after an explicit production-install approval. Registering `.md` or
`.markdown` is another independent approval and must not be bundled into routine
build or installation commands.

## Verification

Write a failing test first for path protection, atomic replacement, SQLite
migration, publication transactions, cache cleanup, task state and idempotency,
Windows Job Objects, single-instance behavior, association restoration, and
archive/task-history separation.

Use the narrowest package checks while iterating, then run the repository gate for
cross-package or shared-contract changes:

```powershell
.\scripts\verify.ps1
```

Every desktop commit must finish with `ship:dev` and report the commit, installed
development EXE timestamp, and SHA-256. Documentation-only commits do not require
an install.

## Data safety

Never commit or copy into the repository any Library content, database, browser
profile, local settings, credentials, model, audio input, generated output, cache,
or log. Persistent Data, disposable Cache, Logs, user-visible Library content, and
Backups are distinct stores. A cache-cleaning implementation must accept only
managed categories and must never accept an arbitrary path.

Formal data migration, legacy frontend removal, paid or long-audio QA, production
installation, Markdown association changes, remote history rewriting, remote ref
changes, and restoration or archival of external repositories remain independent
authorization gates.
