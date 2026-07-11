use super::{CommandClaim, ControlDb};
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskEvent, TaskKind, TaskOutcome, TaskProgress,
    TaskSnapshot,
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
