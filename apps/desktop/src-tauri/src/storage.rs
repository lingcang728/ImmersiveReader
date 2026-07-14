use crate::settings::AppChannel;
use serde::Serialize;
use std::path::{Path, PathBuf};

mod path_guard;
pub use path_guard::validate_library_root;

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
                    roaming.join(r"immersive-reader\settings.json"),
                    local.join("ImmersiveReader"),
                    documents_root.join("Library"),
                    documents_root.join("Backups"),
                )
            }
            AppChannel::Qa(run_id) => {
                let app_root = local.join("ImmersiveReader-QA").join(run_id);
                (
                    "qa".to_string(),
                    app_root.join(r"Settings\settings.json"),
                    app_root.clone(),
                    documents
                        .join(r"Codex\ImmersiveReader-QA")
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
        let roaming =
            dirs::data_dir().ok_or_else(|| "Roaming AppData is unavailable".to_string())?;
        let local =
            dirs::data_local_dir().ok_or_else(|| "Local AppData is unavailable".to_string())?;
        let documents = dirs::document_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
            .ok_or_else(|| "Documents directory is unavailable".to_string())?;
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let qa_run_id = std::env::var("IMMERSIVE_QA_RUN_ID").ok();
        let channel = AppChannel::detect(qa_run_id.as_deref())?;
        let runtime_root = if let Some(configured) = std::env::var_os("IMMERSIVE_RUNTIME_ROOT") {
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

    /// Default storage roots plus the user-configured Library path from settings.
    /// Podcast publish / open must use this — never the bare default Documents path alone.
    pub fn current_with_library_settings() -> Result<Self, String> {
        let mut locations = Self::current()?;
        let settings = crate::settings::load_settings()?;
        locations.library_root = PathBuf::from(settings.library_root);
        Ok(locations)
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_library_root, StorageLocations};
    use crate::settings::AppChannel;
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
