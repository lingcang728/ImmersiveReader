use super::transaction::PublishTransaction;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Relative paths under managed roots must satisfy the shared contract
/// (`contracts::is_safe_relative_path` / TS `requireRelativePath` /
/// `trash::parse_relative`): forward-slash only — not blank, no leading `/`,
/// no drive prefix, no `\`, no NUL, and no empty / `.` / `..` segments.
/// `Path::components()` cannot enforce this: it silently normalizes `a/./b`
/// and treats `\` as a separator on Windows, so the raw string is checked.
pub fn managed_relative(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !crate::contracts::is_safe_relative_path(relative) {
        return Err("Publish path must be relative to the Library root".to_string());
    }
    Ok(root.join(relative))
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Published metadata is missing {key}"))
}

pub fn validate_book(
    root: &Path,
    relative: &str,
    transaction: &PublishTransaction,
) -> Result<(), String> {
    let book_root = managed_relative(root, relative)?;
    let manifest_path = book_root.join("manifest.json");
    let provenance_path = book_root.join("provenance.json");
    // Hash and parse the SAME bytes — a `hash_file` + `fs::read` pair reads
    // the file twice, so a mid-check rewrite could satisfy the journal hash
    // while the parsed JSON differs (or vice versa).
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let provenance_bytes = fs::read(&provenance_path).map_err(|error| error.to_string())?;
    let manifest_hash = format!("{:x}", Sha256::digest(&manifest_bytes));
    let provenance_hash = format!("{:x}", Sha256::digest(&provenance_bytes));
    if manifest_hash != transaction.manifest_sha256
        || provenance_hash != transaction.provenance_sha256
    {
        return Err("Published metadata hash mismatch".to_string());
    }
    let manifest: Value =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    let provenance: Value =
        serde_json::from_slice(&provenance_bytes).map_err(|error| error.to_string())?;
    if required_string(&manifest, "bookId")? != transaction.book_id
        || required_string(&provenance, "bookId")? != transaction.book_id
    {
        return Err("Published book id does not match the transaction".to_string());
    }
    if required_string(&manifest, "sourceId")? != required_string(&provenance, "sourceId")? {
        return Err("Manifest and provenance source ids differ".to_string());
    }
    if provenance.get("revision").and_then(Value::as_u64) != Some(transaction.revision) {
        return Err("Provenance revision does not match the transaction".to_string());
    }
    if required_string(&provenance, "manifestSha256")? != transaction.manifest_sha256 {
        return Err("Provenance manifest hash does not match the manifest".to_string());
    }
    // Chapter payload check: the hash pair above only pins manifest.json and
    // provenance.json — a crash during staging could leave a truncated or
    // zero-length chapter file that every later phase would still call
    // "valid". Require every manifest chapter to resolve inside the book
    // and exist as a non-empty file. An absent/empty chapters array is
    // already pinned by the manifest hash — nothing extra to verify.
    if let Some(chapters) = manifest.get("chapters").and_then(Value::as_array) {
        for chapter in chapters {
            let relative = chapter
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "Published chapter is missing its path".to_string())?;
            if !crate::contracts::is_safe_relative_path(relative) {
                return Err(format!("Published chapter path is unsafe: {relative}"));
            }
            let chapter_path = book_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            let metadata = fs::metadata(&chapter_path)
                .map_err(|error| format!("Published chapter {relative} is missing: {error}"))?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err(format!("Published chapter {relative} is empty"));
            }
        }
    }
    Ok(())
}
