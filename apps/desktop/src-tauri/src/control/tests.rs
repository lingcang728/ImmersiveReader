use super::{CommandClaim, ControlDb, CONTROL_SCHEMA_VERSION, TASK_EVENT_KEEP_LATEST};
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome,
    TaskProgress, TaskSnapshot,
};
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

#[test]
fn command_result_is_idempotent_across_database_reopen() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-idempotent-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        assert!(matches!(
            database
                .claim_command("request-1", "clear_safe_cache", "input-a")
                .expect("first claim must succeed"),
            CommandClaim::New
        ));
        database
            .complete_command("request-1", r#"{"ok":true}"#, None, Some(4))
            .expect("command result must persist");
    }

    let reopened = ControlDb::open(&path).expect("control database must reopen");
    match reopened
        .claim_command("request-1", "clear_safe_cache", "input-a")
        .expect("same request must replay")
    {
        CommandClaim::Existing(result) => {
            assert_eq!(result.result_json.as_deref(), Some(r#"{"ok":true}"#));
            assert_eq!(result.resulting_revision, Some(4));
        }
        CommandClaim::New => panic!("completed request must not execute twice"),
    }
    let error = reopened
        .claim_command("request-1", "clear_safe_cache", "different-input")
        .expect_err("request id reuse with different input must fail");
    assert!(error.contains("IDEMPOTENCY_KEY_REUSED"));
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn concurrent_command_claims_are_deterministic() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-concurrent-claim-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let path = path.clone();
            thread::spawn(move || {
                let database = ControlDb::open(&path).expect("control database must open");
                barrier.wait();
                database
                    .claim_command("request-concurrent", "command", "input")
                    .expect("claim must not fail with a uniqueness error")
            })
        })
        .collect::<Vec<_>>();
    let claims = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread must finish"))
        .collect::<Vec<_>>();
    assert_eq!(
        claims
            .iter()
            .filter(|claim| **claim == CommandClaim::New)
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, CommandClaim::Existing(_)))
            .count(),
        1
    );
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn control_database_creates_all_v3_control_tables() {
    let root =
        std::env::temp_dir().join(format!("immersive-control-schema-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    let tables = database.table_names().expect("table names must load");

    for expected in [
        "task_snapshots",
        "task_events",
        "command_results",
        "engine_instances",
        "publish_transaction_index",
        "migration_runs",
        "cancel_discard_intents",
    ] {
        assert!(tables.contains(&expected.to_string()), "missing {expected}");
    }
    // P3-24: schema v2 drops `cache_leases` — it was created but never
    // written or read; the live lease store is per-task `recovery.json`.
    assert!(
        !tables.contains(&"cache_leases".to_string()),
        "dead cache_leases table must be dropped"
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn migration_runs_survive_reopening_and_keep_receipt_location() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-migration-run-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        database
            .begin_migration_run("migration-1", "preview-1", "settings")
            .expect("migration run must start");
        database
            .complete_migration_run(
                "migration-1",
                "success",
                Some(r"Data\Migrations\migration-1\receipt.json"),
                r#"{"status":"success"}"#,
            )
            .expect("migration run must complete");
    }

    let reopened = ControlDb::open(&path).expect("control database must reopen");
    let run = reopened
        .migration_run("migration-1")
        .expect("migration run must load")
        .expect("migration run must exist");
    assert_eq!(run.status, "success");
    assert_eq!(run.preview_id, "preview-1");
    assert_eq!(
        run.receipt_path.as_deref(),
        Some(r"Data\Migrations\migration-1\receipt.json")
    );
    assert_eq!(run.result_json.as_deref(), Some(r#"{"status":"success"}"#));
    let runs = reopened.migration_runs().expect("migration runs must list");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].migration_id, "migration-1");
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

fn task_event_for(
    task_id: &str,
    kind: TaskKind,
    book_id: &str,
    sequence: u64,
    revision: u64,
) -> TaskEvent {
    let now = "2026-07-11T12:00:00Z".to_string();
    let snapshot = TaskSnapshot {
        id: task_id.to_string(),
        kind,
        revision,
        last_sequence: sequence,
        lifecycle_state: LifecycleState::Running,
        outcome: TaskOutcome::None,
        required_action: RequiredAction::None,
        progress: TaskProgress {
            mode: ProgressMode::Determinate,
            percent: Some(sequence as f64 * 10.0),
            completed_units: Some(sequence),
            total_units: Some(10),
            label: Some("transcribing".to_string()),
            unit: None,
            source_total_units: None,
            skipped_units: None,
        },
        error_code: None,
        error_message: None,
        retry_after_seconds: None,
        engine_stage: "transcribe".to_string(),
        engine_status: "working".to_string(),
        recoverable: true,
        can_pause: true,
        can_resume: false,
        can_retry: false,
        can_cancel: true,
        book_id: Some(book_id.to_string()),
        source_id: Some("sha256".to_string()),
        display_name: None,
        cache_lease_bytes: 42,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_heartbeat_at: None,
        checkpoint_at: None,
    };
    TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence,
        revision,
        event_type: "progress".to_string(),
        snapshot,
        created_at: now,
    }
}

fn task_event(sequence: u64, revision: u64) -> TaskEvent {
    task_event_for("podcast-1", TaskKind::Podcast, "book-1", sequence, revision)
}

#[test]
fn cancel_discard_capture_survives_reopen_until_cache_cleanup_completes() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-cancel-intent-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let mut database = ControlDb::open(&path).expect("control database must open");
        database
            .persist_task_event(&task_event(1, 1))
            .expect("task must persist");
        assert_eq!(
            database
                .capture_cancel_discard()
                .expect("cancel set must persist"),
            vec!["podcast-1"]
        );
        database
            .cancel_active_tasks()
            .expect("task must become terminal");
    }
    let reopened = ControlDb::open(&path).expect("control database must reopen");
    assert_eq!(
        reopened
            .pending_cancel_discard()
            .expect("pending intents must load"),
        vec!["podcast-1"]
    );
    reopened
        .complete_cancel_discard("podcast-1")
        .expect("intent must complete");
    assert!(reopened
        .pending_cancel_discard()
        .expect("pending intents must reload")
        .is_empty());
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn task_snapshot_and_events_survive_reopen() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-task-events-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        database
            .persist_task_event(&task_event(1, 1))
            .expect("first event must persist");
        database
            .persist_task_event(&task_event(2, 2))
            .expect("second event must persist");
    }

    let reopened = ControlDb::open(&path).expect("control database must reopen");
    let snapshot = reopened
        .task_snapshot("podcast-1")
        .expect("snapshot must load")
        .expect("snapshot must exist");
    assert_eq!(snapshot.revision, 2);
    assert_eq!(snapshot.last_sequence, 2);
    let events = reopened
        .task_events("podcast-1", 1, 100)
        .expect("event gap must load");
    assert_eq!(events, vec![task_event(2, 2)]);
    assert_eq!(
        reopened
            .task_snapshots_for_book("book-1")
            .expect("book task records must load"),
        vec![snapshot.clone()]
    );
    assert!(reopened
        .task_snapshots_for_book("other-book")
        .expect("unrelated book task records must load")
        .is_empty());
    assert_eq!(
        reopened
            .task_snapshots(Some(TaskKind::Podcast))
            .expect("podcast snapshots must load"),
        vec![snapshot]
    );
    assert!(reopened
        .task_snapshots(Some(TaskKind::Zhihu))
        .expect("zhihu snapshots must load")
        .is_empty());
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn task_events_keep_first_and_latest_n_per_task() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-event-cap-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let database = ControlDb::open(&path).expect("control database must open");
    let total = TASK_EVENT_KEEP_LATEST as u64 + 3;
    for sequence in 1..=total {
        database
            .persist_task_event(&task_event(sequence, sequence))
            .expect("event must persist");
    }

    let all = database
        .task_events("podcast-1", 0, 10_000)
        .expect("events must load");
    assert_eq!(all.len() as i64, TASK_EVENT_KEEP_LATEST + 1);
    assert_eq!(all.first().map(|event| event.sequence), Some(1));
    assert_eq!(all.last().map(|event| event.sequence), Some(total));
    // The reader-side contract holds through the gap: the newest event is
    // always re-fetchable for rebroadcast.
    let latest = database
        .task_events("podcast-1", total - 1, 1)
        .expect("latest event must load");
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].sequence, total);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn prune_terminal_tasks_filters_on_sql_side() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-prune-terminal-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let database = ControlDb::open(&path).expect("control database must open");

    let mut terminal_old = task_event_for("old-terminal", TaskKind::Podcast, "book-1", 1, 1);
    terminal_old.snapshot.lifecycle_state = LifecycleState::Terminal;
    terminal_old.snapshot.updated_at = "2000-01-01T00:00:00Z".to_string();
    database
        .persist_task_event(&terminal_old)
        .expect("old terminal event must persist");

    let mut terminal_recent = task_event_for("recent-terminal", TaskKind::Podcast, "book-1", 1, 1);
    terminal_recent.snapshot.lifecycle_state = LifecycleState::Terminal;
    terminal_recent.snapshot.updated_at = chrono::Utc::now().to_rfc3339();
    database
        .persist_task_event(&terminal_recent)
        .expect("recent terminal event must persist");

    // A non-terminal row past the cutoff must survive the sweep.
    let mut running_old = task_event_for("old-running", TaskKind::Podcast, "book-1", 1, 1);
    running_old.snapshot.updated_at = "2000-01-01T00:00:00Z".to_string();
    database
        .persist_task_event(&running_old)
        .expect("old running event must persist");

    assert_eq!(
        database
            .prune_terminal_tasks_older_than(7)
            .expect("prune must run"),
        1
    );
    assert!(database
        .task_snapshot("old-terminal")
        .expect("snapshot lookup must run")
        .is_none());
    // Cascade: the pruned task's events go with the snapshot row.
    assert!(database
        .task_events("old-terminal", 0, 100)
        .expect("event lookup must run")
        .is_empty());
    assert!(database
        .task_snapshot("recent-terminal")
        .expect("snapshot lookup must run")
        .is_some());
    assert!(database
        .task_snapshot("old-running")
        .expect("snapshot lookup must run")
        .is_some());
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn set_task_display_name_patches_snapshot_json_in_place() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-display-name-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let database = ControlDb::open(&path).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("task must persist");

    database
        .set_task_display_name("podcast-1", "episode-42")
        .expect("display name must persist");
    let snapshot = database
        .task_snapshot("podcast-1")
        .expect("snapshot must load")
        .expect("snapshot must exist");
    assert_eq!(snapshot.display_name.as_deref(), Some("episode-42"));
    // json_set touches only the one key — the rest of the row is intact.
    assert_eq!(snapshot.lifecycle_state, LifecycleState::Running);
    assert_eq!(snapshot.last_sequence, 1);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn podcast_and_zhihu_active_snapshots_can_coexist() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-parallel-tasks-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let database = ControlDb::open(&path).expect("control database must open");
    database
        .persist_task_event(&task_event_for(
            "podcast-active",
            TaskKind::Podcast,
            "podcast-book",
            1,
            1,
        ))
        .expect("podcast task must persist");
    database
        .persist_task_event(&task_event_for(
            "zhihu-active",
            TaskKind::Zhihu,
            "zhihu-book",
            1,
            1,
        ))
        .expect("zhihu task must persist");

    let active = database
        .task_snapshots(None)
        .expect("active task snapshots must load");
    assert_eq!(active.len(), 2);
    assert!(active.iter().all(|snapshot| {
        snapshot.lifecycle_state == LifecycleState::Running && snapshot.outcome == TaskOutcome::None
    }));
    assert!(active
        .iter()
        .any(|snapshot| { snapshot.id == "podcast-active" && snapshot.kind == TaskKind::Podcast }));
    assert!(active
        .iter()
        .any(|snapshot| { snapshot.id == "zhihu-active" && snapshot.kind == TaskKind::Zhihu }));

    drop(database);
    fs::remove_dir_all(root).expect("test root must be removed");
}

