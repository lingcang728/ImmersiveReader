use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const READER_PREFERENCES_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReaderPreferences {
    pub schema_version: u32,
    pub font_scale: f64,
    pub line_height: f64,
    pub content_width: u32,
    pub font_family: String,
}

impl Default for ReaderPreferences {
    fn default() -> Self {
        Self {
            schema_version: READER_PREFERENCES_SCHEMA_VERSION,
            font_scale: 1.0,
            line_height: 1.8,
            content_width: 760,
            font_family: "sans".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderPreferencesLoad {
    pub preferences: ReaderPreferences,
    pub store_exists: bool,
}

fn validate(preferences: &ReaderPreferences) -> Result<(), String> {
    if preferences.schema_version != READER_PREFERENCES_SCHEMA_VERSION {
        return Err("Unsupported reader preferences schema version".to_string());
    }
    if !(0.8..=1.5).contains(&preferences.font_scale) {
        return Err("Reader font scale must be between 0.8 and 1.5".to_string());
    }
    if ![1.6, 1.8, 2.0].contains(&preferences.line_height) {
        return Err("Unsupported reader line height".to_string());
    }
    if ![680, 760, 840].contains(&preferences.content_width) {
        return Err("Unsupported reader content width".to_string());
    }
    if !matches!(preferences.font_family.as_str(), "sans" | "serif") {
        return Err("Unsupported reader font family".to_string());
    }
    Ok(())
}

fn preferences_path() -> PathBuf {
    crate::settings::app_state_dir().join("reader-preferences.json")
}

pub(crate) fn load_from(path: &Path) -> Result<ReaderPreferencesLoad, String> {
    if !path.exists() {
        return Ok(ReaderPreferencesLoad {
            preferences: ReaderPreferences::default(),
            store_exists: false,
        });
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw.as_str());
    let preferences: ReaderPreferences =
        serde_json::from_str(raw).map_err(|error| error.to_string())?;
    validate(&preferences)?;
    Ok(ReaderPreferencesLoad {
        preferences,
        store_exists: true,
    })
}

pub(crate) fn save_to(path: &Path, preferences: &ReaderPreferences) -> Result<(), String> {
    validate(preferences)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(preferences).map_err(|error| error.to_string())?;
    crate::atomic_write_file(path, &data)
}

pub fn load() -> Result<ReaderPreferencesLoad, String> {
    load_from(&preferences_path())
}

pub fn save(preferences: &ReaderPreferences) -> Result<(), String> {
    save_to(&preferences_path(), preferences)
}

#[cfg(test)]
mod tests {
    use super::{load_from, save_to, ReaderPreferences};
    use std::fs;

    #[test]
    fn preferences_survive_a_round_trip_outside_webview_storage() {
        let root = std::env::temp_dir().join(format!(
            "immersive-reader-preferences-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary preferences directory must exist");
        let path = root.join("reader-preferences.json");
        let preferences = ReaderPreferences {
            font_scale: 1.5,
            line_height: 2.0,
            content_width: 840,
            font_family: "serif".to_string(),
            ..ReaderPreferences::default()
        };

        save_to(&path, &preferences).expect("preferences must save");
        let loaded = load_from(&path).expect("preferences must load");

        assert!(loaded.store_exists);
        assert_eq!(loaded.preferences, preferences);
        fs::remove_dir_all(root).expect("temporary preferences directory must be removed");
    }

    #[test]
    fn missing_preferences_do_not_override_legacy_webview_values() {
        let root = std::env::temp_dir().join(format!(
            "immersive-reader-preferences-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let loaded =
            load_from(&root.join("reader-preferences.json")).expect("missing store must load");

        assert!(!loaded.store_exists);
        assert_eq!(loaded.preferences, ReaderPreferences::default());
    }

    #[test]
    fn rejects_out_of_range_font_scale() {
        let root = std::env::temp_dir().join(format!(
            "immersive-reader-preferences-invalid-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary preferences directory must exist");
        let path = root.join("reader-preferences.json");
        let preferences = ReaderPreferences {
            font_scale: 1.75,
            ..ReaderPreferences::default()
        };

        assert!(save_to(&path, &preferences).is_err());
        fs::remove_dir_all(root).expect("temporary preferences directory must be removed");
    }
}
