use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

mod channel;
pub use channel::AppChannel;
mod recovery;
use recovery::load_status_from;
pub use recovery::SettingsLoadState;

// P2-29: `additionalProperties: false` in settings.schema.json — unknown keys
// must fail here too instead of being silently dropped on load.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub library_root: String,
}

// Legacy v1/v2 files may carry fields the current app no longer reads
// (companionRoot, temporaryRoots). Keep this struct permissive — migration
// must not break on the extra keys real legacy installs contain.
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
    AppSettings {
        schema_version: 3,
        library_root: AppChannel::current()
            .default_library(&documents)
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
    // PowerShell / some editors write UTF-8 with BOM; serde_json rejects it as
    // "expected value at line 1 column 1".
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw.as_str());
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
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
        SettingsLoadState::Active(settings) => {
            // Always keep production library at Documents/沉浸阅读/Library.
            // Repair any legacy project-path override silently.
            Ok(normalize_production_library_root(settings)?)
        }
        SettingsLoadState::Recovery(recovery) => Err(format!(
            "Settings recovery mode: {} ({})",
            recovery.error,
            recovery.source_path.display()
        )),
    }
}

/// Production library is fixed under Documents; only QA keeps its isolated path.
fn normalize_production_library_root(mut settings: AppSettings) -> Result<AppSettings, String> {
    let locations = crate::storage::StorageLocations::current()?;
    if locations.channel != "production" {
        return Ok(settings);
    }
    let canonical = locations.library_root.to_string_lossy().replace('/', "\\");
    let current = settings.library_root.replace('/', "\\");
    if current.eq_ignore_ascii_case(&canonical) {
        return Ok(settings);
    }
    // Rewrite stale/custom roots (e.g. project-local Library) to the Documents default.
    settings.library_root = locations.library_root.to_string_lossy().into_owned();
    let path = locations.settings_path.clone();
    let _ = save_compatible_to(&path, &settings);
    Ok(settings)
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let locations = crate::storage::StorageLocations::current()?;
    let mut settings = settings.clone();
    // Production always pins the library under Documents/沉浸阅读/Library.
    if locations.channel == "production" {
        settings.library_root = locations.library_root.to_string_lossy().into_owned();
    }
    crate::storage::validate_library_root(Path::new(&settings.library_root), &locations)?;
    save_compatible_to(&locations.settings_path, &settings)
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
    fn detect_without_qa_run_id_is_production() {
        let channel = AppChannel::detect(None).expect("production channel must parse");

        assert_eq!(channel, AppChannel::Production);
        assert_eq!(channel.settings_directory_name(), "immersive-reader");
        assert_eq!(channel.local_data_directory_name(), "ImmersiveReader");
    }

    #[test]
    fn keeps_production_and_qa_default_libraries_separate() {
        let documents = Path::new(r"C:\Users\reader\Documents");

        let production = AppChannel::Production.default_library(documents);
        let qa = AppChannel::Qa("run-1".to_string()).default_library(documents);

        assert_eq!(production, documents.join(r"沉浸阅读\Library"));
        assert_eq!(
            qa,
            documents.join(r"Codex\ImmersiveReader-QA\run-1\Library")
        );
        assert_ne!(production, qa);
    }

    #[test]
    fn qa_channel_rejects_unsafe_run_ids() {
        let error =
            AppChannel::detect(Some(r"..\production")).expect_err("unsafe QA run id must be rejected");

        assert!(error.contains("QA run id"));
    }

    /// Restores `IMMERSIVE_QA_RUN_ID` even when the test panics so the env
    /// mutation cannot leak into parallel tests.
    struct QaRunIdEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl QaRunIdEnvGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var_os("IMMERSIVE_QA_RUN_ID");
            std::env::set_var("IMMERSIVE_QA_RUN_ID", value);
            Self { previous }
        }
    }

    impl Drop for QaRunIdEnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("IMMERSIVE_QA_RUN_ID", value),
                None => std::env::remove_var("IMMERSIVE_QA_RUN_ID"),
            }
        }
    }

    #[test]
    fn current_derives_the_qa_channel_from_the_run_id_environment() {
        let _guard = QaRunIdEnvGuard::set("test-run-19");

        let channel = AppChannel::current();

        assert_eq!(channel, AppChannel::Qa("test-run-19".to_string()));
        assert_eq!(
            channel.local_data_directory_name(),
            r"ImmersiveReader-QA\test-run-19"
        );
    }

    #[test]
    fn current_rejects_an_unsafe_run_id_instead_of_falling_back_to_production() {
        let _guard = QaRunIdEnvGuard::set(r"..\production");

        let result = std::panic::catch_unwind(AppChannel::current);

        assert!(
            result.is_err(),
            "unsafe QA run id must not reach Production"
        );
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
    fn loads_utf8_bom_prefixed_settings() {
        let root =
            std::env::temp_dir().join(format!("immersive-settings-bom-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(
            br#"{"schemaVersion":3,"libraryRoot":"C:\\Users\\reader\\Documents\\Library"}"#,
        );
        fs::write(&path, bytes).expect("bom settings must write");

        let loaded = load_compatible_from(&path).expect("bom settings must load");

        assert_eq!(loaded.schema_version, 3);
        assert_eq!(loaded.library_root, r"C:\Users\reader\Documents\Library");
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn v3_settings_reject_unknown_fields() {
        // P2-29: mirrors `additionalProperties: false` in settings.schema.json —
        // an unrecognized key must fail instead of being silently dropped.
        let root = std::env::temp_dir().join(format!(
            "immersive-settings-unknown-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        let path = root.join("settings.json");
        fs::write(
            &path,
            r#"{"schemaVersion":3,"libraryRoot":"C:\\Library","futureField":true}"#,
        )
        .expect("settings must write");

        assert!(load_compatible_from(&path).is_err());
        match load_status_from(&path) {
            SettingsLoadState::Recovery(_) => {}
            SettingsLoadState::Active(_) => panic!("unknown fields must not become active"),
        }
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    /// P2-29: the settings fixtures are the shared contract — the Python
    /// parity script (schema verdicts) and this test (Rust verdicts) consume
    /// the same settings-expectations.json table.
    #[derive(serde::Deserialize)]
    struct SettingsExpectation {
        fixture: String,
        expect: String,
    }

    #[test]
    fn shared_settings_fixtures_match_schema_verdicts() {
        let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/contracts/fixtures");
        let raw = std::fs::read_to_string(fixtures_dir.join("settings-expectations.json"))
            .expect("fixtures/settings-expectations.json must exist");
        let expectations: Vec<SettingsExpectation> =
            serde_json::from_str(&raw).expect("settings-expectations.json must parse");
        assert!(
            !expectations.is_empty(),
            "settings parity suite must cover fixtures"
        );
        let root = std::env::temp_dir().join(format!(
            "immersive-settings-fixtures-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp directory must be created");
        for entry in expectations {
            let text = std::fs::read_to_string(fixtures_dir.join(&entry.fixture))
                .unwrap_or_else(|error| panic!("{}: {error}", entry.fixture));
            let path = root.join("settings.json");
            fs::write(&path, &text).expect("fixture must write");
            let accepted = load_compatible_from(&path).is_ok();
            assert_eq!(
                accepted,
                entry.expect == "valid",
                "fixture {}",
                entry.fixture
            );
        }
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
