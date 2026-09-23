use crate::settings::AppChannel;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

mod path_guard;
pub(crate) use path_guard::path_within;
pub use path_guard::validate_library_root;

/// Android has no `dirs` backend — `dirs::data_dir`/`document_dir` all return
/// `None` there (`dirs-sys` `home_dir()` has no fallback without `$HOME`), so
/// every storage root derives from the app-private directories exposed by
/// Tauri's built-in mobile path plugin instead. The plugin needs a live
/// `AppHandle`, so the roots are captured once during `Builder::setup` into
/// this `OnceLock`; `StorageLocations::current()` then stays synchronous.
#[cfg(target_os = "android")]
mod android_roots {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    #[derive(Clone, Debug)]
    pub(crate) struct AndroidBaseDirs {
        /// `app_data_dir()` → `Context.getDataDir()` — the app-private data
        /// root. Feeds `resolve_for`'s `roaming` slot (settings.json parent).
        pub(crate) app_data: PathBuf,
        /// `app_local_data_dir()` → also `getDataDir()` on Android. Feeds the
        /// `local` slot (`Data`/`Cache`/`Logs`/`RuntimeState` live under it).
        pub(crate) app_local_data: PathBuf,
        /// `document_dir()` → `getExternalFilesDir(DIRECTORY_DOCUMENTS)`,
        /// which returns `null` when shared storage is unavailable — falls
        /// back to `app_data` so the Library always lands somewhere writable.
        pub(crate) documents: PathBuf,
        /// Lazy placeholder under the private cache dir. The managed
        /// Zhihu/Podcast runtimes never ship to Android, so tool readiness
        /// probes report "error" here instead of crashing on a bogus path.
        pub(crate) runtime: PathBuf,
    }

    static ANDROID_BASE: OnceLock<AndroidBaseDirs> = OnceLock::new();

    /// Must be the first call inside `Builder::setup` — every subsequent
    /// `StorageLocations::current()` depends on it. Fails loudly: without
    /// these roots the app cannot read settings, state, or its database.
    pub(crate) fn init_android_roots(app: &tauri::AppHandle) -> Result<(), String> {
        use tauri::Manager;

        let resolver = app.path();
        let app_data = resolver
            .app_data_dir()
            .map_err(|error| format!("Android app data directory is unavailable: {error}"))?;
        let app_local_data = resolver
            .app_local_data_dir()
            .unwrap_or_else(|_| app_data.clone());
        let app_cache = resolver
            .app_cache_dir()
            .map_err(|error| format!("Android app cache directory is unavailable: {error}"))?;
        let documents = resolver
            .document_dir()
            .unwrap_or_else(|_| app_data.join("Documents"));
        let runtime = app_cache.join("runtime");
        ANDROID_BASE
            .set(AndroidBaseDirs {
                app_data,
                app_local_data,
                documents,
                runtime,
            })
            .map_err(|_| "Android storage roots were already initialized".to_string())
    }

    /// Errors before `init_android_roots` completes — callers must surface the
    /// failure, never fall back to `dirs` or the process CWD.
    pub(crate) fn android_base_dirs() -> Result<&'static AndroidBaseDirs, String> {
        ANDROID_BASE
            .get()
            .ok_or_else(|| "Android storage roots were not initialized".to_string())
    }
}

#[cfg(target_os = "android")]
pub(crate) use android_roots::{android_base_dirs, init_android_roots};

