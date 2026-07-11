use super::transaction::PublishTransaction;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

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

pub fn managed_relative(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Publish path must be relative to the Library root".to_string());
    }
    Ok(root.join(relative_path))
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
    let manifest_hash = hash_file(&manifest_path)?;
    let provenance_hash = hash_file(&provenance_path)?;
    if manifest_hash != transaction.manifest_sha256
        || provenance_hash != transaction.provenance_sha256
    {
        return Err("Published metadata hash mismatch".to_string());
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let provenance: Value =
        serde_json::from_slice(&fs::read(&provenance_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
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
    Ok(())
}
