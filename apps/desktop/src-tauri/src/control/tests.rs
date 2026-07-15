use super::{CommandClaim, ControlDb};
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome,
    TaskProgress, TaskSnapshot,
};
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

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
        "cache_leases",
        "engine_instances",
        "publish_transaction_index",
        "migration_runs",
        "cancel_discard_intents",
    ] {
        assert!(tables.contains(&expected.to_string()), "missing {expected}");
    }
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
        let mut database = ControlDb::open(&path).expect("control database must open");
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
fn podcast_and_zhihu_active_snapshots_can_coexist() {
    let root = std::env::temp_dir().join(format!(
        "immersive-control-parallel-tasks-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let path = root.join("control.db");
    let mut database = ControlDb::open(&path).expect("control database must open");
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
        snapshot.lifecycle_state == LifecycleState::Running
            && snapshot.outcome == TaskOutcome::None
    }));
    assert!(active.iter().any(|snapshot| {
        snapshot.id == "podcast-active" && snapshot.kind == TaskKind::Podcast
    }));
    assert!(active.iter().any(|snapshot| {
        snapshot.id == "zhihu-active" && snapshot.kind == TaskKind::Zhihu
    }));

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
    let mut database =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
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
        let mut database = ControlDb::open(&path).expect("control database must open");
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
    assert_eq!(stdout.snapshot.progress.label.as_deref(), Some("正在切分音频"));
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
    assert_eq!(ndjson.snapshot.progress.label.as_deref(), Some("正在语音转写"));
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
    let mut database = ControlDb::open(&root.join("control.db")).expect("control database must open");
    let mut queued = task_event_for(
        "zhihu-start",
        TaskKind::Zhihu,
        "zhihu:author",
        1,
        1,
    );
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