/// User-facing acquisition hint for the managed runtime bundle. The NSIS
/// installer ships only the application; the multi-gigabyte runtime is a
/// separate GitHub Release asset (`runtime-bundle.zip.*` volumes) that must
/// be unpacked so `runtime\` sits next to the executable. See
/// `docs/runtime-acquisition.md`.
pub fn runtime_bundle_help(runtime_root: &Path) -> String {
    format!(
        "受管工具运行时缺失或不完整（期望位置：{}）。请从沉浸阅读 GitHub Release 下载同版本的 runtime-bundle.zip 全部分卷，合并解压到该目录后重启应用；获取与校验步骤见仓库 docs/runtime-acquisition.md。",
        runtime_root.display()
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLocations {
    pub channel: String,
    pub settings_path: PathBuf,
    pub data_root: PathBuf,
    pub cache_root: PathBuf,
    pub logs_root: PathBuf,
    pub runtime_state_root: PathBuf,
    pub backups_root: PathBuf,
    pub library_root: PathBuf,
    pub runtime_root: PathBuf,
}

impl StorageLocations {
    pub fn resolve_for(
        channel: &AppChannel,
        roaming: &Path,
        local: &Path,
        documents: &Path,
        runtime_root: &Path,
    ) -> Self {
        let (channel_name, settings_path, app_root, library_root, backups_root) = match channel {
            AppChannel::Production => {
                let documents_root = documents.join("沉浸阅读");
                (
                    "production".to_string(),
                    roaming.join("immersive-reader").join("settings.json"),
                    local.join("ImmersiveReader"),
                    documents_root.join("Library"),
                    documents_root.join("Backups"),
                )
            }
            AppChannel::Qa(run_id) => {
                let app_root = local.join("ImmersiveReader-QA").join(run_id);
                (
                    "qa".to_string(),
                    app_root.join("Settings").join("settings.json"),
                    app_root.clone(),
                    documents
                        .join("Codex")
                        .join("ImmersiveReader-QA")
                        .join(run_id)
                        .join("Library"),
                    app_root.join("Backups"),
                )
            }
        };

        Self {
            channel: channel_name,
            settings_path,
            data_root: app_root.join("Data"),
            cache_root: app_root.join("Cache"),
            logs_root: app_root.join("Logs"),
            runtime_state_root: app_root.join("RuntimeState"),
            backups_root,
            library_root,
            runtime_root: runtime_root.to_path_buf(),
        }
    }

    pub fn current() -> Result<Self, String> {
        let qa_run_id = std::env::var("IMMERSIVE_QA_RUN_ID").ok();
        let channel = AppChannel::detect(qa_run_id.as_deref())?;

        #[cfg(target_os = "android")]
        {
            // `dirs` returns no usable directories on Android; the roots were
            // captured from the mobile path plugin during setup. Before that
            // (e.g. the pre-setup `app_log` call) this errors — callers such as
            // `app_log` already fail silently, so no panic path is introduced.
            let base = android_base_dirs()?;
            let runtime_root = std::env::var_os("IMMERSIVE_RUNTIME_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.runtime.clone());
            Ok(Self::resolve_for(
                &channel,
                &base.app_data,
                &base.app_local_data,
                &base.documents,
                &runtime_root,
            ))
        }

        #[cfg(not(target_os = "android"))]
        {
            let roaming =
                dirs::data_dir().ok_or_else(|| "Roaming AppData is unavailable".to_string())?;
            let local =
                dirs::data_local_dir().ok_or_else(|| "Local AppData is unavailable".to_string())?;
            let documents = dirs::document_dir()
                .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
                .ok_or_else(|| "Documents directory is unavailable".to_string())?;
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let runtime_root = if let Some(configured) = std::env::var_os("IMMERSIVE_RUNTIME_ROOT")
            {
                PathBuf::from(configured)
            } else {
                executable
                    .parent()
                    .ok_or_else(|| "Application directory is unavailable".to_string())?
                    .join("runtime")
            };

            Ok(Self::resolve_for(
                &channel,
                &roaming,
                &local,
                &documents,
                &runtime_root,
            ))
        }
    }

    /// Default storage roots plus the user-configured Library path from settings.
    /// Podcast publish / open must use this — never the bare default Documents path alone.
    pub fn current_with_library_settings() -> Result<Self, String> {
        let mut locations = Self::current()?;
        let settings = crate::settings::load_settings()?;
        locations.library_root = PathBuf::from(settings.library_root);
        Ok(locations)
    }
}

/// P3-23: `Logs\` is created on startup but nothing ever wrote to it, so
/// release builds have no diagnostics once `eprintln!` goes nowhere. This is
/// the smallest fail-silent sink: append one `[timestamp] [component] msg`
/// line to `Logs\app.log`, rotating to a single `app.log.1` generation past
/// the cap — the same pattern as the zhihu-packer logger. Any failure is
/// swallowed: logging must never break the operation it describes.
const APP_LOG_LIMIT: u64 = 512 * 1024;

pub(crate) fn app_log(component: &str, message: &str) {
    let Ok(locations) = StorageLocations::current() else {
        return;
    };
    app_log_at(&locations.logs_root, component, message);
}

fn app_log_at(logs_root: &Path, component: &str, message: &str) {
    use std::io::Write;

    let run = || -> Result<(), std::io::Error> {
        fs::create_dir_all(logs_root)?;
        let path = logs_root.join("app.log");
        if fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) > APP_LOG_LIMIT {
            let rotated = logs_root.join("app.log.1");
            let _ = fs::remove_file(&rotated);
            fs::rename(&path, &rotated)?;
        }
        // Keep one entry per line — callers may pass text with embedded
        // newlines (error chains, stderr captures).
        let flattened = message.replace(['\r', '\n'], " ");
        let line = format!(
            "[{}] [{component}] {flattened}\n",
            chrono::Utc::now().to_rfc3339()
        );
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(line.as_bytes()))
    };
    let _ = run();
}

#[cfg(test)]
mod tests {
    use super::{validate_library_root, StorageLocations};
    use crate::settings::AppChannel;
    use std::fs;
    use std::path::Path;

    #[test]
    fn production_roots_keep_persistent_data_out_of_cache() {
        let locations = StorageLocations::resolve_for(
            &AppChannel::Production,
            Path::new(r"C:\Users\reader\AppData\Roaming"),
            Path::new(r"C:\Users\reader\AppData\Local"),
            Path::new(r"C:\Users\reader\Documents"),
            Path::new(r"C:\Program Files\ImmersiveReader\runtime"),
        );

        assert_eq!(
            locations.data_root,
            Path::new(r"C:\Users\reader\AppData\Local\ImmersiveReader\Data")
        );
        assert_eq!(
            locations.cache_root,
            Path::new(r"C:\Users\reader\AppData\Local\ImmersiveReader\Cache")
        );
        assert_eq!(
            locations.library_root,
            Path::new(r"C:\Users\reader\Documents\沉浸阅读\Library")
        );
        assert_ne!(locations.data_root, locations.cache_root);
        assert!(!locations.data_root.starts_with(&locations.cache_root));
        assert!(!locations.cache_root.starts_with(&locations.data_root));
    }

    #[test]
    fn qa_roots_are_scoped_by_safe_run_id() {
        let locations = StorageLocations::resolve_for(
            &AppChannel::Qa("run-20260711".to_string()),
            Path::new(r"C:\Users\reader\AppData\Roaming"),
            Path::new(r"C:\Users\reader\AppData\Local"),
            Path::new(r"C:\Users\reader\Documents"),
            Path::new(r"C:\repo\runtime"),
        );

        assert_eq!(
            locations.settings_path,
            Path::new(
                r"C:\Users\reader\AppData\Local\ImmersiveReader-QA\run-20260711\Settings\settings.json"
            )
        );
        assert_eq!(
            locations.library_root,
            Path::new(r"C:\Users\reader\Documents\Codex\ImmersiveReader-QA\run-20260711\Library")
        );
    }

    #[test]
    fn library_path_rejects_managed_roots_and_their_parents() {
        let locations = StorageLocations::resolve_for(
            &AppChannel::Production,
            Path::new(r"C:\Users\reader\AppData\Roaming"),
            Path::new(r"C:\Users\reader\AppData\Local"),
            Path::new(r"C:\Users\reader\Documents"),
            Path::new(r"C:\repo\runtime"),
        );

        for unsafe_path in [
            locations.data_root.as_path(),
            locations.cache_root.as_path(),
            Path::new(r"C:\Users\reader\AppData\Local\ImmersiveReader"),
            Path::new(r"C:\"),
        ] {
            assert!(
                validate_library_root(unsafe_path, &locations).is_err(),
                "unsafe Library path was accepted: {}",
                unsafe_path.display()
            );
        }
    }

    #[test]
    fn app_log_appends_and_rotates_one_generation() {
        let root = std::env::temp_dir().join(format!("immersive-app-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        super::app_log_at(&root, "test", "first line");
        let log = root.join("app.log");
        let text = fs::read_to_string(&log).expect("log line must be written");
        assert!(text.contains("[test] first line"));

        // Past the cap the file rotates to a single app.log.1 generation.
        fs::write(&log, vec![b'x'; (super::APP_LOG_LIMIT + 1) as usize])
            .expect("oversized log must write");
        super::app_log_at(&root, "test", "after rotation");
        assert!(root.join("app.log.1").exists());
        let text = fs::read_to_string(&log).expect("fresh log must be written");
        assert!(text.contains("after rotation"));
        assert!(text.len() < super::APP_LOG_LIMIT as usize);

        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn library_path_accepts_a_separate_absolute_directory() {
        let locations = StorageLocations::resolve_for(
            &AppChannel::Production,
            Path::new(r"C:\Users\reader\AppData\Roaming"),
            Path::new(r"C:\Users\reader\AppData\Local"),
            Path::new(r"C:\Users\reader\Documents"),
            Path::new(r"C:\Program Files\ImmersiveReader\runtime"),
        );

        assert!(validate_library_root(Path::new(r"D:\Reading\Library"), &locations).is_ok());
    }
}
