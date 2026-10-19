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
        // F-20: WRITE_THROUGH makes NTFS commit the rename's directory entry
        // to the journal before returning — the Windows analogue of fsync'ing
        // the parent directory after rename(2) on POSIX.
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

/// P3-22: true when the entry is any kind of reparse point — junction,
/// mount point, or symlink. `FileType::is_symlink` misses junctions and
/// mount points on Windows, but they can still form directory loops, so
/// recursive walks must not descend through them. `metadata` must come from
/// a non-following lookup (`symlink_metadata` / `DirEntry::metadata`).
pub(crate) fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// P3-21: an orphaned `*.tmp.<uuid>` sibling is left behind whenever the
/// process dies between temp-file create and the atomic replace. Every write
/// sweeps its own directory for temp files older than a day — residue is
/// collected lazily, exactly where it is produced, with one bounded
/// `read_dir` per write.
const STALE_TMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Matches only the temp names `TempFileGuard` produces — the suffix after
/// `.tmp.` must be a uuid, so a user's real `notes.tmp.backup` is untouched.
fn is_own_tmp_name(name: &str) -> bool {
    name.rsplit_once(".tmp.")
        .is_some_and(|(_, suffix)| uuid::Uuid::parse_str(suffix).is_ok())
}

fn sweep_tmps_older_than(dir: &Path, min_age: std::time::Duration) {
    let now = std::time::SystemTime::now();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let entries: Vec<_> = entries.flatten().collect();
    // Pending write recovery owns its temporary and backup bytes, regardless of their age.
    if entries.iter().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .ends_with(".ir-write-journal")
    }) {
        return;
    }
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_own_tmp_name(name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= min_age);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn sweep_stale_tmps(dir: &Path) {
    sweep_tmps_older_than(dir, STALE_TMP_AGE);
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

// Some EFS/filter-driver directories reject same-directory rename with ERROR_NOT_SAME_DEVICE.
// Keep encryption enabled and use a synced write-ahead record plus an original backup there.
// Readers and writers share this lock, and an interrupted copy is recovered before metadata is read.
#[cfg(all(windows, not(miri)))]
fn io_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
#[cfg(all(windows, not(miri)))]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteJournal {
    temporary: String,
    backup: Option<String>,
    new_hash: String,
    old_hash: Option<String>,
}
#[cfg(all(windows, not(miri)))]
fn journal_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.ir-write-journal",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}
#[cfg(all(windows, not(miri)))]
fn digest(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}
#[cfg(all(windows, not(miri)))]
fn sync_copy(source: &Path, target: &Path) -> io::Result<()> {
    fs::copy(source, target)?;
    OpenOptions::new().write(true).open(target)?.sync_all()
}
#[cfg(all(windows, not(miri)))]
fn recover_locked(path: &Path) -> io::Result<()> {
    let journal = journal_path(path);
    if !journal.exists() {
        return Ok(());
    }
    if fs::metadata(&journal)?.len() > 4096 {
        return Err(io::Error::other("Invalid write recovery record"));
    }
    let record: WriteJournal =
        serde_json::from_slice(&fs::read(&journal)?).map_err(io::Error::other)?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing write directory"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let safe = |entry: &str, prefix: &str| -> io::Result<PathBuf> {
        if !entry.starts_with(prefix)
            || !is_own_tmp_name(entry)
            || Path::new(entry).components().count() != 1
        {
            return Err(io::Error::other("Invalid recovery filename"));
        }
        let file = parent.join(entry);
        let metadata = fs::symlink_metadata(&file)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(io::Error::other("Invalid recovery file"));
        }
        Ok(file)
    };
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(io::Error::other("Invalid recovery target"));
        }
    }
    let temporary = safe(&record.temporary, &format!("{name}.tmp."));
    if let Ok(source) = &temporary {
        if digest(source)? != record.new_hash {
            return Err(io::Error::other("Recovery bytes failed verification"));
        }
        sync_copy(source, path)?;
    } else if let (Some(backup), Some(hash)) = (&record.backup, &record.old_hash) {
        let backup = safe(backup, &format!("{name}.backup.tmp."))?;
        if digest(&backup)? != *hash {
            return Err(io::Error::other("Original bytes failed verification"));
        }
        sync_copy(&backup, path)?;
    } else {
        return Err(io::Error::other("Write recovery bytes missing"));
    }
    fs::remove_file(&journal)?;
    if let Ok(source) = temporary {
        let _ = fs::remove_file(source);
    }
    if let Some(backup) = record.backup {
        let _ = fs::remove_file(parent.join(backup));
    }
    Ok(())
}
/// Repair an interrupted EFS fallback before an exists/metadata check.
pub(crate) fn recover_file(path: &Path) -> io::Result<()> {
    #[cfg(all(windows, not(miri)))]
    {
        let _lock = io_lock()
            .lock()
            .map_err(|_| io::Error::other("Write recovery lock unavailable"))?;
        recover_locked(&long_path(path))?;
    }
    #[cfg(any(not(windows), miri))]
    let _ = path;
    Ok(())
}
pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    #[cfg(all(windows, not(miri)))]
    {
        let _lock = io_lock()
            .lock()
            .map_err(|_| io::Error::other("Write recovery lock unavailable"))?;
        recover_locked(&long_path(path))?;
        fs::read_to_string(long_path(path))
    }
    #[cfg(any(not(windows), miri))]
    {
        fs::read_to_string(path)
    }
}
pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    #[cfg(all(windows, not(miri)))]
    {
        let _lock = io_lock()
            .lock()
            .map_err(|_| io::Error::other("Write recovery lock unavailable"))?;
        recover_locked(&long_path(path))?;
        fs::read(long_path(path))
    }
    #[cfg(any(not(windows), miri))]
    {
        fs::read(path)
    }
}
#[cfg(all(windows, not(miri)))]
fn recoverable_replace(source: &Path, target: &Path) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("Missing write directory"))?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let backup = target
        .exists()
        .then(|| parent.join(format!("{name}.backup.tmp.{}", uuid::Uuid::new_v4())));
    if let Some(backup) = &backup {
        sync_copy(target, backup)?;
    }
    let record = WriteJournal {
        temporary: source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        backup: backup.as_ref().map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        }),
        new_hash: digest(source)?,
        old_hash: backup.as_ref().map(|p| digest(p)).transpose()?,
    };
    let journal = journal_path(target);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&journal)?;
    file.write_all(&serde_json::to_vec(&record).map_err(io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = sync_copy(source, target) {
        if let Some(backup) = &backup {
            sync_copy(backup, target)?;
        } else {
            let _ = fs::remove_file(target);
        }
        let _ = fs::remove_file(&journal);
        if let Some(backup) = backup {
            let _ = fs::remove_file(backup);
        }
        return Err(error);
    }
    // Journal removal commits the new file; until this succeeds, recovery has both versions.
    fs::remove_file(journal)?;
    if let Some(backup) = backup {
        let _ = fs::remove_file(backup);
    }
    fs::remove_file(source)?;
    Ok(())
}

fn write_with(path: &Path, data: &[u8], replacer: &impl AtomicReplacer) -> Result<(), String> {
    // P2-20: normalize once — the temp sibling, parent mkdir and final
    // replace all inherit the `\\?\` form when the target is near MAX_PATH.
    let normalized = long_path(path);
    let path = normalized.as_path();
    #[cfg(all(windows, not(miri)))]
    let _lock = io_lock()
        .lock()
        .map_err(|_| "Write recovery lock unavailable".to_string())?;
    #[cfg(all(windows, not(miri)))]
    recover_locked(path).map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Atomic write target has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    // P3-21: collect orphaned temp files left by earlier crashes before
    // creating this write's own temp sibling (fresh — never swept).
    sweep_stale_tmps(parent);
    let (mut guard, mut file) = TempFileGuard::create(path).map_err(|error| error.to_string())?;
    file.write_all(data).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    if let Err(error) = replacer.replace(&guard.path, path) {
        #[cfg(all(windows, not(miri)))]
        {
            use std::os::windows::fs::MetadataExt;
            let encrypted =
                fs::metadata(&guard.path).is_ok_and(|meta| meta.file_attributes() & 0x4000 != 0);
            if error.raw_os_error() == Some(17) && encrypted {
                recoverable_replace(&guard.path, path).map_err(|error| error.to_string())?;
            } else {
                return Err(error.to_string());
            }
        }
        #[cfg(any(not(windows), miri))]
        return Err(error.to_string());
    }
    guard.disarm();
    Ok(())
}

pub fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    write_with(path, data, &PlatformReplacer)
}

