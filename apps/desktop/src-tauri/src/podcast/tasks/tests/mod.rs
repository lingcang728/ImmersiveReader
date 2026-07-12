use super::{AddPodcastFilesRequest, DuplicatePolicy, PodcastPreviewStore};
use crate::podcast::{
    PodcastBudgetApproval, PodcastBudgetPreview, PodcastFilePreview, PodcastFilesPreview,
    PodcastPreviewOptions,
};
use crate::storage::StorageLocations;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> (PathBuf, StorageLocations, PodcastFilesPreview) {
    let root = std::env::temp_dir().join(format!(
        "immersive-podcast-task-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let source = root.join("source.wav");
    fs::create_dir_all(&root).expect("fixture root must exist");
    fs::write(&source, b"verified-audio").expect("source must exist");
    let input_sha256 = format!("{:x}", Sha256::digest(b"verified-audio"));
    let runtime_root = root.join("Runtime");
    let engine = runtime_root.join(r"podcast\app\scripts\transcribe_task.py");
    fs::create_dir_all(engine.parent().expect("engine parent must exist"))
        .expect("engine parent must be created");
    fs::write(engine, b"engine-script").expect("engine script must exist");
    let model_config =
        runtime_root.join(r"podcast\models\faster-whisper-large-v3-turbo-local\config.json");
    fs::create_dir_all(model_config.parent().expect("model parent must exist"))
        .expect("model parent must be created");
    fs::write(model_config, b"model-config").expect("model config must exist");
    let locations = StorageLocations {
        channel: "test".to_string(),
        settings_path: root.join(r"Settings\settings.json"),
        data_root: root.join("Data"),
        cache_root: root.join("Cache"),
        logs_root: root.join("Logs"),
        runtime_state_root: root.join("RuntimeState"),
        backups_root: root.join("Backups"),
        library_root: root.join("Library"),
        runtime_root,
    };
    let preview = PodcastFilesPreview {
        preview_id: "preview-1".to_string(),
        files: vec![PodcastFilePreview {
            path: source.to_string_lossy().into_owned(),
            file_name: "source.wav".to_string(),
            bytes: 14,
            duration_seconds: 12.0,
            input_sha256: input_sha256.clone(),
            source_id: input_sha256.clone(),
            book_id: format!("podcast:{input_sha256}"),
            duplicate_book_id: None,
        }],
        budget: PodcastBudgetPreview {
            estimated_disk_bytes: 1024,
            estimated_translation_tokens: 100,
            estimated_api_cost_upper_cny: 0.1,
            available_disk_bytes: 2048,
            estimate_version: "test-estimate".to_string(),
            confirmation_required: true,
        },
    };
    (root, locations, preview)
}

fn approved() -> PodcastBudgetApproval {
    PodcastBudgetApproval {
        estimated_disk_bytes: 1024,
        estimated_api_cost_upper_cny: 0.1,
    }
}

fn request<'a>(
    duplicate_policy: DuplicatePolicy,
    budget_approval: Option<&'a PodcastBudgetApproval>,
    request_id: &'a str,
) -> AddPodcastFilesRequest<'a> {
    AddPodcastFilesRequest {
        preview_id: "preview-1",
        duplicate_policy,
        budget_approval,
        request_id,
    }
}

mod creation;
mod policy;
