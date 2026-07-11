# ImmersiveReader V3 implementation progress

Updated: 2026-07-11

This file tracks the implementation against the approved V3 plan. A checked
item means the behavior exists in the repository and has a focused automated
test. It does not waive any independent authorization gate in the V3 plan.

## Completed

- [x] Preserve the unified-shell reference image and record the design rules.
- [x] Create the isolated development application, data roots, Library, runtime
  junction, shortcut, and `ship:dev` install path without file associations.
- [x] Separate development and production installation rules in `AGENTS.md` and
  `CONTRIBUTING.md`.
- [x] Resolve production, development, and QA managed roots in Rust.
- [x] Load Settings schema 1/2 as schema 3 without rewriting the source.
- [x] Reject unsafe, overlapping, root, temporary, and managed Library paths.
- [x] Enter read-only recovery mode when Settings cannot be parsed.
- [x] Make authoritative file replacement fail closed on Windows.
- [x] Store Podcast recovery metadata in Data and work files in Cache.
- [x] Protect cache-leased tasks and verify Data, Library, and Backups around
  safe cache cleanup.
- [x] Persist same-volume Library publish transactions and recover every
  pre-commit phase idempotently.
- [x] Create verified SQLite rollback material, checkpoint WAL, run integrity
  and foreign-key checks, preserve schema/table counts, and commit a receipt.
- [x] Produce deterministic, read-only legacy migration previews.
- [x] Produce Zhihu database/filesystem reconciliation categories without
  choosing or deleting conflicting candidates.
- [x] Create `control.db` and persist request-id claims and completed results
  across application restarts.

## In progress

- [ ] Add a persistent migration execution coordinator that revalidates the
  preview, records the run, executes only supported typed operations, and can
  replay an idempotent result after restart.
- [ ] Inherit legacy Settings, recent content, single-file reading state,
  Library reading state, temporary Podcast content/config/tasks, Zhihu data and
  profile, manifests, and trash metadata with per-class validation and rollback.
- [ ] Persist reconciliation reports and migration receipts under the managed
  migration root, without secrets or Profile contents.

## Remaining product work

- [ ] Complete the Rust task snapshot/event state machine, revisions, event-gap
  recovery, and persistent command idempotency at every mutating command.
- [ ] Store DeepSeek credentials in channel-specific Windows Credential Manager
  targets and keep secrets out of disk, logs, IPC payloads, and backups.
- [ ] Add channel-scoped single instance, Windows Job Objects, authenticated
  asynchronous sidecars, READY protocol validation, and one unified tray.
- [ ] Add Podcast single-task TaskSpec, verified input copy, compatibility
  recovery, pause/cancel semantics, deduplication, revisions, and budget gates.
- [ ] Split Zhihu acquisition history from the permanent archive catalog and
  make recrawls non-destructive.
- [ ] Build the unified shell, acquisition pages, task/event panels, settings,
  migration/recovery, trash, provenance, and Library detail surfaces.
- [ ] Wrap the current reader in `ReaderWorkspace`, add navigation protection,
  and embed continuous reading in the Tauri window.
- [ ] After the explicit removal gate, remove legacy Podcast/Zhihu frontends and
  tighten CSP, capabilities, filesystem access, and external opening.
- [ ] Run isolated QA, the two short audio samples, the specified Zhihu Top 5,
  and all non-paid regression gates.
- [ ] After their separate approvals, run full paid audio QA, formal install,
  Markdown association changes, clean-history reconstruction, and remote ref
  updates.

## Independent authorization gates still closed

- Real production-data migration.
- Removal of the old Podcast and Zhihu frontends.
- Full-length audio/API-cost QA.
- Production `ship:local`.
- Markdown file-association changes.
- Force-push of `origin/main` or changes to other remote refs.
- Restoration or archival of the external `Zhihu_packer` repository.

