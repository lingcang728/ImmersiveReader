use crate::settings::AppSettings;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

static LAUNCHED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLaunch {
    pub tool: String,
    pub message: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub tool: String,
    pub state: String,
    pub version: String,
    pub message: String,
}

#[derive(Clone, Copy)]
enum ToolKind {
    Zhihu,
    Podcast,
}

impl ToolKind {
    fn parse(tool: &str) -> Result<Self, String> {
        match tool {
            "zhihu" => Ok(Self::Zhihu),
            "podcast" => Ok(Self::Podcast),
            _ => Err("Only the configured Zhihu and Podcast tools may be launched".to_string()),
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Zhihu => "zhihu",
            Self::Podcast => "podcast",
        }
    }

    const fn url(self) -> Option<&'static str> {
        match self {
            Self::Zhihu => Some("http://127.0.0.1:3000"),
            Self::Podcast => None,
        }
    }
}

struct ToolPaths {
    executable: PathBuf,
    script: PathBuf,
    working_directory: PathBuf,
}

fn tool_paths(runtime_root: &Path, kind: ToolKind) -> ToolPaths {
    match kind {
        ToolKind::Zhihu => ToolPaths {
            executable: runtime_root.join(r"zhihu\node\node.exe"),
            script: runtime_root.join(r"zhihu\app\dist\server.js"),
            working_directory: runtime_root.join(r"zhihu\app"),
        },
        ToolKind::Podcast => ToolPaths {
            executable: runtime_root.join(r"podcast\python\python.exe"),
            script: runtime_root.join(r"podcast\app\scripts\run_with_gui.py"),
            working_directory: runtime_root.join(r"podcast\app"),
        },
    }
}

fn action_for(tool: &str) -> Result<ToolKind, String> {
    ToolKind::parse(tool)
}

fn require_runtime(paths: &ToolPaths) -> Result<(), String> {
    if paths.executable.is_file() && paths.script.is_file() && paths.working_directory.is_dir() {
        return Ok(());
    }
    Err("受管工具运行时不完整，请重新运行 scripts\\prepare-runtime.ps1。".to_string())
}

fn prepare_podcast_data(paths: &ToolPaths) -> Result<PathBuf, String> {
    let data_root = crate::settings::local_runtime_data().join("podcast");
    for name in ["input", "output", "work"] {
        fs::create_dir_all(data_root.join(name)).map_err(|error| error.to_string())?;
    }
    let config = data_root.join("config.json");
    if !config.exists() {
        fs::copy(paths.working_directory.join("config.example.json"), &config)
            .map_err(|error| error.to_string())?;
    }
    Ok(data_root)
}

pub fn status(tool: &str) -> Result<ToolStatus, String> {
    let kind = action_for(tool)?;
    let paths = tool_paths(&crate::settings::runtime_root()?, kind);
    let ready = require_runtime(&paths).is_ok();
    let running = LAUNCHED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| "Tool launch state is unavailable".to_string())?
        .contains(tool);
    Ok(ToolStatus {
        tool: tool.to_string(),
        state: if running {
            "running"
        } else if ready {
            "ready"
        } else {
            "error"
        }
        .to_string(),
        version: "1.0.0".to_string(),
        message: if ready {
            "受管运行时已就绪。"
        } else {
            "受管运行时缺失，请重新准备运行时。"
        }
        .to_string(),
    })
}

pub fn launch(tool: &str, settings: &AppSettings) -> Result<ToolLaunch, String> {
    let kind = action_for(tool)?;
    let runtime_root = crate::settings::runtime_root()?;
    let paths = tool_paths(&runtime_root, kind);
    require_runtime(&paths)?;
    let key = kind.key();
    let url = kind.url();
    let launches = LAUNCHED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = launches
        .lock()
        .map_err(|_| "Tool launch state is unavailable".to_string())?;
    if guard.contains(key) {
        return Ok(ToolLaunch {
            tool: key.to_string(),
            message: "工具已在本次应用会话中启动。".to_string(),
            url: url.map(str::to_string),
        });
    }

    let mut command = Command::new(&paths.executable);
    command
        .arg(&paths.script)
        .current_dir(&paths.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match kind {
        ToolKind::Zhihu => {
            let data_root = crate::settings::local_runtime_data().join("zhihu");
            fs::create_dir_all(&data_root).map_err(|error| error.to_string())?;
            command
                .env("IMMERSIVE_LIBRARY_ROOT", &settings.library_root)
                .env(
                    "IMMERSIVE_ZHIHU_OUTPUT",
                    Path::new(&settings.library_root).join("知乎"),
                )
                .env("IMMERSIVE_ZHIHU_DB", data_root.join("zhihu-packer.db"))
                .env("IMMERSIVE_ZHIHU_PROFILE", data_root.join("browser-profile"))
                .env(
                    "IMMERSIVE_CHROMIUM_EXECUTABLE",
                    runtime_root.join(r"zhihu\chromium\msedge.exe"),
                );
        }
        ToolKind::Podcast => {
            let data_root = prepare_podcast_data(&paths)?;
            let mut path_parts = vec![runtime_root.join(r"podcast\ffmpeg")];
            if let Some(existing) = std::env::var_os("PATH") {
                path_parts.extend(std::env::split_paths(&existing));
            }
            command
                .env("IMMERSIVE_PODCAST_DATA_ROOT", data_root)
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
    command.spawn().map_err(|error| error.to_string())?;
    guard.insert(key.to_string());
    Ok(ToolLaunch {
        tool: key.to_string(),
        message: match kind {
            ToolKind::Zhihu => "知乎归档控制台正在启动。",
            ToolKind::Podcast => "播客转写窗口正在启动。",
        }
        .to_string(),
        url: url.map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::{action_for, tool_paths, ToolKind};
    use std::path::Path;

    #[test]
    fn rejects_arbitrary_commands() {
        assert!(action_for("cmd /c calc").is_err());
        assert!(action_for("zhihu").is_ok());
        assert!(action_for("podcast").is_ok());
    }

    #[test]
    fn resolves_tools_inside_the_managed_runtime() {
        let root = Path::new(r"C:\ImmersiveReader\runtime");

        let zhihu = tool_paths(root, ToolKind::Zhihu);
        let podcast = tool_paths(root, ToolKind::Podcast);

        assert_eq!(zhihu.executable, root.join(r"zhihu\node\node.exe"));
        assert_eq!(
            podcast.script,
            root.join(r"podcast\app\scripts\run_with_gui.py")
        );
    }
}
