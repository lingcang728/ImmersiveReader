# ImmersiveReader V3 implementation progress

Updated: 2026-07-12

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
- [x] Persist migration-run lifecycle and execute Settings schema migration with
  preview freshness, rollback material, receipt verification, and request replay.
- [x] Persist structured task snapshots and events with monotonic revision and
  sequence checks plus event-gap queries.
- [x] Isolate DeepSeek production/development targets in Windows Credential
  Manager without returning secrets to the frontend.
- [x] Add channel-scoped Tauri single-instance handling.
- [x] Add a Windows Job Object with `KILL_ON_JOB_CLOSE` and verify that closing
  it terminates an assigned process tree.
- [x] Add a schema-versioned Podcast single-task entry with independent Data,
  Cache, and Library roots plus sidecar path revalidation.
- [x] Copy Podcast input through `input.partial`, stream SHA-256 verification,
  source stability checks, atomic promotion, and an active cache lease.
- [x] Expose authoritative task snapshots and sequence-gap events through Tauri
  commands, including recoverable cache usage totals.
- [x] Split Zhihu permanent authors/items/revisions from deletable task history;
  deleting a task no longer removes author navigation or successful output paths.
- [x] Make Zhihu force recrawl preserve the last successful Markdown instead of
  deleting files before the replacement fetch succeeds.

## In progress

- [ ] Extend the persistent migration coordinator beyond the completed Settings
  operation to every legacy data class.
- [ ] Inherit legacy Settings, recent content, single-file reading state,
  Library reading state, temporary Podcast content/config/tasks, Zhihu data and
  profile, manifests, and trash metadata with per-class validation and rollback.
- [ ] Persist reconciliation reports and migration receipts under the managed
  migration root, without secrets or Profile contents.

## Remaining product work

- [ ] Emit persisted task events from Rust to the frontend and complete all
  remaining mutating commands with expectedRevision/requestId.
- [ ] Add suspended sidecar spawning, ToolManager ownership, authenticated async
  IPC, READY protocol validation, crash mapping, and one unified tray.
- [ ] Finish Podcast TaskSpec creation, preview/budget estimates, compatibility
  recovery, pause/cancel semantics, deduplication, revisions, and publishing.
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

## Latest verification

- `scripts\verify.ps1`: passed on 2026-07-11.
- Desktop TypeScript tests: 35 passed.
- Desktop Rust tests: 58 passed.
- Zhihu tests: 17 passed.
- Podcast tests with the managed runtime Python: 19 passed.
- Podcast quick validation: passed.
