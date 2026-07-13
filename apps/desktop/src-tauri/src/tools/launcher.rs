use super::ToolKind;
use crate::settings::AppSettings;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
fn sidecar_stdout() -> Stdio {
    Stdio::piped()
}

#[cfg(not(windows))]
fn sidecar_stdout() -> Stdio {
    Stdio::null()
}

pub(super) struct ToolPaths {
    executable: PathBuf,
    script: PathBuf,
    working_directory: PathBuf,
}

pub(super) fn tool_paths(runtime_root: &Path, kind: ToolKind) -> ToolPaths {
    match kind {
        ToolKind::Zhihu => ToolPaths {
            executable: runtime_root.join(r"zhihu\node\node.exe"),
            script: runtime_root.join(r"zhihu\app\dist\server.js"),
            working_directory: runtime_root.join(r"zhihu\app"),
        },
        ToolKind::Podcast => ToolPaths {
            executable: runtime_root.join(r"podcast\python\python.exe"),
            script: runtime_root.join(r"podcast\app\scripts\sidecar_server.py"),
            working_directory: runtime_root.join(r"podcast\app"),
        },
    }
}

pub(super) fn require_runtime(paths: &ToolPaths) -> Result<(), String> {
    if paths.executable.is_file() && paths.script.is_file() && paths.working_directory.is_dir() {
        return Ok(());
    }
    Err("受管工具运行时不完整，请重新运行 scripts\\prepare-runtime.ps1。".to_string())
}

fn prepare_podcast_data(
    paths: &ToolPaths,
    locations: &crate::storage::StorageLocations,
) -> Result<(PathBuf, PathBuf), String> {
    let data_root = locations.data_root.join("Podcast");
    let cache_root = locations.cache_root.join("Podcast");
    fs::create_dir_all(&data_root).map_err(|error| error.to_string())?;
    for name in ["input", "output", "work"] {
        fs::create_dir_all(cache_root.join(name)).map_err(|error| error.to_string())?;
    }
    let config = data_root.join("config.json");
    if !config.exists() {
        fs::copy(paths.working_directory.join("config.example.json"), &config)
            .map_err(|error| error.to_string())?;
    }
    Ok((data_root, cache_root))
}

pub(super) fn command_for(
    runtime_root: &Path,
    settings: &AppSettings,
    kind: ToolKind,
    token: &str,
) -> Result<Command, String> {
    let paths = tool_paths(runtime_root, kind);
    require_runtime(&paths)?;
    let mut command = Command::new(&paths.executable);
    command
        .arg(&paths.script)
        .current_dir(&paths.working_directory)
        .stdin(Stdio::null())
        .stdout(sidecar_stdout())
        .stderr(Stdio::null());
    command
        .env("IMMERSIVE_SIDECAR_PORT", "0")
        .env("IMMERSIVE_SIDECAR_TOKEN", token);
    match kind {
        ToolKind::Zhihu => {
            let locations = crate::storage::StorageLocations::current()?;
            let data_root = locations.data_root.join("Zhihu");
            let profile_root = locations.data_root.join("Private").join("ZhihuProfile");
            let browser_cache = locations.cache_root.join("Zhihu").join("BrowserCache");
            fs::create_dir_all(&data_root).map_err(|error| error.to_string())?;
            fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
            fs::create_dir_all(&browser_cache).map_err(|error| error.to_string())?;
            command
                .env("IMMERSIVE_LIBRARY_ROOT", &settings.library_root)
                .env(
                    "IMMERSIVE_ZHIHU_OUTPUT",
                    Path::new(&settings.library_root).join("知乎"),
                )
                .env("IMMERSIVE_ZHIHU_DB", data_root.join("zhihu-packer.db"))
                .env("IMMERSIVE_ZHIHU_PROFILE", profile_root)
                .env("IMMERSIVE_ZHIHU_BROWSER_CACHE", browser_cache)
                .env("ZHIHU_PACKER_TOKEN", token)
                .env(
                    "IMMERSIVE_CHROMIUM_EXECUTABLE",
                    runtime_root.join(r"zhihu\chromium\msedge.exe"),
                );
        }
        ToolKind::Podcast => {
            let locations = crate::storage::StorageLocations::current()?;
            let (data_root, cache_root) = prepare_podcast_data(&paths, &locations)?;
            let mut path_parts = vec![runtime_root.join(r"podcast\ffmpeg")];
            if let Some(existing) = std::env::var_os("PATH") {
                path_parts.extend(std::env::split_paths(&existing));
            }
            command
                .env("IMMERSIVE_PODCAST_DATA_ROOT", data_root)
                .env("IMMERSIVE_PODCAST_CACHE_ROOT", cache_root)
                .env(
                    "IMMERSIVE_PODCAST_MODEL_ROOT",
                    runtime_root.join(r"podcast\models"),
                )
                .env("IMMERSIVE_PODCAST_PYTHON", &paths.executable)
                .env(
                    "PATH",
                    std::env::join_paths(path_parts).map_err(|error| error.to_string())?,
                );
        }
    }
    if let Some(api_key) =
        crate::secrets::deepseek_api_key(&crate::settings::AppChannel::current())?
    {
        command.env("DEEPSEEK_API_KEY", api_key);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::{prepare_podcast_data, tool_paths, ToolKind};
    use crate::storage::StorageLocations;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn resolves_tools_inside_the_managed_runtime() {
        let root = Path::new(r"C:\ImmersiveReader\runtime");

        let zhihu = tool_paths(root, ToolKind::Zhihu);
        let podcast = tool_paths(root, ToolKind::Podcast);

        assert_eq!(zhihu.executable, root.join(r"zhihu\node\node.exe"));
        assert_eq!(
            podcast.script,
            root.join(r"podcast\app\scripts\sidecar_server.py")
        );
    }

    #[test]
    fn podcast_sidecar_keeps_config_in_data_and_work_in_cache() {
        let root = std::env::temp_dir().join(format!(
            "immersive-podcast-sidecar-roots-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let runtime = root.join("runtime");
        let paths = tool_paths(&runtime, ToolKind::Podcast);
        fs::create_dir_all(&paths.working_directory).expect("runtime app must exist");
        fs::write(paths.working_directory.join("config.example.json"), b"{}")
            .expect("example config must exist");
        let locations = StorageLocations {
            channel: "development".to_string(),
            settings_path: root.join(r"Settings\settings.json"),
            data_root: root.join("Data"),
            cache_root: root.join("Cache"),
            logs_root: root.join("Logs"),
            runtime_state_root: root.join("RuntimeState"),
            backups_root: root.join("Backups"),
            library_root: root.join("Library"),
            runtime_root: PathBuf::from(&runtime),
        };

        let (data_root, cache_root) =
            prepare_podcast_data(&paths, &locations).expect("managed roots must prepare");

        assert_eq!(data_root, locations.data_root.join("Podcast"));
        assert_eq!(cache_root, locations.cache_root.join("Podcast"));
        assert!(data_root.join("config.json").is_file());
        for name in ["input", "output", "work"] {
            assert!(cache_root.join(name).is_dir());
            assert!(!data_root.join(name).exists());
        }
        fs::remove_dir_all(root).expect("fixture must clean up");
    }
}