/// Copy `source` to `target` and flush the destination before returning.
/// `fs::copy` leaves bytes in the write cache — a crash right after could
/// commit a zero-filled/truncated file under a good-looking name inside a
/// staged book tree. Syncing here keeps the publish/import staging durable
/// through the rename that follows. Returns the copied byte count so callers
/// can detect a torn copy (`source` shrank mid-copy) instead of shelving it.
pub(crate) fn copy_file_synced(source: &Path, target: &Path) -> Result<u64, String> {
    let normalized_source = long_path(source);
    let normalized_target = long_path(target);
    if let Some(parent) = normalized_target.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    #[cfg(windows)]
    {
        // CopyFile propagates EFS keys and readonly attributes. Managed copies
        // must inherit their destination directory's policy instead: copying
        // a readable encrypted cache file can otherwise fail with error 6000.
        let mut input = File::open(&normalized_source).map_err(|e| e.to_string())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&normalized_target)
            .map_err(|e| e.to_string())?;
        if windows_same_file(&input, &output).map_err(|e| e.to_string())? {
            return Err("复制来源与目标是同一个文件".to_string());
        }
        output.set_len(0).map_err(|e| e.to_string())?;
        let bytes = io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    #[cfg(not(windows))]
    {
        let bytes = fs::copy(&normalized_source, &normalized_target).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .write(true)
            .open(&normalized_target)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string()).map(|_| bytes)
    }
}

