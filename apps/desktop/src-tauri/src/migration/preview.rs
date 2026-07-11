use crate::storage::StorageLocations;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationScope {
    All,
    Settings,
    ReadingState,
    Podcast,
    Zhihu,
    Library,
}

#[derive(Clone, Debug)]
pub struct LegacyLocations {
    pub settings: PathBuf,
    pub immersive_state: PathBuf,
    pub mmbook_state: PathBuf,
    pub podcast_root: PathBuf,
    pub zhihu_root: PathBuf,
    pub library_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationItem {
    pub kind: String,
    pub source_path: String,
    pub target_path: String,
    pub exists: bool,
    pub bytes: u64,
    pub sensitive: bool,
    pub conflict: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPreview {
    pub preview_id: String,
    pub generated_at: String,
    pub scope: MigrationScope,
    pub items: Vec<MigrationItem>,
    pub total_bytes: u64,
    pub conflict_count: u64,
    pub sensitive_item_count: u64,
}

pub fn current_legacy_locations(library_root: PathBuf) -> Result<LegacyLocations, String> {
    let roaming = dirs::data_dir().ok_or_else(|| "Roaming AppData is unavailable".to_string())?;
    let local = dirs::data_local_dir().ok_or_else(|| "Local AppData is unavailable".to_string())?;
    Ok(LegacyLocations {
        settings: roaming.join(r"immersive-reader\settings.json"),
        immersive_state: roaming.join("immersive-reader"),
        mmbook_state: roaming.join("mmbook"),
        podcast_root: local.join(r"ImmersiveReader\podcast"),
        zhihu_root: local.join(r"ImmersiveReader\zhihu"),
        library_root,
    })
}

fn path_bytes(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(metadata.len());
    }
    fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map_err(|error| error.to_string()))
        .try_fold(0_u64, |total, entry| {
            let size = path_bytes(&entry?.path())?;
            Ok(total.saturating_add(size))
        })
}

fn included(scope: MigrationScope, item_scope: MigrationScope) -> bool {
    scope == MigrationScope::All || scope == item_scope
}

struct ItemSpec<'a> {
    scope: MigrationScope,
    kind: &'a str,
    source: PathBuf,
    target: PathBuf,
    sensitive: bool,
    in_place: bool,
}

fn item(spec: ItemSpec<'_>) -> Result<MigrationItem, String> {
    let exists = spec.source.exists();
    let source_key = spec.source.to_string_lossy().to_lowercase();
    let target_key = spec.target.to_string_lossy().to_lowercase();
    let conflict = exists && spec.target.exists() && source_key != target_key;
    Ok(MigrationItem {
        kind: spec.kind.to_string(),
        source_path: spec.source.to_string_lossy().into_owned(),
        target_path: spec.target.to_string_lossy().into_owned(),
        exists,
        bytes: if spec.in_place {
            0
        } else {
            path_bytes(&spec.source)?
        },
        sensitive: spec.sensitive,
        conflict,
    })
}

pub fn preview_for(
    legacy: &LegacyLocations,
    target: &StorageLocations,
    scope: MigrationScope,
) -> Result<MigrationPreview, String> {
    let control_db = target.data_root.join(r"App\control.db");
    let specs = vec![
        ItemSpec {
            scope: MigrationScope::Settings,
            kind: "app_settings",
            source: legacy.settings.clone(),
            target: target.settings_path.clone(),
            sensitive: false,
            in_place: legacy.settings == target.settings_path,
        },
        ItemSpec {
            scope: MigrationScope::ReadingState,
            kind: "immersive_reading_state",
            source: legacy.immersive_state.clone(),
            target: control_db.clone(),
            sensitive: true,
            in_place: false,
        },
        ItemSpec {
            scope: MigrationScope::ReadingState,
            kind: "mmbook_reading_state",
            source: legacy.mmbook_state.clone(),
            target: control_db,
            sensitive: true,
            in_place: false,
        },
        ItemSpec {
            scope: MigrationScope::Podcast,
            kind: "podcast_data",
            source: legacy.podcast_root.clone(),
            target: target.data_root.join("Podcast"),
            sensitive: true,
            in_place: false,
        },
        ItemSpec {
            scope: MigrationScope::Zhihu,
            kind: "zhihu_database",
            source: legacy.zhihu_root.join("zhihu-packer.db"),
            target: target.data_root.join(r"Zhihu\zhihu-packer.db"),
            sensitive: true,
            in_place: false,
        },
        ItemSpec {
            scope: MigrationScope::Zhihu,
            kind: "zhihu_profile",
            source: legacy.zhihu_root.join("browser-profile"),
            target: target.data_root.join(r"Private\ZhihuProfile"),
            sensitive: true,
            in_place: false,
        },
        ItemSpec {
            scope: MigrationScope::Library,
            kind: "library_manifests_and_reading",
            source: legacy.library_root.clone(),
            target: target.library_root.clone(),
            sensitive: false,
            in_place: legacy.library_root == target.library_root,
        },
        ItemSpec {
            scope: MigrationScope::Library,
            kind: "library_trash",
            source: legacy.library_root.join(".trash"),
            target: target.library_root.join(".trash"),
            sensitive: false,
            in_place: legacy.library_root == target.library_root,
        },
    ];
    let items = specs
        .into_iter()
        .filter(|spec| included(scope, spec.scope))
        .map(item)
        .collect::<Result<Vec<_>, _>>()?;
    let mut hasher = Sha256::new();
    for value in &items {
        hasher.update(serde_json::to_vec(value).map_err(|error| error.to_string())?);
        let source = Path::new(&value.source_path);
        if value.exists && source.is_file() {
            hasher.update(crate::publish::hash_file(source)?.as_bytes());
        }
    }
    Ok(MigrationPreview {
        preview_id: format!("{:x}", hasher.finalize()),
        generated_at: chrono::Utc::now().to_rfc3339(),
        total_bytes: items.iter().map(|value| value.bytes).sum(),
        conflict_count: items.iter().filter(|value| value.conflict).count() as u64,
        sensitive_item_count: items.iter().filter(|value| value.sensitive).count() as u64,
        items,
        scope,
    })
}
