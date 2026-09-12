use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

trait AtomicReplacer {
    fn replace(&self, source: &Path, target: &Path) -> io::Result<()>;
}

struct PlatformReplacer;

#[cfg(all(target_os = "windows", not(miri)))]
extern "system" {
    fn MoveFileExW(
        lp_existing_file_name: *const u16,
        lp_new_file_name: *const u16,
        flags: u32,
    ) -> i32;
}

/// P2-20: absolute, `\\?\`-prefixed ("extended-length") spelling of `path`.
/// The prefix opts the call out of MAX_PATH (260) at the Win32 API level,
/// which works even when the app manifest lacks `longPathAware`. UNC paths
/// map to `\\?\UNC\server\share`; already-verbatim paths pass through.
#[cfg(windows)]
fn verbatim_string(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let text = absolute.to_string_lossy().replace('/', "\\");
    if text.starts_with(r"\\?\") {
        return text;
    }
    if let Some(rest) = text.strip_prefix(r"\\") {
        return format!(r"\\?\UNC\{rest}");
    }
    format!(r"\\?\{text}")
}

/// P2-20: normalize a path so `std::fs` calls keep working past MAX_PATH.
/// Paths comfortably below the limit keep their original spelling (relative
/// stays relative and `..` keeps its API-level normalization semantics —
/// a verbatim prefix would make them literal instead).
pub(crate) fn long_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    fn normalize(path: &Path) -> PathBuf {
        // 240 leaves headroom under 260 for callers that append a file name
        // or temp suffix to the returned path.
        if path.as_os_str().len() < 240 {
            return path.to_path_buf();
        }
        PathBuf::from(verbatim_string(path))
    }
    #[cfg(not(windows))]
    fn normalize(path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    normalize(path)
}

#[cfg(all(target_os = "windows", not(miri)))]
fn wide_path(path: &Path) -> Vec<u16> {
    // P2-20: MoveFileExW honours the extended-length prefix at every path
    // length, so emit it unconditionally — renames keep working past
    // MAX_PATH even without longPathAware in the app manifest.
    verbatim_string(path)
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

#[cfg(all(target_os = "windows", not(miri)))]
impl AtomicReplacer for PlatformReplacer {
    fn replace(&self, source: &Path, target: &Path) -> io::Result<()> {
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        let source_wide = wide_path(source);
        let target_wide = wide_path(target);
        // SAFETY: Category 8 (FFI boundary). Both buffers are owned, NUL-terminated
        // UTF-16 paths and remain alive for the entire synchronous Win32 call.
        let replaced = unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(any(not(target_os = "windows"), miri))]
impl AtomicReplacer for PlatformReplacer {
    fn replace(&self, source: &Path, target: &Path) -> io::Result<()> {
        fs::rename(source, target)
    }
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn create(target: &Path) -> io::Result<(Self, File)> {
        let extension = target
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}.tmp.{}", uuid::Uuid::new_v4()))
            .unwrap_or_else(|| format!("tmp.{}", uuid::Uuid::new_v4()));
        let path = target.with_extension(extension);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((Self { path, armed: true }, file))
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn write_with(path: &Path, data: &[u8], replacer: &impl AtomicReplacer) -> Result<(), String> {
    // P2-20: normalize once — the temp sibling, parent mkdir and final
    // replace all inherit the `\\?\` form when the target is near MAX_PATH.
    let normalized = long_path(path);
    let path = normalized.as_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Atomic write target has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let (mut guard, mut file) = TempFileGuard::create(path).map_err(|error| error.to_string())?;
    file.write_all(data).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    replacer
        .replace(&guard.path, path)
        .map_err(|error| error.to_string())?;
    guard.disarm();
    Ok(())
}

pub fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    write_with(path, data, &PlatformReplacer)
}

#[cfg(test)]
mod tests {
    use super::{write, write_with, AtomicReplacer};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    struct FailingReplacer;

    impl AtomicReplacer for FailingReplacer {
        fn replace(&self, _source: &Path, _target: &Path) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected replacement failure",
            ))
        }
    }

    fn temp_file(name: &str) -> PathBuf {
        let base = if cfg!(miri) {
            PathBuf::from(".")
        } else {
            std::env::temp_dir()
        };
        base.join(format!("immersive-atomic-{name}-{}", std::process::id()))
    }

    #[test]
    fn replacement_failure_preserves_authoritative_bytes() {
        let target = temp_file("fail-closed");
        let _ = fs::remove_file(&target);
        fs::write(&target, b"authoritative").expect("fixture must write");

        let error = write_with(&target, b"replacement", &FailingReplacer)
            .expect_err("replacement failure must be returned");

        assert!(error.contains("injected replacement failure"));
        assert_eq!(
            fs::read(&target).expect("authoritative file must remain"),
            b"authoritative"
        );
        fs::remove_file(target).expect("fixture must be removed");
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn platform_replacement_replaces_existing_file() {
        let target = temp_file("platform-replace");
        let _ = fs::remove_file(&target);
        fs::write(&target, b"old").expect("fixture must write");

        write(&target, b"new").expect("platform replacement must succeed");

        assert_eq!(fs::read(&target).expect("target must remain"), b"new");
        fs::remove_file(target).expect("fixture must be removed");
    }

    #[test]
    fn long_path_preserves_short_paths_and_prefixes_long_ones() {
        // P2-20: short paths keep their exact spelling; near/over-MAX_PATH
        // absolute paths come back in `\\?\` form on Windows.
        let short = Path::new("relative/short.md");
        assert_eq!(super::long_path(short), short);

        let deep = "very\\deep\\".repeat(40);
        let long = std::env::temp_dir().join(deep).join("book.md");
        assert!(long.as_os_str().len() > 260);
        let normalized = super::long_path(&long);
        #[cfg(windows)]
        assert!(normalized.to_string_lossy().starts_with(r"\\?\"));
        #[cfg(not(windows))]
        assert_eq!(normalized, long);
    }
}
