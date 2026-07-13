use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppChannel {
    Production,
    Qa(String),
}

impl AppChannel {
    /// Detects the runtime channel. Only `qa_run_id` selects a non-production
    /// channel now that the standalone development build has been removed: the
    /// production executable is the only thing that ships. The QA channel runs
    /// that same production executable with `IMMERSIVE_QA_RUN_ID` set so its
    /// data roots stay isolated from real user data.
    pub fn detect(qa_run_id: Option<&str>) -> Result<Self, String> {
        if let Some(run_id) = qa_run_id {
            if run_id.is_empty()
                || !run_id
                    .bytes()
                    .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
            {
                return Err("QA run id must contain only letters, digits, '-' or '_'".to_string());
            }
            return Ok(Self::Qa(run_id.to_string()));
        }
        Ok(Self::Production)
    }

    pub fn current() -> Self {
        match Self::detect(None) {
            Ok(channel) => channel,
            Err(_) => Self::Production,
        }
    }

    pub fn settings_directory_name(&self) -> String {
        match self {
            Self::Production => "immersive-reader".to_string(),
            Self::Qa(run_id) => format!("ImmersiveReader-QA-{run_id}"),
        }
    }

    pub fn local_data_directory_name(&self) -> String {
        match self {
            Self::Production => "ImmersiveReader".to_string(),
            Self::Qa(run_id) => format!(r"ImmersiveReader-QA\{run_id}"),
        }
    }

    pub fn default_library(&self, documents: &Path) -> PathBuf {
        match self {
            Self::Production => documents.join(r"沉浸阅读\Library"),
            Self::Qa(run_id) => documents
                .join(r"Codex\ImmersiveReader-QA")
                .join(run_id)
                .join("Library"),
        }
    }
}
