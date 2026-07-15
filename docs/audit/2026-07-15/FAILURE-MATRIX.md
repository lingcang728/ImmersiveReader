# Failure-injection and recovery matrix

This matrix records the repeatable evidence used by the audit. “Covered” means a deterministic test or recovery path exists; “live-only” means the behavior requires a sidecar/process or external Windows runtime and is kept as an explicit acceptance gate rather than being simulated with a false green test.

| Domain | Injection/window | Expected invariant | Evidence/status |
| --- | --- | --- | --- |
| Rust publish | Prepared, OldMoved, NewMoved, Committed interruption | old final remains recoverable; committed state is not rolled back | `publish/tests.rs` phase-stop/recovery tests; covered |
| Rust publish | validation failure before promotion | previous successful book remains | `prepared_validation_failure_preserves_last_successful_book`; covered |
| Zhihu publish | old final rename failure / incoming failure / validation failure | old final is never deleted by rollback | `publish.test.ts` rollback regression cases; covered |
| Author identity | same name, case collision, cleaned/truncated collision | final directory remains ID-stable | author directory tests; covered |
| Podcast identity | same display stem, different source IDs | published paths do not collide | same-stem identity test; covered |
| SQLite claim | two connections claim one request; same/different fingerprint | exactly one New; deterministic Existing or key-reuse error | `control/tests.rs`; covered |
| SQLite completion | external side effect before command completion | replay never blindly redoes destructive work | command-result replay and Trash idempotency tests; covered |
| Task event | event/snapshot sequence gap, stale revision, throttled progress | no gaps; terminal event is not throttled | `control/tests.rs`; covered |
| Input copy | one persistence failure | later sequence/revision continues from committed state | Podcast input-copy tests; covered |
| Worker process | stdout/stderr burst, exit mapping, stale engine | supervisor state converges to terminal/recoverable state | `control/tests.rs`, Job Object tests; covered |
| cancel-and-discard | stop before/after supervisor transition; process restart | captured Podcast IDs remain durable until cache cleanup completes | `cancel_discard_intents`, recovery helper and control test; covered |
| Trash move | rename complete, metadata incomplete | restart writes metadata or removes a not-started journal; no invisible book | Trash journal tests; covered |
| Trash restore | metadata removed, destination conflict | source remains listed and metadata is repaired; conflict never overwrites | Trash journal tests; covered |
| Trash permanent delete | content deleted, claim not completed | restart clears completed journal without deleting another item | Trash journal tests; covered |
| Image SSRF | private/loopback/DNS-rebinding/redirect hop/oversized/MIME mismatch | request rejected before unsafe write; bounded temp file and atomic rename | extractor tests and bounded streaming implementation; covered |
| Sidecar READY | timeout, health-before-ready, stale revision | no unverified worker is treated as ready | protocol tests; live process acceptance remains required |
| Reader navigation | slow A response after opening B | A cannot overwrite B | generation/in-flight tests and watcher guards; covered |
| Focus Mode | long block, reflow, keyboard/scroll anchor | anchor ratio and core visuals remain stable | Focus characterization suite; covered |

The remaining live-only acceptance is exercised by the final installed-build smoke run with a fresh synthetic QA root. No real user data is a valid fixture for this matrix.
