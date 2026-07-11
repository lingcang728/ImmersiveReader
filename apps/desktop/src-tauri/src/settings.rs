use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
        .join("immersive-reader")
}

pub fn default_settings() -> AppSettings {
    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    AppSettings {
        schema_version: 2,
        library_root: profile
            .join(r"Documents\沉浸阅读\Library")
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
        .unwrap_or_else(|| app_state_dir())
        .join("ImmersiveReader")
}

#[cfg(test)]
mod tests {
    use super::{default_settings, load_from, save_to};
    use std::fs;

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
