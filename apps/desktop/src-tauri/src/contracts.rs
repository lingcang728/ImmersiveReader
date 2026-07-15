use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub path: String,
    pub title: String,
    pub date: Option<String>,
    #[serde(default)]
    pub vote_count: u64,
    #[serde(default)]
    pub word_count: u64,
    #[serde(default)]
    pub metadata_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub book_id: String,
    pub title: String,
    pub source: String,
    pub source_id: Option<String>,
    pub generated_at: String,
    pub updated_at: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ReadingProgress {
    pub schema_version: u32,
    pub current: String,
    pub position: f64,
    pub read: Vec<String>,
    pub updated: String,
}

impl ReadingProgress {
    pub fn empty(first_chapter: &str) -> Self {
        Self {
            schema_version: 1,
            current: first_chapter.to_string(),
            position: 0.0,
            read: Vec::new(),
            updated: String::new(),
        }
    }
}

pub fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err("Unsupported manifest schema version".to_string());
    }
    if manifest.book_id.trim().is_empty() || manifest.title.trim().is_empty() {
        return Err("Manifest bookId and title are required".to_string());
    }
    if !matches!(manifest.source.as_str(), "zhihu" | "manual" | "podcast") {
        return Err(format!("Unsupported book source: {}", manifest.source));
    }
    if manifest.chapters.is_empty() {
        return Err("Manifest must contain at least one chapter".to_string());
    }
    if chrono::DateTime::parse_from_rfc3339(&manifest.generated_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&manifest.updated_at).is_err()
    {
        return Err("Manifest generatedAt and updatedAt must be RFC-3339 date-times".to_string());
    }
    let mut ids = HashSet::new();
    for chapter in &manifest.chapters {
        if chapter.id.trim().is_empty() || chapter.title.trim().is_empty() {
            return Err("Chapter id and title are required".to_string());
        }
        if !ids.insert(chapter.id.as_str()) {
            return Err(format!("Duplicate chapter id: {}", chapter.id));
        }
        if !is_safe_relative_path(&chapter.path) {
            return Err(format!("Unsafe chapter path: {}", chapter.path));
        }
        if let Some(date) = chapter.date.as_deref() {
            if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                return Err(format!("Invalid chapter date: {date}"));
            }
        }
        if let Some(status) = chapter.metadata_status.as_deref() {
            if !matches!(status, "complete" | "inferred") {
                return Err(format!("Unsupported chapter metadata status: {status}"));
            }
        }
    }
    Ok(())
}

pub fn validate_reading(progress: &ReadingProgress, manifest: &Manifest) -> Result<(), String> {
    if progress.schema_version != 1 {
        return Err("Unsupported reading schema version".to_string());
    }
    if !(0.0..=1.0).contains(&progress.position) {
        return Err("Reading position must be between 0 and 1".to_string());
    }
    let chapter_ids: HashSet<&str> = manifest
        .chapters
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    if progress.current.is_empty() || !chapter_ids.contains(progress.current.as_str()) {
        return Err("Current chapter is not in the manifest".to_string());
    }
    let mut read_ids = HashSet::new();
    for id in &progress.read {
        if !chapter_ids.contains(id.as_str()) {
            return Err(format!("Read chapter is not in the manifest: {id}"));
        }
        if !read_ids.insert(id.as_str()) {
            return Err(format!("Duplicate read chapter: {id}"));
        }
    }
    if chrono::DateTime::parse_from_rfc3339(&progress.updated).is_err() {
        return Err("Reading updated must be an RFC-3339 date-time".to_string());
    }
    Ok(())
}

pub fn is_safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !Path::new(value).is_absolute()
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::{validate_manifest, validate_reading, Manifest, ReadingProgress};

    fn fixture_manifest() -> Manifest {
        let raw = include_str!("../../../../packages/contracts/fixtures/manifest.valid.json");
        serde_json::from_str(raw).expect("fixture must deserialize")
    }

    #[test]
    fn accepts_shared_manifest_fixture() {
        let manifest = fixture_manifest();
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn rejects_duplicate_ids_and_traversal() {
        let mut manifest = fixture_manifest();
        let duplicate = manifest.chapters[0].clone();
        manifest.chapters.push(duplicate);
        assert!(validate_manifest(&manifest).is_err());

        manifest.chapters.pop();
        manifest.chapters[0].path = "../outside.md".to_string();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_empty_manifest_and_invalid_metadata() {
        let mut manifest = fixture_manifest();
        manifest.chapters.clear();
        assert!(validate_manifest(&manifest).is_err());

        let mut manifest = fixture_manifest();
        manifest.chapters[0].metadata_status = Some("unknown".to_string());
        assert!(validate_manifest(&manifest).is_err());

        let mut manifest = fixture_manifest();
        manifest.chapters[0].date = Some("2026-02-30".to_string());
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_unknown_json_fields_and_preserves_metadata_status() {
        let raw = include_str!("../../../../packages/contracts/fixtures/manifest.valid.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
        value["unexpected"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<Manifest>(value).is_err());

        let manifest = fixture_manifest();
        let value = serde_json::to_value(manifest).expect("manifest serializes");
        assert_eq!(value["chapters"][0]["metadataStatus"], "complete");
    }

    #[test]
    fn validates_shared_reading_fixture() {
        let manifest = fixture_manifest();
        let raw = include_str!("../../../../packages/contracts/fixtures/reading.valid.json");
        let progress: ReadingProgress =
            serde_json::from_str(raw).expect("fixture must deserialize");
        assert!(validate_reading(&progress, &manifest).is_ok());
    }

    #[test]
    fn rejects_empty_current_and_invalid_updated_timestamp() {
        let manifest = fixture_manifest();
        let mut progress: ReadingProgress = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reading.valid.json"
        ))
        .expect("fixture must deserialize");
        progress.current.clear();
        assert!(validate_reading(&progress, &manifest).is_err());
        progress.current = manifest.chapters[0].id.clone();
        progress.updated = "2026-07-10".to_string();
        assert!(validate_reading(&progress, &manifest).is_err());
    }
}
