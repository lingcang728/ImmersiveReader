use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

mod channel;
pub use channel::AppChannel;
mod recovery;
use recovery::load_status_from;
pub use recovery::SettingsLoadState;

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
    if let Ok(locations) = crate::storage::StorageLocations::current() {
        if let Some(parent) = locations.settings_path.parent() {
            return parent.to_path_buf();
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(AppChannel::current().settings_directory_name())
}

pub fn default_settings() -> AppSettings {
    if let Ok(locations) = crate::storage::StorageLocations::current() {
        return AppSettings {
            schema_version: 3,
            library_root: locations.library_root.to_string_lossy().into_owned(),
        };
    }
    let documents = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."));
    let local = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    AppSettings {
        schema_version: 3,
        library_root: AppChannel::current()
            .default_library(&local, &documents)
            .to_string_lossy()
            .into_owned(),
    }
}

fn validate(settings: &AppSettings) -> Result<(), String> {
    if settings.schema_version != 3 {
        return Err("Unsupported settings schema version".to_string());
    }
    if !Path::new(&settings.library_root).is_absolute() {
        return Err("Library root must be absolute".to_string());
    }
    Ok(())
}

pub(crate) fn load_compatible_from(path: &Path) -> Result<AppSettings, String> {
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
        1 | 2 => {
            let legacy: LegacySettings =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            AppSettings {
                schema_version: 3,
                library_root: legacy.library_root,
            }
        }
        3 => serde_json::from_value(value).map_err(|error| error.to_string())?,
        _ => return Err("Unsupported settings schema version".to_string()),
    };
    validate(&settings)?;
    Ok(settings)
}

pub(crate) fn save_compatible_to(path: &Path, settings: &AppSettings) -> Result<(), String> {
    validate(settings)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    crate::atomic_write_file(path, &data)
}

pub fn load_settings() -> Result<AppSettings, String> {
    let path = crate::storage::StorageLocations::current()?.settings_path;
    match load_status_from(&path) {
        SettingsLoadState::Active(settings) => Ok(settings),
        SettingsLoadState::Recovery(recovery) => Err(format!(
            "Settings recovery mode: {} ({})",
            recovery.error,
            recovery.source_path.display()
        )),
    }
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let locations = crate::storage::StorageLocations::current()?;
    crate::storage::validate_library_root(Path::new(&settings.library_root), &locations)?;
    save_compatible_to(&locations.settings_path, settings)
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

#[cfg(test)]
mod tests {
    use super::{
        default_settings, load_compatible_from, load_status_from, save_compatible_to, AppChannel,
        SettingsLoadState,
    };
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
        assert_eq!(settings.schema_version, 3);
        save_compatible_to(&path, &settings).expect("settings must save");
        let loaded = load_compatible_from(&path).expect("settings must load");
        assert_eq!(loaded.library_root, settings.library_root);
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn reads_schema_one_as_schema_three_without_rewriting_the_source() {
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

        let loaded = load_compatible_from(&path).expect("legacy settings must migrate");

        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.library_root, r"C:\Library");
        let saved = fs::read_to_string(&path).expect("migrated settings must persist");
        assert!(saved.contains("companionRoot"));
        assert!(saved.contains("temporaryRoots"));
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn reads_schema_two_as_schema_three_without_rewriting_custom_library() {
        let root =
            std::env::temp_dir().join(format!("immersive-settings-v2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        let original = r#"{"schemaVersion":2,"libraryRoot":"D:\\My Reading"}"#;
        fs::write(&path, original).expect("schema two settings must write");

        let loaded = load_compatible_from(&path).expect("schema two settings must load compatibly");

        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.library_root, r"D:\My Reading");
        assert_eq!(
            fs::read_to_string(&path).expect("source settings must remain readable"),
            original
        );
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn malformed_settings_enter_recovery_without_overwriting_source() {
        let root = std::env::temp_dir().join(format!(
            "immersive-settings-recovery-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        let original = br#"{"schemaVersion":2,"libraryRoot":"C:\\Broken"#;
        fs::write(&path, original).expect("malformed settings must write");

        let state = load_status_from(&path);

        match state {
            SettingsLoadState::Recovery(recovery) => {
                assert!(recovery.read_only);
                assert_eq!(recovery.source_path, path);
                assert!(!recovery.error.is_empty());
            }
            SettingsLoadState::Active(_) => panic!("malformed settings must not become active"),
        }
        assert_eq!(
            fs::read(&path).expect("source settings must remain readable"),
            original
        );
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }
}
