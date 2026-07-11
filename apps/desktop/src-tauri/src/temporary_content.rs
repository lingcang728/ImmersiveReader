use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fs;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryItem {
    pub source: String,
    pub title: String,
    pub path: String,
    pub modified_at: Option<String>,
}

pub fn items() -> Result<Vec<TemporaryItem>, String> {
    let mut items = Vec::new();
    let path = crate::settings::local_runtime_data().join(r"podcast\output");
    if !path.exists() {
        return Ok(items);
    }
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            continue;
        }
        let is_markdown = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| matches!(value.to_ascii_lowercase().as_str(), "md" | "markdown"))
            .unwrap_or(false);
        if !is_markdown {
            continue;
        }
        let modified_at = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339());
        items.push(TemporaryItem {
            source: "podcast".to_string(),
            title: entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("临时内容")
                .to_string(),
            path: entry.path().to_string_lossy().into_owned(),
            modified_at,
        });
    }
    items.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    Ok(items)
}
