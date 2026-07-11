use super::{load_from, AppSettings};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRecovery {
    pub source_path: PathBuf,
    pub error: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum SettingsLoadState {
    Active(AppSettings),
    Recovery(SettingsRecovery),
}

pub(super) fn load_status_from(path: &Path) -> SettingsLoadState {
    match load_from(path) {
        Ok(settings) => SettingsLoadState::Active(settings),
        Err(error) => SettingsLoadState::Recovery(SettingsRecovery {
            source_path: path.to_path_buf(),
            error,
            read_only: true,
        }),
    }
}
