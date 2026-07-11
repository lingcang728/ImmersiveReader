use serde::{Deserialize, Serialize};
use std::fs;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub library_root: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySettings {
    library_root: String,
}

pub fn app_state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(AppChannel::current().settings_directory_name())
}

pub fn default_settings() -> AppSettings {
    let documents = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."));
    let local = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    AppSettings {
        schema_version: 2,
        library_root: AppChannel::current()
            .default_library(&local, &documents)
            .to_string_lossy()
            .into_owned(),
    }
}

fn validate(settings: &AppSettings) -> Result<(), String> {
    if settings.schema_version != 2 {
        return Err("Unsupported settings schema version".to_string());
    }
    if !Path::new(&settings.library_root).is_absolute() {
        return Err("Library root must be absolute".to_string());
    }
    Ok(())
}

fn load_from(path: &Path) -> Result<AppSettings, String> {
    if !path.exists() {
        return Ok(default_settings());
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Settings schema version is missing".to_string())?;
    let settings = match version {
        1 => {
            let legacy: LegacySettings =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            let migrated = AppSettings {
                schema_version: 2,
                library_root: legacy.library_root,
            };
            save_to(path, &migrated)?;
            migrated
        }
        2 => serde_json::from_value(value).map_err(|error| error.to_string())?,
        _ => return Err("Unsupported settings schema version".to_string()),
    };
    validate(&settings)?;
    Ok(settings)
}

fn save_to(path: &Path, settings: &AppSettings) -> Result<(), String> {
    validate(settings)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    crate::atomic_write_file(path, &data)
}

pub fn load_settings() -> Result<AppSettings, String> {
    load_from(&app_state_dir().join("settings.json"))
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    save_to(&app_state_dir().join("settings.json"), settings)
}

pub fn runtime_root() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("IMMERSIVE_RUNTIME_ROOT") {
        return Ok(PathBuf::from(configured));
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let parent = executable
        .parent()
        .ok_or_else(|| "Application directory is unavailable".to_string())?;
    Ok(parent.join("runtime"))
}

pub fn local_runtime_data() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(app_state_dir)
        .join(AppChannel::current().local_data_directory_name())
}

#[cfg(test)]
mod tests {
    use super::{default_settings, load_from, save_to, AppChannel};
    use std::fs;
    use std::path::Path;

    #[test]
    fn detects_development_channel_from_executable_name() {
        let channel = AppChannel::detect(Path::new(r"C:\app\immersive-reader-dev.exe"), None)
            .expect("development executable must parse");

        assert_eq!(channel, AppChannel::Development);
        assert_eq!(channel.settings_directory_name(), "immersive-reader-dev");
        assert_eq!(channel.local_data_directory_name(), "ImmersiveReader-Dev");
    }

    #[test]
    fn keeps_production_and_development_default_libraries_separate() {
        let local = Path::new(r"C:\Users\reader\AppData\Local");
        let documents = Path::new(r"C:\Users\reader\Documents");

        let production = AppChannel::Production.default_library(local, documents);
        let development = AppChannel::Development.default_library(local, documents);

        assert_eq!(production, documents.join(r"沉浸阅读\Library"));
        assert_eq!(development, local.join(r"ImmersiveReader-Dev\Library"));
        assert_ne!(production, development);
    }

    #[test]
    fn qa_channel_rejects_unsafe_run_ids() {
        let error = AppChannel::detect(
            Path::new(r"C:\app\immersive-reader-dev.exe"),
            Some(r"..\production"),
        )
        .expect_err("unsafe QA run id must be rejected");

        assert!(error.contains("QA run id"));
    }

    #[test]
    fn settings_round_trip_outside_the_library() {
        let root = std::env::temp_dir().join(format!("immersive-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        let settings = default_settings();
        save_to(&path, &settings).expect("settings must save");
        let loaded = load_from(&path).expect("settings must load");
        assert_eq!(loaded.library_root, settings.library_root);
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn migrates_schema_one_without_legacy_runtime_paths() {
        let root =
            std::env::temp_dir().join(format!("immersive-settings-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        fs::write(
            &path,
            r#"{"schemaVersion":1,"libraryRoot":"C:\\Library","companionRoot":"C:\\Old","temporaryRoots":[]}"#,
        )
        .expect("legacy settings must write");

        let loaded = load_from(&path).expect("legacy settings must migrate");

        assert_eq!(loaded.schema_version, 2);
        assert_eq!(loaded.library_root, r"C:\Library");
        let saved = fs::read_to_string(&path).expect("migrated settings must persist");
        assert!(!saved.contains("companionRoot"));
        assert!(!saved.contains("temporaryRoots"));
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }
}