#[cfg(windows)]
fn windows_same_file(left: &File, right: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    fn identity(file: &File) -> io::Result<(u32, u32, u32)> {
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: Category 8 (FFI). The handle is owned by a live File; the
        // output struct is read only after the API reports full initialization.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetFileInformationByHandle succeeded and initialized info.
        let info = unsafe { info.assume_init() };
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
    Ok(identity(left)? == identity(right)?)
}

#[cfg(test)]
mod tests {
    use super::{write, write_with, AtomicReplacer};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    #[test]
    #[cfg(all(windows, not(miri)))]
    // Windows-only fixture cleanup restores the readonly bit, not Unix modes.
    #[allow(clippy::permissions_set_readonly_false)]
    fn managed_copy_of_readonly_source_is_writable_and_aliases_preserve_source() {
        let dir = std::env::temp_dir().join(format!("ir-copy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.md");
        let target = dir.join("target.md");
        fs::write(&source, b"original").unwrap();
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).unwrap();
        assert_eq!(super::copy_file_synced(&source, &target).unwrap(), 8);
        fs::write(&target, b"editable managed copy").unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"original");
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&source, permissions).unwrap();
        let alias = dir.join("alias.md");
        fs::hard_link(&source, &alias).unwrap();
        assert!(super::copy_file_synced(&source, &source).is_err());
        assert!(super::copy_file_synced(&source, &alias).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        fs::remove_dir_all(dir).unwrap();
    }

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
    #[cfg(all(windows, not(miri)))]
    fn interrupted_efs_copy_recovers_verified_new_bytes_and_keeps_original_on_rejection() {
        let target = temp_file("efs-recovery");
        let name = target.file_name().unwrap().to_string_lossy();
        let source = target.with_file_name(format!("{name}.tmp.{}", uuid::Uuid::new_v4()));
        let backup = target.with_file_name(format!("{name}.backup.tmp.{}", uuid::Uuid::new_v4()));
        fs::write(&source, b"new authoritative data").unwrap();
        fs::write(&backup, b"old authoritative data").unwrap();
        fs::write(&target, b"torn").unwrap();
        let record = super::WriteJournal {
            temporary: source.file_name().unwrap().to_string_lossy().into_owned(),
            backup: Some(backup.file_name().unwrap().to_string_lossy().into_owned()),
            new_hash: super::digest(&source).unwrap(),
            old_hash: Some(super::digest(&backup).unwrap()),
        };
        fs::write(
            super::journal_path(&target),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert_eq!(super::read(&target).unwrap(), b"new authoritative data");
        assert!(!source.exists());
        assert!(!backup.exists());
        assert!(!super::journal_path(&target).exists());
        let invalid = super::WriteJournal {
            temporary: "../foreign.tmp.00000000-0000-0000-0000-000000000000".into(),
            backup: None,
            new_hash: "bad".into(),
            old_hash: None,
        };
        fs::write(
            super::journal_path(&target),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert!(super::read(&target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"new authoritative data");
        fs::remove_file(super::journal_path(&target)).unwrap();
        fs::remove_file(target).unwrap();
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
    #[cfg_attr(miri, ignore)]
    fn sweep_removes_only_own_stale_tmp_names() {
        // P3-21: orphaned `<name>.<ext>.tmp.<uuid>` siblings are collected;
        // same-shaped foreign files (non-uuid suffix) are left alone.
        let dir =
            std::env::temp_dir().join(format!("immersive-atomic-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir must exist");
        let orphan = dir.join(format!("old.md.tmp.{}", uuid::Uuid::new_v4()));
        fs::write(&orphan, b"orphan").expect("orphan must write");
        let lookalike = dir.join("notes.tmp.backup");
        fs::write(&lookalike, b"keep").expect("lookalike must write");
        let plain = dir.join("chapter.md");
        fs::write(&plain, b"keep").expect("plain must write");

        // Zero min_age = everything matching is "stale" — the 24h threshold is
        // exercised through the public write() path in production.
        super::sweep_tmps_older_than(&dir, std::time::Duration::ZERO);

        assert!(!orphan.exists(), "stale own tmp must be swept");
        assert!(lookalike.exists(), "foreign tmp name must stay");
        assert!(plain.exists(), "regular file must stay");
        fs::remove_dir_all(dir).expect("dir must be removed");
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