#[test]
fn task_event_rejects_sequence_gaps_and_old_revisions() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-task-conflict-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("first event must persist");

    assert_eq!(
        database
            .persist_task_event(&task_event(3, 2))
            .expect_err("sequence gap must fail"),
        "EVENT_SEQUENCE_CONFLICT"
    );
    assert_eq!(
        database
            .persist_task_event(&task_event(2, 1))
            .expect_err("old revision must fail"),
        "REVISION_CONFLICT"
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn engine_crash_marks_active_tasks_interrupted_and_is_idempotent() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-engine-crash-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    database
        .record_engine_instance(
            "podcast",
            4242,
            Some(43210),
            Some(1),
            "2026-07-12T08:00:00Z",
        )
        .expect("engine instance must persist");

    assert!(database
        .mark_engine_crashed("podcast", 4242, Some(7))
        .expect("engine crash must persist"));
    let snapshot = database
        .task_snapshot("podcast-1")
        .expect("snapshot must load")
        .expect("snapshot must exist");
    assert_eq!(snapshot.lifecycle_state, LifecycleState::Terminal);
    assert_eq!(snapshot.outcome, TaskOutcome::Interrupted);
    assert_eq!(snapshot.error_code, Some(TaskErrorCode::EngineCrashed));
    assert_eq!(snapshot.engine_status, "exited");
    assert!(snapshot.can_retry);
    assert!(!snapshot.can_pause);
    assert!(!database
        .mark_engine_crashed("podcast", 4242, Some(7))
        .expect("duplicate engine crash must be idempotent"));
    assert_eq!(
        database
            .task_events("podcast-1", 1, 100)
            .expect("events must load")
            .len(),
        1
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn stale_running_engine_is_recovered_after_reopen() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-stale-engine-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        database
            .persist_task_event(&task_event(1, 1))
            .expect("running task must persist");
        database
            .record_engine_instance(
                "podcast",
                4343,
                Some(43211),
                Some(1),
                "2026-07-12T08:05:00Z",
            )
            .expect("engine instance must persist");
    }
    let mut reopened = ControlDb::open(&path).expect("control database must reopen");
    assert_eq!(
        reopened
            .recover_stale_engine_instances()
            .expect("stale engine must recover"),
        1
    );
    assert_eq!(
        reopened
            .task_snapshot("podcast-1")
            .expect("snapshot must load")
            .expect("snapshot must exist")
            .outcome,
        TaskOutcome::Interrupted
    );
    assert_eq!(reopened.recover_stale_engine_instances().unwrap(), 0);
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn cancel_active_tasks_marks_them_cancelled_and_is_idempotent() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-cancel-discard-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");

    let podcast_ids = database
        .cancel_active_tasks()
        .expect("active tasks must be cancelled");
    assert_eq!(podcast_ids, vec!["podcast-1".to_string()]);
    let snapshot = database
        .task_snapshot("podcast-1")
        .expect("snapshot must load")
        .expect("snapshot must exist");
    assert_eq!(snapshot.lifecycle_state, LifecycleState::Terminal);
    assert_eq!(snapshot.outcome, TaskOutcome::Cancelled);
    assert_eq!(snapshot.error_code, Some(TaskErrorCode::CancelledByUser));
    assert!(!snapshot.recoverable);
    assert!(!snapshot.can_retry);
    assert_eq!(
        database
            .cancel_active_tasks()
            .expect("second cleanup must be idempotent"),
        Vec::<String>::new()
    );
    assert_eq!(
        database
            .task_events("podcast-1", 1, 100)
            .expect("events must load")
            .len(),
        1
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn worker_stdout_stderr_and_exit_map_to_task_events() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-worker-events-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    let stdout = database
        .record_worker_line(
            "podcast-1",
            "stdout",
            r#"{"type":"progress","stage":"chunking","percent":50.0,"message":"切分中"}"#,
        )
        .expect("stdout must map")
        .expect("stdout event must exist");
    assert_eq!(stdout.event_type, "worker_progress");
    // chunking band is 18–24 → raw 50% maps to 21.
    assert_eq!(stdout.snapshot.progress.percent, Some(21.0));
    assert_eq!(stdout.snapshot.engine_stage, "chunking");
    assert_eq!(
        stdout.snapshot.progress.label.as_deref(),
        Some("正在切分音频")
    );
    let ndjson = database
        .record_worker_line(
            "podcast-1",
            "stdout",
            r#"{"type":"progress","stage":"transcribe","percent":50.0,"completedUnits":11,"totalUnits":20,"unit":"块","message":"转写第 11 块"}"#,
        )
        .expect("ndjson must map")
        .expect("ndjson event must exist");
    assert_eq!(ndjson.event_type, "worker_progress");
    // transcribing band is 24–70 → raw 50% maps to 47; floor keeps progress monotonic.
    assert_eq!(ndjson.snapshot.progress.percent, Some(47.0));
    assert_eq!(ndjson.snapshot.engine_stage, "transcribing");
    assert_eq!(ndjson.snapshot.progress.completed_units, Some(11));
    assert_eq!(ndjson.snapshot.progress.total_units, Some(20));
    assert_eq!(
        ndjson.snapshot.progress.label.as_deref(),
        Some("正在语音转写")
    );
    // Spammy stderr without stage/% change is throttled (prevents UI flicker).
    assert!(database
        .record_worker_line("podcast-1", "stderr", "worker warning")
        .expect("stderr throttle")
        .is_none());
    let fatal = database
        .record_worker_line(
            "podcast-1",
            "stderr",
            r#"{"type":"fatal","errorCode":"BUDGET_CONFIRMATION_REQUIRED","message":"budget exceeds"}"#,
        )
        .expect("fatal must map")
        .expect("fatal event must exist");
    assert_eq!(fatal.event_type, "worker_fatal");
    assert!(fatal
        .snapshot
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("budget"));
    let done = database
        .finish_worker_task("podcast-1", true, None)
        .expect("worker completion must map")
        .expect("completion event must exist");
    assert_eq!(done.event_type, "worker_completed");
    assert_eq!(done.snapshot.outcome, TaskOutcome::Success);
    assert_eq!(done.snapshot.progress.percent, Some(100.0));
    let mut failed_task = task_event(1, 1);
    failed_task.task_id = "podcast-2".to_string();
    failed_task.snapshot.id = "podcast-2".to_string();
    database
        .persist_task_event(&failed_task)
        .expect("failed task must persist");
    let failed = database
        .finish_worker_task(
            "podcast-2",
            false,
            Some(r#"{"errorCode":"RATE_LIMITED","retryAfterSeconds":9}"#),
        )
        .expect("worker failure must map")
        .expect("failure event must exist");
    assert_eq!(failed.snapshot.error_code, Some(TaskErrorCode::RateLimited));
    assert_eq!(failed.snapshot.retry_after_seconds, Some(9));
    let mut budget_task = task_event(1, 1);
    budget_task.task_id = "podcast-3".to_string();
    budget_task.snapshot.id = "podcast-3".to_string();
    database
        .persist_task_event(&budget_task)
        .expect("budget task must persist");
    let budget = database
        .finish_worker_task(
            "podcast-3",
            false,
            Some(r#"{"errorCode":"BUDGET_CONFIRMATION_REQUIRED"}"#),
        )
        .expect("budget failure must map")
        .expect("budget event must exist");
    assert_eq!(
        budget.snapshot.required_action,
        RequiredAction::ApproveBudget
    );
    assert!(!budget.snapshot.can_retry);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn stale_worker_output_never_revives_a_queued_task() {
    // 02-F9: a worker line/finish landing on a Queued row belongs to a stale
    // pipe (the task was requeued/rolled back while it drained) — both must
    // be dropped instead of resurrecting the row as Running/Terminal.
    let root = std::env::temp_dir().join(format!(
        "immersive-control-queued-stale-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let mut queued = task_event(1, 1);
    queued.task_id = "podcast-queued".to_string();
    queued.snapshot.id = "podcast-queued".to_string();
    queued.snapshot.lifecycle_state = LifecycleState::Queued;
    queued.snapshot.engine_stage = "queued".to_string();
    queued.snapshot.engine_status = "waiting".to_string();
    database
        .persist_task_event(&queued)
        .expect("queued task must persist");

    assert!(database
        .record_worker_line(
            "podcast-queued",
            "stdout",
            r#"{"type":"progress","stage":"transcribe","percent":50.0}"#,
        )
        .expect("queued worker line must not error")
        .is_none());
    assert!(database
        .finish_worker_task("podcast-queued", true, None)
        .expect("queued worker finish must not error")
        .is_none());
    let snapshot = database
        .task_snapshot("podcast-queued")
        .expect("snapshot must load")
        .expect("task must exist");
    assert_eq!(snapshot.lifecycle_state, LifecycleState::Queued);
    assert_eq!(snapshot.revision, 1);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn task_controls_enforce_revision_and_transition_pause_resume_cancel() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-task-controls-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    let paused = database
        .control_task("podcast-1", "pause", 1)
        .expect("pause must persist");
    assert_eq!(paused.snapshot.lifecycle_state, LifecycleState::Paused);
    assert_eq!(paused.snapshot.revision, 2);
    assert_eq!(
        database
            .control_task("podcast-1", "resume", 1)
            .expect_err("stale pause revision must fail"),
        "REVISION_CONFLICT"
    );
    let resumed = database
        .control_task("podcast-1", "resume", 2)
        .expect("resume must persist");
    assert_eq!(resumed.snapshot.lifecycle_state, LifecycleState::Running);
    let cancelled = database
        .control_task("podcast-1", "cancel", 3)
        .expect("cancel must persist");
    assert_eq!(cancelled.snapshot.outcome, TaskOutcome::Cancelled);
    assert!(cancelled.snapshot.recoverable);
    assert!(cancelled.snapshot.can_retry);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn start_reservation_can_be_rolled_back_without_losing_the_task() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-start-reservation-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let mut queued = task_event_for("zhihu-start", TaskKind::Zhihu, "zhihu:author", 1, 1);
    queued.snapshot.lifecycle_state = LifecycleState::Queued;
    queued.snapshot.engine_stage = "queued".to_string();
    queued.snapshot.engine_status = "waiting".to_string();
    database
        .persist_task_event(&queued)
        .expect("queued task must persist");
    assert_eq!(
        database
            .validate_task_control("zhihu-start", TaskKind::Zhihu, 0)
            .expect_err("stale revision must be rejected"),
        "REVISION_CONFLICT"
    );
    let starting = database
        .mark_task_starting("zhihu-start")
        .expect("start reservation must persist")
        .expect("start event must exist");
    assert_eq!(starting.snapshot.lifecycle_state, LifecycleState::Starting);
    let rolled_back = database
        .rollback_starting_task("zhihu-start")
        .expect("reservation rollback must persist")
        .expect("rollback event must exist");
    assert_eq!(rolled_back.snapshot.lifecycle_state, LifecycleState::Queued);
    assert_eq!(rolled_back.snapshot.revision, 3);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn external_snapshot_ignores_identical_progress_but_records_heartbeat() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-external-heartbeat-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    let current = database
        .task_snapshot("podcast-1")
        .expect("load")
        .expect("exists");
    assert!(database
        .record_external_snapshot(current.clone(), "engine_progress")
        .expect("noop progress")
        .is_none());
    let mut heartbeat = current;
    heartbeat.last_heartbeat_at = Some("2026-07-14T01:00:00Z".to_string());
    let event = database
        .record_external_snapshot(heartbeat, "engine_heartbeat")
        .expect("heartbeat")
        .expect("heartbeat event");
    assert_eq!(event.event_type, "engine_heartbeat");
    assert_eq!(
        event.snapshot.last_heartbeat_at.as_deref(),
        Some("2026-07-14T01:00:00Z")
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn zhihu_sidecar_success_overrides_false_interrupted_terminal() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-zhihu-override-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let mut event = task_event_for("zhihu-1", TaskKind::Zhihu, "zhihu:author", 1, 1);
    event.snapshot.lifecycle_state = LifecycleState::Terminal;
    event.snapshot.outcome = TaskOutcome::Interrupted;
    event.snapshot.error_code = Some(TaskErrorCode::EngineCrashed);
    event.snapshot.engine_stage = "crashed".to_string();
    event.snapshot.engine_status = "exited".to_string();
    event.snapshot.can_retry = true;
    event.snapshot.can_pause = false;
    event.snapshot.can_cancel = false;
    database
        .persist_task_event(&event)
        .expect("interrupted zhihu must persist");

    let mut success = event.snapshot.clone();
    success.lifecycle_state = LifecycleState::Terminal;
    success.outcome = TaskOutcome::Success;
    success.error_code = None;
    success.error_message = None;
    success.engine_stage = "content".to_string();
    success.engine_status = "success".to_string();
    success.progress.percent = Some(100.0);
    success.progress.completed_units = Some(10);
    success.progress.total_units = Some(10);
    success.can_retry = false;
    success.recoverable = false;

    let applied = database
        .record_external_snapshot(success, "engine_completed")
        .expect("override")
        .expect("success event");
    assert_eq!(applied.snapshot.outcome, TaskOutcome::Success);
    assert_eq!(applied.snapshot.lifecycle_state, LifecycleState::Terminal);
    assert!(applied.snapshot.error_code.is_none());

    // True terminal success must not be overwritten again.
    let mut again = applied.snapshot.clone();
    again.outcome = TaskOutcome::Failed;
    assert!(database
        .record_external_snapshot(again, "engine_completed")
        .expect("second write")
        .is_none());
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn orphaned_podcast_tasks_without_contract_are_marked_input_copy_failed() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-orphan-podcast-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let data_root = root.join("data");
    fs::create_dir_all(&data_root).expect("data root");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let mut queued = task_event(1, 1);
    queued.snapshot.lifecycle_state = LifecycleState::Queued;
    queued.snapshot.engine_stage = "queued".to_string();
    queued.snapshot.engine_status = "waiting".to_string();
    queued.snapshot.can_pause = false;
    database
        .persist_task_event(&queued)
        .expect("queued podcast must persist");
    assert_eq!(
        database
            .repair_orphaned_podcast_tasks_at(&data_root)
            .expect("repair"),
        1
    );
    let snapshot = database
        .task_snapshot("podcast-1")
        .expect("load")
        .expect("exists");
    assert_eq!(snapshot.outcome, TaskOutcome::Failed);
    assert_eq!(snapshot.error_code, Some(TaskErrorCode::InputCopyFailed));
    assert!(!snapshot.recoverable);
    assert_eq!(
        database
            .repair_orphaned_podcast_tasks_at(&data_root)
            .expect("idempotent"),
        0
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

fn stale_stamp() -> String {
    (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339()
}

fn fresh_stamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// P1-16: a control.db SQLite cannot parse must be quarantined to
/// `control.db.corrupt-<epoch>` and rebuilt empty instead of permanently
/// failing every task command.
#[test]
fn corrupt_control_database_is_quarantined_and_rebuilt() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-corrupt-db-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    // Well over SQLite's 100-byte header so it is unambiguously "not a database".
    fs::write(&path, vec![0x5a_u8; 4096]).expect("corrupt fixture must write");
    fs::write(root.join("control.db-wal"), b"stale wal").expect("wal fixture");
    fs::write(root.join("control.db-shm"), b"stale shm").expect("shm fixture");

    let database = ControlDb::open(&path).expect("corrupt database must self-heal");
    let tables = database.table_names().expect("rebuilt schema must load");
    assert!(tables.contains(&"task_snapshots".to_string()));
    assert!(matches!(
        database
            .claim_command("heal-1", "command", "input")
            .expect("rebuilt database must accept commands"),
        CommandClaim::New
    ));
    let corrupt_files: Vec<String> = fs::read_dir(&root)
        .expect("root must list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".corrupt-"))
        .collect();
    assert!(
        corrupt_files
            .iter()
            .any(|name| name.starts_with("control.db.corrupt-")),
        "damaged database must be quarantined: {corrupt_files:?}"
    );
    // A stale WAL sidecar must never survive at its original name where it
    // could replay into the rebuilt database — quarantine renames it, and
    // SQLite may also simply remove it when the corrupt handle is dropped.
    let stale_wal_left = fs::read(root.join("control.db-wal"))
        .map(|bytes| bytes == b"stale wal")
        .unwrap_or(false);
    assert!(
        !stale_wal_left,
        "stale WAL sidecar must not survive rebuild"
    );
    drop(database);
    // The rebuilt file is a real database: reopen is a plain open and the
    // idempotency claim persists.
    let reopened = ControlDb::open(&path).expect("rebuilt database must reopen");
    assert!(matches!(
        reopened
            .claim_command("heal-1", "command", "input")
            .expect("claim must replay"),
        CommandClaim::Existing(_)
    ));
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P1-16: a healthy database must never be quarantined or rebuilt over.
#[test]
fn healthy_control_database_is_never_quarantined() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-healthy-db-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        database
            .claim_command("keep-1", "command", "input")
            .expect("claim must persist");
        database
            .complete_command("keep-1", r#"{"ok":true}"#, None, None)
            .expect("completion must persist");
    }
    let reopened = ControlDb::open(&path).expect("healthy database must reopen");
    match reopened
        .claim_command("keep-1", "command", "input")
        .expect("claim must replay")
    {
        CommandClaim::Existing(record) => {
            assert_eq!(record.result_json.as_deref(), Some(r#"{"ok":true}"#))
        }
        CommandClaim::New => panic!("healthy database must not have been rebuilt"),
    }
    let quarantined = fs::read_dir(&root)
        .expect("root must list")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"));
    assert!(!quarantined, "healthy database must not be quarantined");
    drop(reopened);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P1-4: worker-side suspend gating reads the persisted lifecycle state.
#[test]
fn task_lifecycle_state_reports_serde_names() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-lifecycle-state-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    assert_eq!(
        database
            .task_lifecycle_state("podcast-1")
            .expect("lifecycle state must load")
            .as_deref(),
        Some("running")
    );
    database
        .control_task("podcast-1", "pause", 1)
        .expect("pause must persist");
    assert_eq!(
        database
            .task_lifecycle_state("podcast-1")
            .expect("paused state must load")
            .as_deref(),
        Some("paused")
    );
    assert!(database
        .task_lifecycle_state("missing-task")
        .expect("missing task must load")
        .is_none());
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P1-4: a worker line buffered before the suspend lands must not flip a
/// Paused task back to Running — that is what made `pause_task` suspend an
/// already-suspended process and left resume powerless.
#[test]
fn worker_line_does_not_resurrect_paused_task() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-paused-line-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    database
        .persist_task_event(&task_event(1, 1))
        .expect("running task must persist");
    database
        .control_task("podcast-1", "pause", 1)
        .expect("pause must persist");

    let event = database
        .record_worker_line(
            "podcast-1",
            "stdout",
            r#"{"type":"progress","stage":"transcribe","percent":40.0}"#,
        )
        .expect("buffered line must persist")
        .expect("stage change must emit an event");
    assert_eq!(event.snapshot.lifecycle_state, LifecycleState::Paused);
    assert_eq!(event.snapshot.engine_stage, "paused");
    assert!(event.snapshot.can_resume);
    assert!(!event.snapshot.can_pause);
    assert_eq!(
        database
            .task_lifecycle_state("podcast-1")
            .expect("state must load")
            .as_deref(),
        Some("paused")
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P1-1: active podcast tasks with no live worker (or an expired heartbeat)
/// are marked Interrupted so they can be retried instead of staying Running
/// forever; terminal/queued rows and live workers are untouched.
#[test]
fn recover_interrupted_tasks_marks_workerless_active_tasks() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-recover-interrupted-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    // Running task with no live worker — the classic post-quit state.
    let mut orphaned = task_event(1, 1);
    orphaned.snapshot.last_heartbeat_at = Some(stale_stamp());
    orphaned.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&orphaned)
        .expect("running task must persist");

    // Paused task whose worker is gone — must also recover.
    let mut paused = task_event_for("podcast-paused", TaskKind::Podcast, "book-2", 1, 1);
    paused.snapshot.lifecycle_state = LifecycleState::Paused;
    paused.snapshot.engine_stage = "paused".to_string();
    paused.snapshot.engine_status = "paused".to_string();
    paused.snapshot.last_heartbeat_at = Some(stale_stamp());
    paused.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&paused)
        .expect("paused task must persist");

    // Queued task — never had a worker, must be left alone.
    let mut queued = task_event_for("podcast-queued", TaskKind::Podcast, "book-3", 1, 1);
    queued.snapshot.lifecycle_state = LifecycleState::Queued;
    queued.snapshot.engine_stage = "queued".to_string();
    queued.snapshot.engine_status = "waiting".to_string();
    database
        .persist_task_event(&queued)
        .expect("queued task must persist");

    // Running task whose worker IS registered and heartbeating — keep it.
    let mut live = task_event_for("podcast-live", TaskKind::Podcast, "book-4", 1, 1);
    live.snapshot.last_heartbeat_at = Some(fresh_stamp());
    live.snapshot.updated_at = fresh_stamp();
    database
        .persist_task_event(&live)
        .expect("live task must persist");

    assert_eq!(
        database
            .recover_interrupted_tasks(&["podcast-live".to_string()])
            .expect("recovery must run"),
        2
    );
    for task_id in ["podcast-1", "podcast-paused"] {
        let snapshot = database
            .task_snapshot(task_id)
            .expect("snapshot must load")
            .expect("snapshot must exist");
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Terminal);
        assert_eq!(snapshot.outcome, TaskOutcome::Interrupted);
        assert_eq!(snapshot.error_code, Some(TaskErrorCode::EngineCrashed));
        assert!(snapshot.recoverable);
        assert!(snapshot.can_retry);
        assert!(!snapshot.can_pause);
    }
    let events = database
        .task_events("podcast-1", 1, 100)
        .expect("events must load");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "worker_lost");
    assert_eq!(
        database
            .task_snapshot("podcast-queued")
            .expect("queued must load")
            .expect("queued must exist")
            .lifecycle_state,
        LifecycleState::Queued
    );
    assert_eq!(
        database
            .task_snapshot("podcast-live")
            .expect("live must load")
            .expect("live must exist")
            .lifecycle_state,
        LifecycleState::Running
    );

    // Idempotent: a second pass finds only terminal/healthy rows.
    assert_eq!(
        database
            .recover_interrupted_tasks(&["podcast-live".to_string()])
            .expect("second recovery must be a no-op"),
        0
    );

    // A registered worker whose heartbeat has expired is dead weight too.
    let mut hung = task_event_for("podcast-hung", TaskKind::Podcast, "book-5", 1, 1);
    hung.snapshot.last_heartbeat_at = Some(stale_stamp());
    hung.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&hung)
        .expect("hung task must persist");
    assert_eq!(
        database
            .recover_interrupted_tasks(&["podcast-live".to_string(), "podcast-hung".to_string()])
            .expect("stale heartbeat must recover"),
        1
    );
    assert_eq!(
        database
            .task_snapshot("podcast-hung")
            .expect("hung must load")
            .expect("hung must exist")
            .outcome,
        TaskOutcome::Interrupted
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P1-15: a Running task whose DB heartbeat and `work/state/<task>.json` are
/// both stale past `stale_after` is reaped as Interrupted; live workers, Paused
/// tasks and terminal rows are never reaped.
#[test]
fn reap_stale_workers_marks_silent_running_tasks() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-reap-stale-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let work_root = root.join("Tasks");
    fs::create_dir_all(&work_root).expect("work root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");
    let stale_after = Duration::from_secs(300);

    // Dead worker: stale DB heartbeat, no state file.
    let mut dead = task_event(1, 1);
    dead.snapshot.last_heartbeat_at = Some(stale_stamp());
    dead.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&dead)
        .expect("dead-worker task must persist");

    // Alive but silent on stdout: DB heartbeat stale, yet the Python
    // TaskHeartbeat still touches work/state/<task>.json — must NOT be reaped.
    let mut silent = task_event_for("podcast-silent", TaskKind::Podcast, "book-6", 1, 1);
    silent.snapshot.last_heartbeat_at = Some(stale_stamp());
    silent.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&silent)
        .expect("silent task must persist");
    let state_dir = work_root.join("podcast-silent").join("work").join("state");
    fs::create_dir_all(&state_dir).expect("state dir must exist");
    let heartbeat = chrono::Local::now()
        .naive_local()
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    fs::write(
        state_dir.join("podcast-silent.json"),
        format!(
            r#"{{"task_id":"podcast-silent","status":"transcribing","last_heartbeat_at":"{heartbeat}","updated_at":"{heartbeat}","last_update_at":"{heartbeat}"}}"#
        ),
    )
    .expect("state file must write");

    // User-paused tasks suspend their worker on purpose — never reap.
    let mut paused = task_event_for("podcast-paused", TaskKind::Podcast, "book-7", 1, 1);
    paused.snapshot.lifecycle_state = LifecycleState::Paused;
    paused.snapshot.last_heartbeat_at = Some(stale_stamp());
    paused.snapshot.updated_at = stale_stamp();
    database
        .persist_task_event(&paused)
        .expect("paused task must persist");

    let reaped_events = database
        .reap_stale_workers(&work_root, stale_after)
        .expect("reaper must run");
    assert_eq!(reaped_events.len(), 1);
    assert_eq!(reaped_events[0].snapshot.id, "podcast-1");
    let reaped = database
        .task_snapshot("podcast-1")
        .expect("reaped snapshot must load")
        .expect("reaped snapshot must exist");
    assert_eq!(reaped.lifecycle_state, LifecycleState::Terminal);
    assert_eq!(reaped.outcome, TaskOutcome::Interrupted);
    assert_eq!(reaped.engine_stage, "stalled");
    assert!(reaped.recoverable);
    assert!(reaped.can_retry);
    let events = database
        .task_events("podcast-1", 1, 100)
        .expect("reaped events must load");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "worker_heartbeat_stale");
    for task_id in ["podcast-silent", "podcast-paused"] {
        assert_eq!(
            database
                .task_snapshot(task_id)
                .expect("snapshot must load")
                .expect("snapshot must exist")
                .lifecycle_state,
            if task_id == "podcast-paused" {
                LifecycleState::Paused
            } else {
                LifecycleState::Running
            },
            "{task_id} must not be reaped"
        );
    }

    // Idempotent: everything is either fresh or already terminal now.
    assert!(database
        .reap_stale_workers(&work_root, stale_after)
        .expect("second reap must be a no-op")
        .is_empty());
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P2-9: a claim left in-progress by a crash is re-seized once it ages past
/// the reclamation window — the retried request re-executes and completes
/// instead of replaying COMMAND_IN_PROGRESS forever. Fresh in-progress claims
/// and mismatched inputs are never reclaimed.
#[test]
fn abandoned_claim_is_reclaimed_after_timeout() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-claim-reclaim-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    assert!(matches!(
        database
            .claim_command("req-stale", "add_podcast_files", "input-a")
            .expect("claim must succeed"),
        CommandClaim::New
    ));
    // A live in-progress claim replays as Existing — the duplicate request
    // waits on the first executor's outcome rather than running twice.
    match database
        .claim_command("req-stale", "add_podcast_files", "input-a")
        .expect("duplicate claim must replay")
    {
        CommandClaim::Existing(record) => assert!(record.completed_at.is_none()),
        CommandClaim::New => panic!("fresh in-progress claim must not be reclaimed"),
    }

    // Age the claim past the reclamation window, as a crash leaves it.
    let stale = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    database
        .connection
        .execute(
            "UPDATE command_results SET created_at = ?2 WHERE request_id = ?1",
            rusqlite::params!["req-stale", stale],
        )
        .expect("claim timestamp must update");

    // A mismatched input can never reclaim — the id still belongs to the
    // original request.
    assert_eq!(
        database
            .claim_command("req-stale", "add_podcast_files", "other-input")
            .expect_err("input mismatch must still fail"),
        "IDEMPOTENCY_KEY_REUSED"
    );

    // The same request re-seizes the abandoned claim (New → re-execute).
    assert!(matches!(
        database
            .claim_command("req-stale", "add_podcast_files", "input-a")
            .expect("stale claim must be reclaimed"),
        CommandClaim::New
    ));
    // The refreshed claim is live: a concurrent duplicate sees it in-progress.
    assert!(matches!(
        database
            .claim_command("req-stale", "add_podcast_files", "input-a")
            .expect("fresh reclaimed claim must replay"),
        CommandClaim::Existing(_)
    ));
    database
        .complete_command("req-stale", r#"{"tasks":[]}"#, None, None)
        .expect("reclaimed command must complete");
    match database
        .claim_command("req-stale", "add_podcast_files", "input-a")
        .expect("completed claim must replay")
    {
        CommandClaim::Existing(record) => {
            assert_eq!(record.result_json.as_deref(), Some(r#"{"tasks":[]}"#));
            assert!(record.completed_at.is_some());
        }
        CommandClaim::New => panic!("completed request must not re-execute"),
    }
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P2-10: IMMEDIATE transactions take the reserved lock at BEGIN, so two
/// connections doing read-then-write degrade to an ordinary busy wait
/// covered by busy_timeout — under DEFERRED this intermittently died on
/// SQLITE_BUSY_SNAPSHOT, which busy_timeout never retries.
#[test]
fn concurrent_writers_do_not_hit_busy_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-immediate-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    ControlDb::open(&path).expect("schema must initialize");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|writer| {
            let barrier = Arc::clone(&barrier);
            let path = path.clone();
            thread::spawn(move || {
                let database = ControlDb::open(&path).expect("control database must open");
                barrier.wait();
                for step in 0..30_u64 {
                    database
                        .persist_task_event(&task_event_for(
                            &format!("task-{writer}"),
                            TaskKind::Podcast,
                            "book-writer",
                            step + 1,
                            step + 1,
                        ))
                        .expect("event must persist without busy snapshot");
                    // Exercise the second read-then-write transaction too.
                    let _ = database
                        .capture_cancel_discard()
                        .expect("cancel capture must not busy snapshot");
                    let request_id = format!("request-{writer}-{step}");
                    assert!(matches!(
                        database
                            .claim_command(&request_id, "command", "input")
                            .expect("claim must succeed"),
                        CommandClaim::New
                    ));
                    database
                        .complete_command(&request_id, "{}", None, None)
                        .expect("complete must succeed");
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("writer thread must finish");
    }
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P2-12: a healthy database at the current `user_version` opens without
/// re-running the schema batch; a version-behind or partially-missing schema
/// still bootstraps exactly once and heals.
#[test]
fn open_skips_schema_bootstrap_when_version_is_current() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-schema-gate-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");

    let database = ControlDb::open(&path).expect("control database must open");
    assert!(
        database.schema_bootstrap_ran(),
        "fresh database must run the schema batch"
    );
    drop(database);

    let reopened = ControlDb::open(&path).expect("control database must reopen");
    assert!(
        !reopened.schema_bootstrap_ran(),
        "versioned database must skip the schema batch"
    );
    let version: i64 = reopened
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user_version must read");
    // P3-25: the stamp comes from CONTROL_SCHEMA_VERSION, not a hardcoded
    // literal — bumping the const must propagate to newly written versions.
    assert_eq!(version, CONTROL_SCHEMA_VERSION);
    drop(reopened);

    // Rewinding the version re-runs the batch once (upgrade path).
    {
        let behind = ControlDb::open(&path).expect("control database must reopen");
        behind
            .connection
            .execute_batch("PRAGMA user_version = 0")
            .expect("user_version must rewind");
    }
    let upgraded = ControlDb::open(&path).expect("control database must reopen");
    assert!(
        upgraded.schema_bootstrap_ran(),
        "version-behind database must re-run the schema batch"
    );
    drop(upgraded);

    // A database whose schema partially vanished bootstraps again and heals —
    // the version stamp alone cannot prove the tables exist.
    {
        let broken = ControlDb::open(&path).expect("control database must reopen");
        assert!(!broken.schema_bootstrap_ran());
        broken
            .connection
            .execute_batch("DROP TABLE command_results")
            .expect("sentinel table must drop");
    }
    let healed = ControlDb::open(&path).expect("control database must reopen");
    assert!(
        healed.schema_bootstrap_ran(),
        "missing sentinel table must re-run the schema batch"
    );
    assert!(healed
        .table_names()
        .expect("tables must list")
        .contains(&"command_results".to_string()));
    drop(healed);

    // P3-25: a database stamped by a NEWER build must never be downgraded —
    // opening it at all would risk reading a format this build cannot
    // understand, so it is refused outright.
    {
        let newer = ControlDb::open(&path).expect("control database must reopen");
        newer
            .connection
            .execute_batch("PRAGMA user_version = 99")
            .expect("user_version must set");
    }
    match ControlDb::open(&path) {
        Err(error) => assert!(
            error.starts_with("CONTROL_SCHEMA_TOO_NEW"),
            "future version must be rejected with CONTROL_SCHEMA_TOO_NEW, got {error}"
        ),
        Ok(_) => panic!("future-version database must not open"),
    }
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-24/P3-25: a v1 database carrying the never-written `cache_leases`
/// table upgrades once — the batch drops it and stamps the new version.
#[test]
fn schema_v2_upgrade_drops_dead_cache_leases() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-schema-v2-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    {
        let database = ControlDb::open(&path).expect("control database must open");
        // Recreate what a v1 database looked like: dead table present,
        // version stamped 1.
        database
            .connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS cache_leases (
                  task_id TEXT PRIMARY KEY NOT NULL,
                  cache_relative_path TEXT NOT NULL,
                  reason TEXT NOT NULL,
                  bytes INTEGER NOT NULL,
                  held INTEGER NOT NULL,
                  updated_at TEXT NOT NULL
                );
                PRAGMA user_version = 1;",
            )
            .expect("v1 fixture must write");
    }
    let upgraded = ControlDb::open(&path).expect("control database must reopen");
    assert!(
        upgraded.schema_bootstrap_ran(),
        "v1 database must run the v2 upgrade batch"
    );
    let tables = upgraded.table_names().expect("tables must list");
    assert!(
        !tables.contains(&"cache_leases".to_string()),
        "v2 upgrade must drop dead cache_leases"
    );
    let version: i64 = upgraded
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user_version must read");
    assert_eq!(version, CONTROL_SCHEMA_VERSION);
    drop(upgraded);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

const THIRTY_DAYS_SECS: u64 = 30 * 24 * 60 * 60;

fn table_count(database: &ControlDb, table: &str) -> i64 {
    database
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("row count must load")
}

fn row_exists(database: &ControlDb, table: &str, key_column: &str, key: &str) -> bool {
    database
        .connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key_column} = ?1)"),
            [key],
            |row| row.get::<_, i64>(0),
        )
        .expect("existence must load")
        == 1
}

fn old_stamp(index: u32) -> String {
    format!("2020-01-01T00:00:{index:02}Z")
}

/// P3-25: settled command rows past the retention window are purged while
/// in-progress claims, recent rows and the newest `keep_latest_n` settled
/// rows always survive.
#[test]
fn purge_completed_commands_keeps_in_progress_recent_and_latest_n() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-purge-commands-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    for index in 0..5_u32 {
        let request_id = format!("old-{index}");
        assert!(matches!(
            database
                .claim_command(&request_id, "command", "input")
                .expect("claim must succeed"),
            CommandClaim::New
        ));
        database
            .complete_command(&request_id, "{}", None, None)
            .expect("complete must succeed");
        database
            .connection
            .execute(
                "UPDATE command_results SET completed_at = ?2 WHERE request_id = ?1",
                rusqlite::params![request_id, old_stamp(index)],
            )
            .expect("age stamp must update");
    }
    // One recent settled row and one aged in-progress claim — both survive:
    // the former is inside the window, the latter is owned by
    // abandoned-claim reclamation, not retention.
    database
        .claim_command("recent", "command", "input")
        .expect("claim must succeed");
    database
        .complete_command("recent", "{}", None, None)
        .expect("complete must succeed");
    database
        .claim_command("live", "command", "input")
        .expect("claim must succeed");
    database
        .connection
        .execute(
            "UPDATE command_results SET created_at = ?2 WHERE request_id = ?1",
            rusqlite::params!["live", old_stamp(0)],
        )
        .expect("age stamp must update");

    // Settled rows newest-first: recent, old-4, old-3, … — keeping the
    // latest 2 protects {recent, old-4}; the other four old rows go.
    assert_eq!(
        database
            .purge_completed_commands(THIRTY_DAYS_SECS, 2)
            .expect("purge must run"),
        4
    );
    assert_eq!(table_count(&database, "command_results"), 3);
    for kept in ["recent", "old-4", "live"] {
        assert!(
            row_exists(&database, "command_results", "request_id", kept),
            "{kept} must survive retention"
        );
    }
    for purged in ["old-0", "old-1", "old-2", "old-3"] {
        assert!(
            !row_exists(&database, "command_results", "request_id", purged),
            "{purged} must be purged"
        );
    }

    // A zero window with zero keep deletes every settled row; the
    // in-progress claim is still untouchable.
    assert_eq!(
        database
            .purge_completed_commands(0, 0)
            .expect("purge must run"),
        2
    );
    assert_eq!(table_count(&database, "command_results"), 1);
    assert!(row_exists(
        &database,
        "command_results",
        "request_id",
        "live"
    ));
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-25: finished migration runs age out; a `running` row (live or crashed
/// migration) is never purged no matter how old.
#[test]
fn purge_finished_migration_runs_keeps_running_rows() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-purge-migrations-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    for index in 0..3_u32 {
        let migration_id = format!("m-{index}");
        database
            .begin_migration_run(&migration_id, "preview", "settings")
            .expect("run must start");
        database
            .complete_migration_run(&migration_id, "success", None, "{}")
            .expect("run must complete");
        database
            .connection
            .execute(
                "UPDATE migration_runs SET created_at = ?2, completed_at = ?2 WHERE migration_id = ?1",
                rusqlite::params![migration_id, old_stamp(index)],
            )
            .expect("age stamp must update");
    }
    database
        .begin_migration_run("m-running", "preview", "settings")
        .expect("run must start");
    database
        .connection
        .execute(
            "UPDATE migration_runs SET created_at = ?2 WHERE migration_id = ?1",
            rusqlite::params!["m-running", old_stamp(0)],
        )
        .expect("age stamp must update");
    database
        .begin_migration_run("m-recent", "preview", "settings")
        .expect("run must start");
    database
        .complete_migration_run("m-recent", "success", None, "{}")
        .expect("run must complete");

    // Settled newest-first: m-recent, m-2, m-1, m-0 — keep 2.
    assert_eq!(
        database
            .purge_finished_migration_runs(THIRTY_DAYS_SECS, 2)
            .expect("purge must run"),
        2
    );
    assert_eq!(table_count(&database, "migration_runs"), 3);
    for kept in ["m-recent", "m-2", "m-running"] {
        assert!(
            row_exists(&database, "migration_runs", "migration_id", kept),
            "{kept} must survive retention"
        );
    }
    for purged in ["m-0", "m-1"] {
        assert!(
            !row_exists(&database, "migration_runs", "migration_id", purged),
            "{purged} must be purged"
        );
    }
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-24 + P3-25: `publish_transaction_index` has a real read path
/// (in-flight rows for crash recovery), and retention purges only the
/// settled phases — an aged `prepared` row is never deleted.
#[test]
fn purge_settled_publish_transactions_keeps_in_flight_rows() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-purge-publish-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    for index in 0..3_u32 {
        let transaction_id = format!("txn-{index}");
        database
            .record_publish_transaction(
                &transaction_id,
                &format!("task-{index}"),
                &format!("book-{index}"),
                "committed",
                &format!(".transactions/{transaction_id}.json"),
            )
            .expect("publish index must record");
        database
            .connection
            .execute(
                "UPDATE publish_transaction_index SET updated_at = ?2 WHERE transaction_id = ?1",
                rusqlite::params![transaction_id, old_stamp(index)],
            )
            .expect("age stamp must update");
    }
    database
        .record_publish_transaction(
            "txn-live",
            "task-live",
            "book-live",
            "prepared",
            ".transactions/txn-live.json",
        )
        .expect("publish index must record");
    database
        .connection
        .execute(
            "UPDATE publish_transaction_index SET updated_at = ?2 WHERE transaction_id = ?1",
            rusqlite::params!["txn-live", old_stamp(0)],
        )
        .expect("age stamp must update");
    database
        .record_publish_transaction(
            "txn-recent",
            "task-recent",
            "book-recent",
            "committed",
            ".transactions/txn-recent.json",
        )
        .expect("publish index must record");

    // P3-24 read path: only the in-flight row is unfinished.
    let unfinished = database
        .unfinished_publish_transactions()
        .expect("unfinished publishes must load");
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].transaction_id, "txn-live");
    assert_eq!(unfinished[0].phase, "prepared");
    assert_eq!(
        unfinished[0].journal_relative_path,
        ".transactions/txn-live.json"
    );

    // Settled newest-first: txn-recent, txn-2, txn-1, txn-0 — keep 2.
    assert_eq!(
        database
            .purge_settled_publish_transactions(THIRTY_DAYS_SECS, 2)
            .expect("purge must run"),
        2
    );
    assert_eq!(table_count(&database, "publish_transaction_index"), 3);
    for kept in ["txn-recent", "txn-2", "txn-live"] {
        assert!(
            row_exists(
                &database,
                "publish_transaction_index",
                "transaction_id",
                kept
            ),
            "{kept} must survive retention"
        );
    }
    for purged in ["txn-0", "txn-1"] {
        assert!(
            !row_exists(
                &database,
                "publish_transaction_index",
                "transaction_id",
                purged
            ),
            "{purged} must be purged"
        );
    }
    // The in-flight row still reads back after retention.
    assert_eq!(
        database
            .unfinished_publish_transactions()
            .expect("unfinished publishes must reload")
            .len(),
        1
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-25: completed cancel-discard intents age out; `pending` rows are the
/// durable intent retried after restart and are never purged.
#[test]
fn purge_completed_cancel_intents_keeps_pending_rows() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-purge-intents-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    for index in 0..3_u32 {
        database
            .connection
            .execute(
                "INSERT INTO cancel_discard_intents(task_id, state, created_at, updated_at) VALUES (?1, 'completed', ?2, ?2)",
                rusqlite::params![format!("t-comp-{index}"), old_stamp(index)],
            )
            .expect("completed intent must insert");
    }
    for (task_id, state) in [("t-pending", "pending"), ("t-recent", "completed")] {
        database
            .connection
            .execute(
                "INSERT INTO cancel_discard_intents(task_id, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                rusqlite::params![task_id, state, chrono::Utc::now().to_rfc3339()],
            )
            .expect("intent must insert");
    }
    database
        .connection
        .execute(
            "UPDATE cancel_discard_intents SET created_at = ?2, updated_at = ?2 WHERE task_id = ?1",
            rusqlite::params!["t-pending", old_stamp(0)],
        )
        .expect("age stamp must update");

    // Settled newest-first: t-recent, t-comp-2, t-comp-1, t-comp-0 — keep 2.
    assert_eq!(
        database
            .purge_completed_cancel_intents(THIRTY_DAYS_SECS, 2)
            .expect("purge must run"),
        2
    );
    assert_eq!(table_count(&database, "cancel_discard_intents"), 3);
    for kept in ["t-recent", "t-comp-2", "t-pending"] {
        assert!(
            row_exists(&database, "cancel_discard_intents", "task_id", kept),
            "{kept} must survive retention"
        );
    }
    for purged in ["t-comp-0", "t-comp-1"] {
        assert!(
            !row_exists(&database, "cancel_discard_intents", "task_id", purged),
            "{purged} must be purged"
        );
    }
    // The aged pending intent is still what startup reconciliation sees.
    assert_eq!(
        database
            .pending_cancel_discard()
            .expect("pending intents must load"),
        vec!["t-pending".to_string()]
    );
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-25: `run_maintenance` applies the settled-row retention policy to all
/// four append-only tables at once, keeps every live row, and is idempotent.
#[test]
fn run_maintenance_purges_settled_rows_and_keeps_live_ones() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-maintenance-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");
    let old = old_stamp(0);

    // 105 settled rows per table → SETTLED_KEEP_LATEST (100) survive.
    for index in 0..105_u32 {
        database
            .connection
            .execute(
                "INSERT INTO command_results(request_id, command_name, input_hash, created_at, completed_at) VALUES (?1, 'command', 'input', ?2, ?2)",
                rusqlite::params![format!("settled-cmd-{index}"), old],
            )
            .expect("settled command must insert");
        database
            .connection
            .execute(
                "INSERT INTO migration_runs(migration_id, preview_id, scope, status, created_at, completed_at) VALUES (?1, 'preview', 'settings', 'success', ?2, ?2)",
                rusqlite::params![format!("settled-mig-{index}"), old],
            )
            .expect("settled migration must insert");
        database
            .connection
            .execute(
                "INSERT INTO publish_transaction_index(transaction_id, task_id, book_id, phase, journal_relative_path, updated_at) VALUES (?1, 'task', 'book', 'committed', '.transactions/j.json', ?2)",
                rusqlite::params![format!("settled-txn-{index}"), old],
            )
            .expect("settled publish must insert");
        database
            .connection
            .execute(
                "INSERT INTO cancel_discard_intents(task_id, state, created_at, updated_at) VALUES (?1, 'completed', ?2, ?2)",
                rusqlite::params![format!("settled-intent-{index}"), old],
            )
            .expect("settled intent must insert");
    }
    // One live row per table — aged but unsettled, so retention must keep it.
    database
        .connection
        .execute(
            "INSERT INTO command_results(request_id, command_name, input_hash, created_at) VALUES ('live-cmd', 'command', 'input', ?1)",
            rusqlite::params![old],
        )
        .expect("live command must insert");
    database
        .connection
        .execute(
            "INSERT INTO migration_runs(migration_id, preview_id, scope, status, created_at) VALUES ('live-mig', 'preview', 'settings', 'running', ?1)",
            rusqlite::params![old],
        )
        .expect("running migration must insert");
    database
        .connection
        .execute(
            "INSERT INTO publish_transaction_index(transaction_id, task_id, book_id, phase, journal_relative_path, updated_at) VALUES ('live-txn', 'task', 'book', 'prepared', '.transactions/live.json', ?1)",
            rusqlite::params![old],
        )
        .expect("in-flight publish must insert");
    database
        .connection
        .execute(
            "INSERT INTO cancel_discard_intents(task_id, state, created_at, updated_at) VALUES ('live-intent', 'pending', ?1, ?1)",
            rusqlite::params![old],
        )
        .expect("pending intent must insert");

    let report = database.run_maintenance().expect("maintenance must run");
    assert_eq!(report.purged_commands, 5);
    assert_eq!(report.purged_migration_runs, 5);
    assert_eq!(report.purged_publish_transactions, 5);
    assert_eq!(report.purged_cancel_intents, 5);
    assert!(
        !report.vacuumed,
        "tiny freed page count must stay under the VACUUM gate"
    );
    for (table, key, live) in [
        ("command_results", "request_id", "live-cmd"),
        ("migration_runs", "migration_id", "live-mig"),
        ("publish_transaction_index", "transaction_id", "live-txn"),
        ("cancel_discard_intents", "task_id", "live-intent"),
    ] {
        assert_eq!(table_count(&database, table), 101, "{table} count");
        assert!(
            row_exists(&database, table, key, live),
            "{live} must survive"
        );
    }

    // Idempotent: a second pass finds nothing left to do.
    let second = database.run_maintenance().expect("maintenance must re-run");
    assert_eq!(second.purged_commands, 0);
    assert_eq!(second.purged_migration_runs, 0);
    assert_eq!(second.purged_publish_transactions, 0);
    assert_eq!(second.purged_cancel_intents, 0);
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

/// P3-25: when a purge frees a meaningful share of pages, `run_maintenance`
/// follows up with a VACUUM so the file actually shrinks.
#[test]
fn run_maintenance_vacuums_when_freelist_is_large() {
    let root =
        std::env::temp_dir().join(format!("immersive-control-vacuum-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let database = ControlDb::open(&root.join("control.db")).expect("control database must open");

    // ~160 settled rows carrying ~4 KiB results ≈ 160+ pages — comfortably
    // past the 32-page / 20%-free VACUUM gate once deleted.
    let blob = "x".repeat(4096);
    for index in 0..160_u32 {
        database
            .connection
            .execute(
                "INSERT INTO command_results(request_id, command_name, input_hash, created_at, completed_at, result_json) VALUES (?1, 'command', 'input', ?2, ?2, ?3)",
                rusqlite::params![format!("bulk-{index}"), old_stamp(0), blob],
            )
            .expect("bulk command must insert");
    }
    assert_eq!(
        database
            .purge_completed_commands(0, 0)
            .expect("settled rows must purge"),
        160
    );
    let freelist_before: i64 = database
        .connection
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .expect("freelist must read");
    assert!(
        freelist_before >= 32,
        "fixture must free enough pages to test the gate, got {freelist_before}"
    );

    let report = database.run_maintenance().expect("maintenance must run");
    assert!(report.vacuumed, "large freelist must trigger VACUUM");
    let freelist_after: i64 = database
        .connection
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .expect("freelist must read");
    assert_eq!(freelist_after, 0, "VACUUM must reclaim the freelist");
    drop(database);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
