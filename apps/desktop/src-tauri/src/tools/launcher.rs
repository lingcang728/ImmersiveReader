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
        // P3-24: the podcast tool runs as a managed worker (podcast/worker.rs
        // spawns transcribe_task.py), never as an HTTP sidecar. These paths
        // only feed status()'s runtime-readiness probe — the runtime bundle
        // still ships sidecar_server.py per scripts/prepare-runtime.ps1.
        ToolKind::Podcast => ToolPaths {
            executable: runtime_root.join(r"podcast\python\python.exe"),
            script: runtime_root.join(r"podcast\app\scripts\sidecar_server.py"),
            working_directory: runtime_root.join(r"podcast\app"),
        },
    }
}

pub(super) fn require_runtime(runtime_root: &Path, paths: &ToolPaths) -> Result<(), String> {
    if paths.executable.is_file() && paths.script.is_file() && paths.working_directory.is_dir() {
        return Ok(());
    }
    Err(crate::storage::runtime_bundle_help(runtime_root))
}

pub(super) fn command_for(
    runtime_root: &Path,
    settings: &AppSettings,
    kind: ToolKind,
    token: &str,
) -> Result<Command, String> {
    let paths = tool_paths(runtime_root, kind);
    require_runtime(runtime_root, &paths)?;
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
        // P3-24: podcast is a managed worker, never a sidecar — no command
        // may be built for it. Refuse loudly instead of spawning
        // sidecar_server.py (the worker sets its own env in
        // podcast/worker.rs; nothing reads the old sidecar envs).
        ToolKind::Podcast => return Err("PODCAST_IS_WORKER_NOT_SIDECAR".to_string()),
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::{tool_paths, ToolKind};
    use std::path::Path;

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
}
