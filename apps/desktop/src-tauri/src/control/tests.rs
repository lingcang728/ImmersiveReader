use super::{CommandClaim, ControlDb};
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome,
    TaskProgress, TaskSnapshot,
};
use std::fs;

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

fn task_event(sequence: u64, revision: u64) -> TaskEvent {
    let now = "2026-07-11T12:00:00Z".to_string();
    let snapshot = TaskSnapshot {
        id: "podcast-1".to_string(),
        kind: TaskKind::Podcast,
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
        book_id: None,
        source_id: Some("sha256".to_string()),
        cache_lease_bytes: 42,
        created_at: now.clone(),
        updated_at: now.clone(),
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
        .record_worker_line("podcast-1", "stdout", "[ 42.50%] transcribing chunk")
        .expect("stdout must map")
        .expect("stdout event must exist");
    assert_eq!(stdout.event_type, "worker_stdout");
    assert_eq!(stdout.snapshot.progress.percent, Some(42.5));
    assert_eq!(stdout.snapshot.engine_stage, "chunking");
    let stderr = database
        .record_worker_line("podcast-1", "stderr", "worker warning")
        .expect("stderr must map")
        .expect("stderr event must exist");
    assert_eq!(stderr.event_type, "worker_stderr");
    assert_eq!(
        stderr.snapshot.error_message.as_deref(),
        Some("worker warning")
    );
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
