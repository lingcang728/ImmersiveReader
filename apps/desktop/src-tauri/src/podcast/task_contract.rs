use super::{
    DuplicatePolicy, PodcastBudgetPreview, PodcastFilePreview, PodcastPreviewOptions,
    VerifiedPodcastInput,
};
use crate::cache::PodcastCompatibility;
use crate::storage::StorageLocations;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;

const PIPELINE_VERSION: &str = "podcast-pipeline-v2";
const TASK_SPEC_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskInput<'a> {
    relative_path: &'a str,
    input_sha256: &'a str,
    bytes: u64,
    duration_seconds: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskCompatibility<'a> {
    pipeline_version: &'a str,
    engine_version: &'a str,
    config_hash: &'a str,
    model_hash: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskOptions {
    translate: bool,
    polish: bool,
    duplicate_policy: DuplicatePolicy,
    max_api_cost_cny: f64,
    budget_limit_cny: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskPublish<'a> {
    book_id: &'a str,
    source_id: &'a str,
    revision: u64,
    incoming_relative_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PodcastTaskSpec<'a> {
    schema_version: u32,
    task_id: &'a str,
    input: TaskInput<'a>,
    compatibility: TaskCompatibility<'a>,
    options: TaskOptions,
    budget: &'a PodcastBudgetPreview,
    publish: TaskPublish<'a>,
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn compatibility_for(
    locations: &StorageLocations,
    input_sha256: &str,
    options: &PodcastPreviewOptions,
) -> Result<PodcastCompatibility, String> {
    let engine_path = locations
        .runtime_root
        .join(r"podcast\app\scripts\transcribe_task.py");
    let model_path = locations
        .runtime_root
        .join(r"podcast\models\faster-whisper-large-v3-turbo-local\config.json");
    if !engine_path.is_file() || !model_path.is_file() {
        return Err("RUNTIME_UNAVAILABLE".to_string());
    }
    Ok(PodcastCompatibility {
        input_sha256: input_sha256.to_string(),
        pipeline_version: PIPELINE_VERSION.to_string(),
        engine_version: crate::publish::hash_file(&engine_path)?,
        config_hash: hash_json(options)?,
        model_hash: crate::publish::hash_file(&model_path)?,
    })
}

pub(super) struct TaskContractRequest<'a> {
    pub task_id: &'a str,
    pub file: &'a PodcastFilePreview,
    pub input: &'a VerifiedPodcastInput,
    pub options: &'a PodcastPreviewOptions,
    pub duplicate_policy: DuplicatePolicy,
    pub budget: &'a PodcastBudgetPreview,
    pub budget_approval: Option<&'a super::PodcastBudgetApproval>,
    pub revision: u64,
}

pub(super) fn write_task_contract(
    locations: &StorageLocations,
    request: &TaskContractRequest<'_>,
) -> Result<PodcastCompatibility, String> {
    let compatibility = compatibility_for(locations, &request.input.input_sha256, request.options)?;
    let spec = PodcastTaskSpec {
        schema_version: TASK_SPEC_SCHEMA_VERSION,
        task_id: request.task_id,
        input: TaskInput {
            relative_path: &request.input.relative_path,
            input_sha256: &request.input.input_sha256,
            bytes: request.input.bytes,
            duration_seconds: request.file.duration_seconds,
        },
        compatibility: TaskCompatibility {
            pipeline_version: &compatibility.pipeline_version,
            engine_version: &compatibility.engine_version,
            config_hash: &compatibility.config_hash,
            model_hash: &compatibility.model_hash,
        },
        options: TaskOptions {
            translate: request.options.translate,
            polish: request.options.polish,
            duplicate_policy: request.duplicate_policy,
            max_api_cost_cny: request.options.max_api_cost_cny,
            budget_limit_cny: request
                .budget_approval
                .map(|approval| approval.estimated_api_cost_upper_cny)
                .unwrap_or(request.options.max_api_cost_cny),
        },
        budget: request.budget,
        publish: TaskPublish {
            book_id: &request.file.book_id,
            source_id: &request.file.source_id,
            revision: request.revision,
            incoming_relative_path: format!(".incoming/{}", request.task_id),
        },
    };
    let task_root = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(request.task_id);
    fs::create_dir_all(&task_root).map_err(|error| error.to_string())?;
    let data = serde_json::to_vec_pretty(&spec).map_err(|error| error.to_string())?;
    crate::atomic_file::write(&task_root.join("task.json"), &data)?;
    Ok(compatibility)
}
