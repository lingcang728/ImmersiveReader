use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppChannel {
    Production,
    Development,
    Qa(String),
}

impl AppChannel {
    pub fn detect(executable: &Path, qa_run_id: Option<&str>) -> Result<Self, String> {
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

        let stem = executable
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if stem.eq_ignore_ascii_case("immersive-reader-dev") {
            Ok(Self::Development)
        } else {
            Ok(Self::Production)
        }
    }

    pub fn current() -> Self {
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return Self::Production,
        };
        match Self::detect(&executable, None) {
            Ok(channel) => channel,
            Err(_) => Self::Production,
        }
    }

    pub fn settings_directory_name(&self) -> String {
        match self {
            Self::Production => "immersive-reader".to_string(),
            Self::Development => "immersive-reader-dev".to_string(),
            Self::Qa(run_id) => format!("ImmersiveReader-QA-{run_id}"),
        }
    }

    pub fn local_data_directory_name(&self) -> String {
        match self {
            Self::Production => "ImmersiveReader".to_string(),
            Self::Development => "ImmersiveReader-Dev".to_string(),
            Self::Qa(run_id) => format!(r"ImmersiveReader-QA\{run_id}"),
        }
    }

    pub fn default_library(&self, local: &Path, documents: &Path) -> PathBuf {
        match self {
            Self::Production => documents.join(r"沉浸阅读\Library"),
            Self::Development => local.join(r"ImmersiveReader-Dev\Library"),
            Self::Qa(run_id) => documents
                .join(r"Codex\ImmersiveReader-QA")
                .join(run_id)
                .join("Library"),
        }
    }
}
