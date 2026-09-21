use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

mod channel;
pub use channel::AppChannel;
mod recovery;
use recovery::load_status_from;
pub use recovery::SettingsLoadState;

/// Same acceptance set as `contracts::deserialize_schema_version`, pinned to
/// the settings schema `const: 3` — the schema treats `3` and `3.0` as the
/// same number, so the reader must too.
fn deserialize_settings_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value.as_f64() {
        Some(3.0) => Ok(3),
        _ => Err(serde::de::Error::custom(
            "unsupported settings schema version",
        )),
    }
}

// P2-29: `additionalProperties: false` in settings.schema.json — unknown keys
// must fail here too instead of being silently dropped on load.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(deserialize_with = "deserialize_settings_schema_version")]
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
    // P3-23: fail closed — never root app state at the process CWD. Same
    // contract as `AppChannel::current`, which panics rather than silently
    // downgrading to the production channel.
    dirs::data_dir()
        .expect("Roaming AppData is unavailable")
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
    // Accept integral floats (`3.0`, `3e0`) — the schema `const` and TS treat
    // them as the same number; a bare as_u64 would misread them as "missing".
    let version = value
        .get("schemaVersion")
        .and_then(|raw| {
            raw.as_u64().or_else(|| {
                raw.as_f64()
                    .filter(|float| {
                        float.fract() == 0.0 && *float >= 0.0 && *float <= u64::MAX as f64
                    })
                    .map(|float| float as u64)
            })
        })
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

/// P3-23: settings.json writes have no transactional CAS, so track the
/// on-disk mtime this process last observed (read or wrote). `save_settings`
/// refuses to clobber a file an external writer touched since then.
static SETTINGS_FILE_STAMP: Mutex<Option<(PathBuf, Option<SystemTime>)>> = Mutex::new(None);

fn file_stamp(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn remember_file_stamp(path: &Path) {
    if let Ok(mut stamp) = SETTINGS_FILE_STAMP.lock() {
        *stamp = Some((path.to_path_buf(), file_stamp(path)));
    }
}

fn stamp_unchanged(
    recorded: &Option<(PathBuf, Option<SystemTime>)>,
    path: &Path,
    observed: Option<SystemTime>,
) -> bool {
    match recorded {
        Some((known_path, stamp)) => known_path == path && observed == *stamp,
        // Never observed by this process: creating a still-absent file is
        // safe, but an existing unread file is never silently overwritten.
        None => observed.is_none(),
    }
}

fn check_file_stamp(path: &Path) -> Result<(), String> {
    let observed = file_stamp(path);
    let recorded = SETTINGS_FILE_STAMP
        .lock()
        .map_err(|_| "Settings state is unavailable".to_string())?
        .clone();
    if stamp_unchanged(&recorded, path, observed) {
        return Ok(());
    }
    Err("settings.json changed on disk since it was last read; refusing to overwrite".to_string())
}

pub(crate) fn save_compatible_to(path: &Path, settings: &AppSettings) -> Result<(), String> {
    validate(settings)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    crate::atomic_write_file(path, &data)?;
    remember_file_stamp(path);
    Ok(())
}

pub fn load_settings() -> Result<AppSettings, String> {
    let path = crate::storage::StorageLocations::current()?.settings_path;
    remember_file_stamp(&path);
    match load_status_from(&path) {
        SettingsLoadState::Active(settings) => {
            // Load/save must be symmetric: an externally written
            // `libraryRoot` of `C:\` or one overlapping a managed root would
            // bypass the save-side `validate_library_root` entirely. Validate
            // on load too — a bad root enters Recovery instead of taking
            // effect.
            let locations = crate::storage::StorageLocations::current()?;
            crate::storage::validate_library_root(Path::new(&settings.library_root), &locations)?;
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
    // P-10-F18: the pin is intentional fail-closed design, but the rewrite
    // still goes through the mtime CAS — a concurrent external edit landing
    // after our load must not be silently clobbered.
    settings.library_root = locations.library_root.to_string_lossy().into_owned();
    let path = locations.settings_path.clone();
    if check_file_stamp(&path).is_ok() {
        let _ = save_compatible_to(&path, &settings);
    }
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
    check_file_stamp(&locations.settings_path)?;
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
        default_settings, load_compatible_from, load_status_from, save_compatible_to,
        stamp_unchanged, AppChannel, SettingsLoadState,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

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
        let error = AppChannel::detect(Some(r"..\production"))
            .expect_err("unsafe QA run id must be rejected");

        assert!(error.contains("QA run id"));
    }

    /// Serializes tests that mutate `IMMERSIVE_QA_RUN_ID`; env vars are
    /// process-global, so two tests writing different run ids concurrently
    /// otherwise observe each other's value mid-assertion.
    static QA_RUN_ID_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Restores `IMMERSIVE_QA_RUN_ID` even when the test panics so the env
    /// mutation cannot leak into parallel tests.
    struct QaRunIdEnvGuard {
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl QaRunIdEnvGuard {
        fn set(value: &str) -> Self {
            let lock = QA_RUN_ID_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var_os("IMMERSIVE_QA_RUN_ID");
            std::env::set_var("IMMERSIVE_QA_RUN_ID", value);
            Self {
                previous,
                _lock: lock,
            }
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
        let root =
            std::env::temp_dir().join(format!("immersive-settings-unknown-{}", std::process::id()));
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
    fn save_cas_rejects_files_not_matching_the_last_observation() {
        // P3-23: the stamp comparison is pure — exercise every divergence
        // between what this process last saw and what is on disk now.
        let path = PathBuf::from("settings.json");
        let t1 = SystemTime::UNIX_EPOCH;
        let t2 = t1 + Duration::from_secs(1);
        let observed = Some((path.clone(), Some(t1)));

        assert!(stamp_unchanged(&observed, &path, Some(t1)));
        assert!(!stamp_unchanged(&observed, &path, Some(t2))); // external write
        assert!(!stamp_unchanged(&observed, &path, None)); // deleted
        assert!(!stamp_unchanged(
            &Some((PathBuf::from("other.json"), Some(t1))),
            &path,
            Some(t1)
        )); // recorded for another path
        let absent = Some((path.clone(), None));
        assert!(stamp_unchanged(&absent, &path, None));
        assert!(!stamp_unchanged(&absent, &path, Some(t1))); // appeared since load
                                                             // Never observed in this process: create is fine, clobber is not.
        assert!(stamp_unchanged(&None, &path, None));
        assert!(!stamp_unchanged(&None, &path, Some(t1)));
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
