use super::*;
use crate::control::ControlDb;

#[test]
fn queued_task_is_persisted_before_broadcast() {
    let (root, locations, preview) = fixture("broadcast");
    let control_path = root.join("control.db");
    let mut control = ControlDb::open(&control_path).expect("control database must open");
    let store = PodcastPreviewStore::default();
    store
        .insert(
            preview,
            PodcastPreviewOptions {
                translate: true,
            polish: true,
                max_api_cost_cny: 0.0,
            },
        )
        .expect("preview must be stored");
    let mut broadcasts = 0_u32;
    let approval = approved();

    let result = super::super::add_podcast_files_at(
        &store,
        &mut control,
        &locations,
        &request(DuplicatePolicy::NewRevision, Some(&approval), "request-1"),
        |event| {
            let verification =
                ControlDb::open(&control_path).expect("verification database must open");
            assert!(verification
                .task_snapshot(&event.task_id)
                .expect("snapshot lookup must succeed")
                .is_some());
            broadcasts += 1;
        },
    )
    .expect("queued task must be created");

    assert_eq!(result.tasks.len(), 1);
    // Preparing + progress + ready (at least the first and last snapshots).
    assert!(broadcasts >= 2, "expected prepare+ready broadcasts, got {broadcasts}");
    let task_id = &result.tasks[0].id;
    let data_task = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    let task_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(data_task.join("task.json")).expect("task spec must be readable"),
    )
    .expect("task spec must be valid JSON");
    assert_eq!(task_json["schemaVersion"], 2);
    assert_eq!(task_json["options"]["translate"], true);
    assert_eq!(task_json["options"]["polish"], true);
    assert_eq!(task_json["taskId"], task_id.as_str());
    assert_eq!(task_json["options"]["budgetLimitCny"], 0.1);
    assert_eq!(
        task_json["input"]["inputSha256"],
        result.tasks[0]
            .source_id
            .clone()
            .expect("source id must exist")
    );
    for field in [
        "pipelineVersion",
        "engineVersion",
        "configHash",
        "modelHash",
    ] {
        assert!(task_json["compatibility"][field].is_string());
    }
    assert!(locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id)
        .join(r"input\source.wav")
        .is_file());
    let recovery =
        fs::read_to_string(data_task.join("recovery.json")).expect("recovery must be readable");
    assert!(recovery.contains("pipelineVersion"));
    assert!(recovery.contains("modelHash"));
    drop(control);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn completed_request_replays_without_copying_or_broadcasting() {
    let (root, locations, preview) = fixture("replay");
    let control_path = root.join("control.db");
    let store = PodcastPreviewStore::default();
    store
        .insert(
            preview,
            PodcastPreviewOptions {
                translate: false,
            polish: true,
                max_api_cost_cny: 0.0,
            },
        )
        .expect("preview must be stored");
    let first = {
        let mut control = ControlDb::open(&control_path).expect("control database must open");
        let approval = approved();
        super::super::add_podcast_files_at(
            &store,
            &mut control,
            &locations,
            &request(DuplicatePolicy::NewRevision, Some(&approval), "request-1"),
            |_| {},
        )
        .expect("first request must succeed")
    };
    let mut replay_broadcasts = 0_u32;
    let replay = {
        let mut reopened = ControlDb::open(&control_path).expect("control database must reopen");
        let approval = approved();
        super::super::add_podcast_files_at(
            &store,
            &mut reopened,
            &locations,
            &request(DuplicatePolicy::NewRevision, Some(&approval), "request-1"),
            |_| replay_broadcasts += 1,
        )
        .expect("completed request must replay")
    };

    assert_eq!(replay, first);
    assert_eq!(replay_broadcasts, 0);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
