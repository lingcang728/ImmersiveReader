use super::{PodcastFilesPreview, PodcastPreviewOptions};
use crate::control::CommandRecord;
use crate::tasks::TaskSnapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct StoredPreview {
    pub preview: PodcastFilesPreview,
    pub options: PodcastPreviewOptions,
}

#[derive(Clone, Default)]
pub struct PodcastPreviewStore {
    previews: Arc<Mutex<HashMap<String, StoredPreview>>>,
}

impl PodcastPreviewStore {
    pub fn insert(
        &self,
        preview: PodcastFilesPreview,
        options: PodcastPreviewOptions,
    ) -> Result<(), String> {
        if preview.preview_id.is_empty() {
            return Err("INVALID_ARGUMENT".to_string());
        }
        self.previews
            .lock()
            .map_err(|_| "Podcast preview store is unavailable".to_string())?
            .insert(
                preview.preview_id.clone(),
                StoredPreview { preview, options },
            );
        Ok(())
    }

    pub(super) fn get(&self, preview_id: &str) -> Result<StoredPreview, String> {
        self.previews
            .lock()
            .map_err(|_| "Podcast preview store is unavailable".to_string())?
            .get(preview_id)
            .cloned()
            .ok_or_else(|| "PODCAST_PREVIEW_STALE".to_string())
    }

    pub(super) fn remove(&self, preview_id: &str) {
        if let Ok(mut previews) = self.previews.lock() {
            previews.remove(preview_id);
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicatePolicy {
    ReuseExisting,
    NewRevision,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastBudgetApproval {
    pub estimated_disk_bytes: u64,
    pub estimated_api_cost_upper_cny: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastAddResult {
    pub tasks: Vec<TaskSnapshot>,
    pub existing_books: Vec<String>,
}

pub struct AddPodcastFilesRequest<'a> {
    pub preview_id: &'a str,
    pub duplicate_policy: DuplicatePolicy,
    pub budget_approval: Option<&'a PodcastBudgetApproval>,
    pub request_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddRequestFingerprint<'a> {
    preview_id: &'a str,
    duplicate_policy: DuplicatePolicy,
    budget_approval: Option<&'a PodcastBudgetApproval>,
}

pub(super) fn request_hash(request: &AddPodcastFilesRequest<'_>) -> Result<String, String> {
    let input = AddRequestFingerprint {
        preview_id: request.preview_id,
        duplicate_policy: request.duplicate_policy,
        budget_approval: request.budget_approval,
    };
    let bytes = serde_json::to_vec(&input).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn replay(record: CommandRecord) -> Result<PodcastAddResult, String> {
    if let Some(error_code) = record.error_code {
        return Err(error_code);
    }
    let result = record
        .result_json
        .ok_or_else(|| "COMMAND_IN_PROGRESS".to_string())?;
    serde_json::from_str(&result).map_err(|error| error.to_string())
}

pub(super) fn validate_budget(
    stored: &StoredPreview,
    approval: Option<&PodcastBudgetApproval>,
) -> Result<(), String> {
    if stored.preview.budget.estimated_disk_bytes > stored.preview.budget.available_disk_bytes {
        return Err("INSUFFICIENT_DISK".to_string());
    }
    if stored.preview.budget.estimated_api_cost_upper_cny <= stored.options.max_api_cost_cny {
        return Ok(());
    }
    let approval = approval.ok_or_else(|| "BUDGET_CONFIRMATION_REQUIRED".to_string())?;
    if approval.estimated_disk_bytes < stored.preview.budget.estimated_disk_bytes
        || approval.estimated_api_cost_upper_cny
            < stored.preview.budget.estimated_api_cost_upper_cny
    {
        return Err("BUDGET_CONFIRMATION_REQUIRED".to_string());
    }
    Ok(())
}
