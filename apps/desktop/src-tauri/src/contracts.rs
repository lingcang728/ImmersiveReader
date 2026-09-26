use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The shared schema (`const: 1`) and the TS validator (`value !== 1`) accept
/// any numeric 1 — including the float form `1.0` — while serde's `u32`
/// rejects floats. Deserialize the same set here so all three implementations
/// agree on `schemaVersion`.
pub(crate) fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value.as_f64() {
        Some(1.0) => Ok(1),
        _ => Err(serde::de::Error::custom("unsupported schema version")),
    }
}

/// Schema `integer` and TS `requireNonNegativeInteger` accept integral floats
/// (`1e2`, `100.0`); serde's `u64` rejects float tokens. Deserialize the same
/// set here so all three read paths agree.
pub(crate) fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                return Ok(value);
            }
            match number.as_f64() {
                Some(float) if float.fract() == 0.0 && float >= 0.0 && float < u64::MAX as f64 => {
                    Ok(float as u64)
                }
                _ => Err(serde::de::Error::custom("must be a non-negative integer")),
            }
        }
        _ => Err(serde::de::Error::custom("must be a non-negative integer")),
    }
}

/// `Option<u64>` variant of `deserialize_u64`: accepts integral floats,
/// `null` and absent keys map to `None`.
pub(crate) fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                return Ok(Some(value));
            }
            match number.as_f64() {
                Some(float) if float.fract() == 0.0 && float >= 0.0 && float < u64::MAX as f64 => {
                    Ok(Some(float as u64))
                }
                _ => Err(serde::de::Error::custom("must be a non-negative integer")),
            }
        }
        _ => Err(serde::de::Error::custom("must be a non-negative integer")),
    }
}

/// Schema/TS reject an explicit `null` on optional string fields — the key
/// must be omitted instead. `Option<String>` would silently accept `null` as
/// `None`; reject it so the read paths agree.
pub(crate) fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Null => Err(serde::de::Error::custom(
            "explicit null is not allowed; omit the field instead",
        )),
        other => String::deserialize(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: String,
    pub path: String,
    pub title: String,
    // Schema/TS reject an explicit `null`; omit the key instead so the JSON we
    // write always validates, and reject it on read via deserialize_with.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_string",
        default
    )]
    pub date: Option<String>,
    // `voteCount`/`wordCount` are `required` in manifest.schema.json — do not
    // silently default what the other implementations treat as an error.
    #[serde(deserialize_with = "deserialize_u64")]
    pub vote_count: u64,
    // Canonical counting (P3-29): non-whitespace Unicode scalar values —
    // `content.chars().filter(|c| !c.is_whitespace()).count()` in importer.rs
    // and podcast/publish.rs. zhihu-packer historically counted UTF-16 code
    // units, which differs for non-BMP characters; the Rust scalar count is
    // the canonical form (note for the contracts/TS side).
    #[serde(deserialize_with = "deserialize_u64")]
    pub word_count: u64,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_string",
        default
    )]
    pub metadata_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub book_id: String,
    pub title: String,
    pub source: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_string",
        default
    )]
    pub source_id: Option<String>,
    pub generated_at: String,
    pub updated_at: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ReadingProgress {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub current: String,
    pub position: f64,
    pub read: Vec<String>,
    pub updated: String,
}

impl ReadingProgress {
    /// In-memory default returned by `load_progress` before validation.
    /// `updated` is an intentional empty sentinel for "never persisted":
    /// `scan_library` maps it to `last_read_at: None` and `merge_progress`
    /// orders it oldest. `validate_reading` (and therefore `save_progress`)
    /// still requires a real RFC-3339 timestamp — an empty `updated` must
    /// never reach disk because the schema and TS `parseReadingState` reject
    /// it (P3-16: this is the "callers skip validation" semantic the rest of
    /// the codebase already assumes).
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
    // sourceId is optional, but a present value must be non-blank — the
    // schema (`minLength: 1`) and TS `requireString` reject `""`; TS also
    // rejects whitespace-only. Rust trims on `White_Space` (NEL yes, FEFF
    // no) — matching trim-based behavior rather than JS `trim()`.
    if manifest
        .source_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("Manifest sourceId must not be blank".to_string());
    }
    if manifest.chapters.is_empty() {
        return Err("Manifest must contain at least one chapter".to_string());
    }
    if !is_rfc3339_date_time(&manifest.generated_at) || !is_rfc3339_date_time(&manifest.updated_at)
    {
        return Err("Manifest generatedAt and updatedAt must be RFC-3339 date-times".to_string());
    }
    let mut ids = HashSet::new();
    // P3-29: NTFS is case-insensitive — `a.md` and `A.MD` address the same
    // file — so path uniqueness is enforced on the case-folded form, not the
    // raw string.
    let mut paths = HashSet::new();
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
        if !paths.insert(chapter.path.to_lowercase()) {
            return Err(format!("Duplicate chapter path: {}", chapter.path));
        }
        if let Some(date) = chapter.date.as_deref() {
            if !is_iso_calendar_date(date) {
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
    if !is_rfc3339_date_time(&progress.updated) {
        return Err("Reading updated must be an RFC-3339 date-time".to_string());
    }
    Ok(())
}

/// `publication.json` sidecar validation — the Rust-side contract twin of
/// `validate_manifest`/`validate_reading`. There is no TS twin yet (the
/// frontend never reads `publication.json` directly; it consumes commands
/// that answer already-validated data), so this stays a Rust-only gate until
/// the fixture set grows a `publication.valid.json`.
pub fn validate_publication(publication: &crate::epub::Publication) -> Result<(), String> {
    if publication.schema_version != 1 {
        return Err("Unsupported publication schema version".to_string());
    }
    if publication.format != crate::epub::Publication::FORMAT_EPUB {
        return Err(format!(
            "Unsupported publication format: {}",
            publication.format
        ));
    }
    if !matches!(publication.epub_version.as_str(), "2" | "3") {
        return Err(format!(
            "Unsupported EPUB version: {}",
            publication.epub_version
        ));
    }
    if publication.title.trim().is_empty() {
        return Err("Publication title is required".to_string());
    }
    for (field, value) in [
        ("creator", &publication.creator),
        ("language", &publication.language),
    ] {
        if value.as_deref().is_some_and(|text| text.trim().is_empty()) {
            return Err(format!("Publication {field} must not be blank"));
        }
    }
    if let Some(cover) = publication.cover.as_deref() {
        if !is_safe_relative_path(cover) {
            return Err(format!("Unsafe cover path: {cover}"));
        }
    }
    if publication.spine.is_empty() {
        return Err("Publication spine must contain at least one chapter".to_string());
    }
    let mut spine_ids = HashSet::new();
    for id in &publication.spine {
        if id.trim().is_empty() {
            return Err("Publication spine contains a blank chapter id".to_string());
        }
        if !spine_ids.insert(id.as_str()) {
            return Err(format!("Duplicate spine chapter id: {id}"));
        }
    }
    let mut nav_stack: Vec<&crate::epub::NavItem> = publication.nav.iter().collect();
    while let Some(item) = nav_stack.pop() {
        if item.title.trim().is_empty() || item.chapter_id.trim().is_empty() {
            return Err("Nav item title and chapterId are required".to_string());
        }
        if !spine_ids.contains(item.chapter_id.as_str()) {
            return Err(format!(
                "Nav item references a chapter outside the spine: {}",
                item.chapter_id
            ));
        }
        nav_stack.extend(item.children.iter());
    }
    for (key, path) in &publication.resources {
        if key.trim().is_empty() {
            return Err("Publication resource id must not be blank".to_string());
        }
        if !is_safe_relative_path(path) {
            return Err(format!("Unsafe resource path: {path}"));
        }
    }
    if publication
        .unsupported
        .iter()
        .any(|tag| tag.trim().is_empty())
    {
        return Err("Publication unsupported flags must not be blank".to_string());
    }
    Ok(())
}

/// Anchor variants of `ReaderLocator` — mirrors `ReaderAnchor` in
/// `packages/contracts/src/index.ts` and the `anchor` oneOf in
/// `reader-locator.schema.json`. Internally tagged by `kind`; serde's
/// `deny_unknown_fields` cannot combine with internal tagging, so unknown
/// fields inside an anchor variant are tolerated here while the schema and
/// TS legs still reject them (the same documented asymmetry as
/// `BookProvenance`/`PublishTransaction`).
// TODO(reader): `ReaderLocator`/`ReaderAnchor`/`validate_reader_locator` are
// the shared contract legs for the reader.db locator JSON — today
// `reader_db::save_locator` stores the payload opaquely, so nothing outside
// tests constructs them yet. Wire the validator into the save/load commands
// when the reader surface lands.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReaderAnchor {
    /// DOM element path inside the chapter document (e.g. `body/p[3]`) —
    /// a document path, not a filesystem path, so no safe-path rules apply.
    Element { path: String },
    /// Text-quote anchor: `quote` is the matched text, `offset` its
    /// code-point index into the chapter's plain text.
    Text {
        quote: String,
        #[serde(deserialize_with = "deserialize_u64")]
        offset: u64,
    },
    /// Pure scroll-ratio restore point.
    Ratio,
}

/// Last-read position record stored per book in reader.db
/// (`reader_locators.locator_json`). Mirrors `ReaderLocator` in
/// `packages/contracts/src/index.ts`.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ReaderLocator {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub book_id: String,
    pub chapter_id: String,
    pub anchor: ReaderAnchor,
    /// Scroll fraction 0..=1 — the fallback when the anchor misses.
    pub ratio: f64,
    /// RFC-3339 date-time of the last locator write.
    pub updated: String,
}

/// `ReaderLocator` validation — the Rust twin of `parseReaderLocator` (TS)
/// and `reader-locator.schema.json`. There is no manifest cross-check: the
/// record is stored per book and `chapter_id` is shape-checked only.
#[allow(dead_code)]
pub fn validate_reader_locator(locator: &ReaderLocator) -> Result<(), String> {
    if locator.schema_version != 1 {
        return Err("Unsupported locator schema version".to_string());
    }
    if locator.book_id.trim().is_empty() {
        return Err("Locator bookId is required".to_string());
    }
    if locator.chapter_id.trim().is_empty() {
        return Err("Locator chapterId is required".to_string());
    }
    match &locator.anchor {
        ReaderAnchor::Element { path } => {
            if path.trim().is_empty() {
                return Err("Locator element anchor path is required".to_string());
            }
        }
        ReaderAnchor::Text { quote, .. } => {
            if quote.trim().is_empty() {
                return Err("Locator text anchor quote is required".to_string());
            }
        }
        ReaderAnchor::Ratio => {}
    }
    if !locator.ratio.is_finite() || !(0.0..=1.0).contains(&locator.ratio) {
        return Err("Locator ratio must be between 0 and 1".to_string());
    }
    if !is_rfc3339_date_time(&locator.updated) {
        return Err("Locator updated must be an RFC-3339 date-time".to_string());
    }
    Ok(())
}

fn fixed_digits(value: &str, digits: usize) -> Option<u32> {
    if value.len() == digits && value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

/// Mirrors `requireIsoDate` in packages/contracts/src/index.ts:
/// `^\d{4}-\d{2}-\d{2}$` plus a real calendar day. `chrono`'s `%Y-%m-%d`
/// parsing is width-tolerant (it accepts `2026-2-3`), so the shape is checked
/// explicitly before delegating day/month validity to `NaiveDate`.
fn is_iso_calendar_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (
        fixed_digits(year, 4),
        fixed_digits(month, 2),
        fixed_digits(day, 2),
    ) else {
        return false;
    };
    chrono::NaiveDate::from_ymd_opt(year as i32, month, day).is_some()
}

/// Mirrors `requireIsoDateTime` in packages/contracts/src/index.ts:
/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$` with real
/// field ranges. This is deliberately stricter than
/// `chrono::DateTime::parse_from_rfc3339`, which also accepts ` `/`t`
/// separators, lowercase `z`, and leap-second `:60` — all rejected by TS.
fn is_rfc3339_date_time(value: &str) -> bool {
    let Some((date, time_and_offset)) = value.split_once('T') else {
        return false;
    };
    if !is_iso_calendar_date(date) {
        return false;
    }
    let (time, offset_ok) = match time_and_offset.strip_suffix('Z') {
        Some(time) => (time, true),
        None => match time_and_offset.find(['+', '-']) {
            Some(index) => {
                let (time, offset) = time_and_offset.split_at(index);
                // `±HH:MM` — checked with str ops only (no byte indexing, so no
                // char-boundary panics on multibyte input).
                let valid = offset
                    .strip_prefix(['+', '-'])
                    .and_then(|rest| rest.split_once(':'))
                    .is_some_and(|(hour, minute)| {
                        fixed_digits(hour, 2).is_some_and(|hour| hour <= 23)
                            && fixed_digits(minute, 2).is_some_and(|minute| minute <= 59)
                    });
                (time, valid)
            }
            None => (time_and_offset, false),
        },
    };
    if !offset_ok {
        return false;
    }
    let (hms, fraction_ok) = match time.split_once('.') {
        Some((hms, fraction)) => (
            hms,
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit()),
        ),
        None => (time, true),
    };
    if !fraction_ok {
        return false;
    }
    let mut parts = hms.split(':');
    let (Some(hour), Some(minute), Some(second)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    matches!(
        (
            fixed_digits(hour, 2),
            fixed_digits(minute, 2),
            fixed_digits(second, 2),
        ),
        (Some(hour), Some(minute), Some(second)) if hour < 24 && minute < 60 && second < 60
    )
}

/// P3-19: Windows reserved device base names — `CON`, `PRN`, `AUX`, `NUL`,
/// `COM1`..`COM9`, `LPT1`..`LPT9` — matched case-insensitively against the
/// segment text before the first `.` (`con.md` still resolves to the CON
/// device, not a file). Keep in lockstep with the identical list in TS
/// `requireRelativePath` (packages/contracts/src/index.ts).
pub(crate) fn is_reserved_device_name(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment);
    let upper = base.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

/// P3-19: one `/`-separated segment must be a usable Win32 file name —
/// non-empty, not `.`/`..`, free of `\` NUL `:` `<` `>` `|` `?` `*`, not
/// ending in `.` or ` ` (Win32 silently strips those, producing a name the
/// stored relative path no longer matches), and not a reserved device name
/// (a `canonicalize`/CreateFile on one opens the device node — these are
/// refused before any path is resolved). Shared by `is_safe_relative_path`
/// for chapter paths and publish/trash managed paths; identical rules live
/// in TS `requireRelativePath`.
pub(crate) fn is_safe_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment
            .bytes()
            .any(|byte| matches!(byte, b'\\' | 0 | b':' | b'<' | b'>' | b'|' | b'?' | b'*'))
        && !segment.ends_with('.')
        && !segment.ends_with(' ')
        && !is_reserved_device_name(segment)
}

/// P-11-F11: one shared shelf-name sanitizer — every pipeline that derives
/// a Library folder from user/sidecar text (podcast publish, manual import)
/// must apply the same mapping so no path produces a Win32-invalid segment.
/// Illegal characters (`< > : " / \ | ? *` and control chars) become spaces,
/// runs collapse, the result is trimmed of spaces/dots and capped at 80
/// chars; an empty remainder falls back to `fallback`. Reserved device names
/// are NOT handled here — the caller decides how to mangle them (importer
/// appends `_`).
pub(crate) fn sanitize_shelf_name(raw: &str, fallback: &str) -> String {
    let mut name = raw
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>();
    while name.contains("  ") {
        name = name.replace("  ", " ");
    }
    let name = name.trim().trim_matches('.').to_string();
    let mut name = if name.is_empty() {
        fallback.to_string()
    } else {
        name
    };
    if name.chars().count() > 80 {
        name = name.chars().take(80).collect::<String>();
        name = name.trim().trim_matches('.').to_string();
    }
    if name.is_empty() {
        fallback.to_string()
    } else {
        name
    }
}

/// Mirrors `requireRelativePath` in packages/contracts/src/index.ts and the
/// `path` pattern in manifest.schema.json: forward-slash relative paths only —
/// non-blank (not empty or whitespace-only), no leading `/`, no drive prefix,
/// and every `/`-separated segment satisfying `is_safe_path_segment` (no
/// empty / `.` / `..` segments, no `\` NUL `:` `<` `>` `|` `?` `*`, no
/// trailing `.`/` `, no reserved device names). Keep all implementations in
/// lockstep.
pub fn is_safe_relative_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive_prefixed = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    !value.trim().is_empty()
        && !value.starts_with('/')
        && !drive_prefixed
        && value.split('/').all(is_safe_path_segment)
}

#[cfg(test)]
mod tests {
    use super::{
        validate_manifest, validate_publication, validate_reader_locator, validate_reading,
        Manifest, ReaderLocator, ReadingProgress,
    };

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

    #[test]
    fn rejects_missing_required_chapter_fields() {
        // voteCount/wordCount are `required` in the schema and in TS; serde must
        // not default them to zero.
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture json");
        let chapter = value["chapters"][0].as_object().expect("chapter").clone();
        let mut missing = chapter.clone();
        missing.remove("voteCount");
        value["chapters"][0] = serde_json::Value::Object(missing);
        assert!(serde_json::from_value::<Manifest>(value.clone()).is_err());
        let mut missing = chapter;
        missing.remove("wordCount");
        value["chapters"][0] = serde_json::Value::Object(missing);
        assert!(serde_json::from_value::<Manifest>(value).is_err());
    }

    #[test]
    fn serialized_manifest_omits_absent_optional_fields() {
        // P1-21 regression: Rust-written manifests must satisfy the shared
        // schema and TS parseManifest, which both reject explicit nulls.
        let manifest = Manifest {
            schema_version: 1,
            book_id: "manual:fixture".to_string(),
            title: "手动导入 fixture".to_string(),
            source: "manual".to_string(),
            source_id: None,
            generated_at: "2026-07-10T00:00:00.000Z".to_string(),
            updated_at: "2026-07-10T00:00:00.000Z".to_string(),
            chapters: vec![super::Chapter {
                id: "manual:fixture-1".to_string(),
                path: "part/001.md".to_string(),
                title: "第一篇".to_string(),
                date: None,
                vote_count: 0,
                word_count: 10,
                metadata_status: None,
            }],
        };
        let value = serde_json::to_value(&manifest).expect("manifest serializes");
        assert!(value.get("sourceId").is_none());
        assert!(value["chapters"][0].get("date").is_none());
        assert!(value["chapters"][0].get("metadataStatus").is_none());
        // The serialized form must be byte-identical in shape to the fixture the
        // schema (verify_contract_parity.py) and TS tests validate.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.optional-omitted.json"
        ))
        .expect("fixture must parse");
        assert_eq!(value, fixture);
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn rejects_unsafe_chapter_paths_like_ts() {
        // All of these are rejected by TS requireRelativePath and the schema
        // pattern; previously several slipped through is_safe_relative_path.
        for path in [
            "",
            "   ",
            "/abs.md",
            "C:abs.md",
            "c:/abs.md",
            "sub\\001.md",
            "a\0b.md",
            "a//b.md",
            "./a.md",
            "a/./b.md",
            "..",
            "../a.md",
            "a/../b.md",
            "a/",
            // P3-19 additions: Win32-forbidden characters, segments Win32
            // would silently rewrite, and reserved device names in both bare
            // and `name.ext` forms — all rejected before any canonicalize.
            "a:b.md",
            "a<b.md",
            "a>b.md",
            "a|b.md",
            "a?b.md",
            "a*b.md",
            "a./b.md",
            "a /b.md",
            "trail./x.md",
            "dir /x.md",
            "con.md",
            "CON",
            "nul/sub.md",
            "aux.txt",
            "com1",
            "LpT9/001.md",
        ] {
            assert!(
                !super::is_safe_relative_path(path),
                "path must be rejected: {path:?}"
            );
        }
        for path in [
            "001.md",
            "sub/002.md",
            ".hidden/001.md",
            "第一篇 .md",
            // P3-19: outside the reserved set — only COM1..COM9/LPT1..LPT9 are
            // device names, and dots inside a name are legal.
            "com0.md",
            "COM10.md",
            "lpt0.md",
            "content.md",
            "a..b.md",
        ] {
            assert!(
                super::is_safe_relative_path(path),
                "path must be accepted: {path:?}"
            );
        }
    }

    #[test]
    fn rejects_case_insensitive_duplicate_chapter_paths() {
        // P3-29: `a.md` and `A.MD` collide on NTFS — the manifest must refuse
        // both spellings of one file.
        let mut manifest = fixture_manifest();
        let mut second = manifest.chapters[0].clone();
        second.id = "answer:fixture-2".to_string();
        second.path = manifest.chapters[0].path.to_uppercase();
        manifest.chapters.push(second);
        let error = validate_manifest(&manifest).expect_err("duplicate path must fail");
        assert!(error.contains("Duplicate chapter path"));

        // Distinct names still pass.
        let mut manifest = fixture_manifest();
        let mut second = manifest.chapters[0].clone();
        second.id = "answer:fixture-2".to_string();
        second.path = "002.md".to_string();
        manifest.chapters.push(second);
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn rejects_noncanonical_dates_and_datetimes_like_ts() {
        let mut manifest = fixture_manifest();
        // %Y-%m-%d is width-tolerant; TS requires exact \d{4}-\d{2}-\d{2}.
        manifest.chapters[0].date = Some("2026-2-3".to_string());
        assert!(validate_manifest(&manifest).is_err());
        let mut manifest = fixture_manifest();
        manifest.chapters[0].date = Some("2026-02-30".to_string());
        assert!(validate_manifest(&manifest).is_err());

        for bad in [
            "2026-07-10",                // date only
            "2026-07-10 00:00:00Z",      // space separator
            "2026-07-10t00:00:00Z",      // lowercase t
            "2026-07-10T00:00:00z",      // lowercase z
            "2026-07-10T23:59:60Z",      // leap second
            "2026-07-10T24:00:00Z",      // hour 24
            "2026-07-10T00:00:00",       // missing offset
            "2026-07-10T00:00:00+24:00", // out-of-range offset
            "2026-07-10T00:00:00.Z",     // empty fraction
        ] {
            let mut manifest = fixture_manifest();
            manifest.generated_at = bad.to_string();
            assert!(
                validate_manifest(&manifest).is_err(),
                "datetime must be rejected: {bad}"
            );
        }
        for good in [
            "2026-07-10T00:00:00Z",
            "2026-07-10T00:00:00.000Z",
            "2026-07-10T00:00:00+08:00",
            "2026-07-10T00:00:00.123456789+08:00",
        ] {
            let mut manifest = fixture_manifest();
            manifest.generated_at = good.to_string();
            manifest.updated_at = good.to_string();
            assert!(
                validate_manifest(&manifest).is_ok(),
                "datetime must be accepted: {good}"
            );
        }
    }

    #[test]
    fn schema_version_accepts_numeric_one_only() {
        // schema `const: 1` and TS `value !== 1` both accept `1.0`; integers
        // other than 1 and non-numeric values are rejected everywhere.
        let raw = include_str!("../../../../packages/contracts/fixtures/manifest.valid.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
        value["schemaVersion"] = serde_json::Value::from(1.0_f64);
        assert_eq!(
            serde_json::from_value::<Manifest>(value.clone())
                .expect("1.0 must deserialize")
                .schema_version,
            1
        );
        value["schemaVersion"] = serde_json::Value::from(1.5_f64);
        assert!(serde_json::from_value::<Manifest>(value.clone()).is_err());
        value["schemaVersion"] = serde_json::Value::from("1");
        assert!(serde_json::from_value::<Manifest>(value).is_err());
    }

    #[test]
    fn accepts_shared_publication_fixture() {
        let publication: crate::epub::Publication = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/publication.valid.json"
        ))
        .expect("fixture must deserialize");
        assert!(validate_publication(&publication).is_ok());
    }

    #[test]
    fn rejects_bad_publication_fields() {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/publication.valid.json"
        ))
        .expect("fixture json");
        value["format"] = serde_json::Value::from("pdf");
        let accepted = serde_json::from_value::<crate::epub::Publication>(value)
            .map(|publication| validate_publication(&publication).is_ok())
            .unwrap_or(false);
        assert!(!accepted);

        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/publication.valid.json"
        ))
        .expect("fixture json");
        // A nav entry pointing outside the spine — a cross-field rule JSON
        // Schema cannot express; the validator must catch it.
        value["nav"][0]["chapterId"] = serde_json::Value::from("ghost-chapter");
        let publication: crate::epub::Publication =
            serde_json::from_value(value).expect("shape still deserializes");
        assert!(validate_publication(&publication).is_err());
    }

    #[test]
    fn accepts_shared_reader_locator_fixture() {
        let locator: ReaderLocator = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reader-locator.valid.json"
        ))
        .expect("fixture must deserialize");
        assert!(validate_reader_locator(&locator).is_ok());
    }

    #[test]
    fn rejects_bad_reader_locator_fields() {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reader-locator.valid.json"
        ))
        .expect("fixture json");
        value["ratio"] = serde_json::Value::from(1.5_f64);
        let locator: ReaderLocator =
            serde_json::from_value(value).expect("shape still deserializes");
        assert!(validate_reader_locator(&locator).is_err());

        // Unknown anchor kind fails at deserialize time, before validation.
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reader-locator.valid.json"
        ))
        .expect("fixture json");
        value["anchor"] = serde_json::json!({ "kind": "bogus" });
        assert!(serde_json::from_value::<ReaderLocator>(value).is_err());
    }

    #[derive(serde::Deserialize)]
    struct FixtureExpectation {
        fixture: String,
        contract: String,
        expect: String,
    }

    /// P1-22⑤: Rust and the TS `node --test` suite consume the same
    /// expectations.json over the same fixture set, so an accept/reject verdict
    /// can never drift between the two implementations.
    #[test]
    fn shared_fixtures_match_schema_and_ts_verdicts() {
        let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../packages/contracts/fixtures");
        let raw = std::fs::read_to_string(fixtures_dir.join("expectations.json"))
            .expect("fixtures/expectations.json must exist");
        let expectations: Vec<FixtureExpectation> =
            serde_json::from_str(&raw).expect("expectations.json must parse");
        assert!(expectations.len() >= 10, "parity suite must cover fixtures");
        let manifest = fixture_manifest();
        for entry in expectations {
            let text = std::fs::read_to_string(fixtures_dir.join(&entry.fixture))
                .unwrap_or_else(|error| panic!("{}: {error}", entry.fixture));
            let accepted = match entry.contract.as_str() {
                "manifest" => serde_json::from_str::<Manifest>(&text)
                    .map(|value| validate_manifest(&value).is_ok())
                    .unwrap_or(false),
                "reading" => serde_json::from_str::<ReadingProgress>(&text)
                    .map(|value| validate_reading(&value, &manifest).is_ok())
                    .unwrap_or(false),
                // The Rust readers for these journals are intentionally more
                // tolerant than the strict schemas (all-Option fields, no
                // deny_unknown_fields) so old files still load — the table
                // only lists fixtures where every leg's verdict agrees.
                "provenance" => {
                    serde_json::from_str::<crate::library::BookProvenance>(&text).is_ok()
                }
                "publish-transaction" => {
                    serde_json::from_str::<crate::publish::PublishTransaction>(&text).is_ok()
                }
                // 01-F5: the acquisition wire contract — full struct
                // deserialize, exactly what the emitted TaskEvent is.
                "task-event" => serde_json::from_str::<crate::tasks::TaskEvent>(&text).is_ok(),
                // EPUB sidecar + locator legs: full strict deserialize
                // (deny_unknown_fields) plus the semantic validators — the
                // same two gates the import/save paths run.
                "publication" => serde_json::from_str::<crate::epub::Publication>(&text)
                    .map(|value| validate_publication(&value).is_ok())
                    .unwrap_or(false),
                "reader-locator" => serde_json::from_str::<ReaderLocator>(&text)
                    .map(|value| validate_reader_locator(&value).is_ok())
                    .unwrap_or(false),
                // 01-F5: the worker's fatal NDJSON record. The host reads
                // fields piecemeal, so the leg mirrors that: type=="fatal",
                // errorCode inside the TaskErrorCode domain, message a
                // string, and requiredAction — when present — inside the
                // RequiredAction domain.
                "worker-fatal" => serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .is_some_and(|value| {
                        value.get("type").and_then(|item| item.as_str()) == Some("fatal")
                            && value
                                .get("errorCode")
                                .cloned()
                                .and_then(|item| {
                                    serde_json::from_value::<crate::tasks::TaskErrorCode>(item).ok()
                                })
                                .is_some()
                            && value
                                .get("message")
                                .and_then(|item| item.as_str())
                                .is_some()
                            && value.get("requiredAction").is_none_or(|item| {
                                serde_json::from_value::<crate::tasks::RequiredAction>(item.clone())
                                    .is_ok()
                            })
                    }),
                other => panic!("{}: unknown contract {other}", entry.fixture),
            };
            assert_eq!(
                accepted,
                entry.expect == "valid",
                "fixture {}",
                entry.fixture
            );
        }
    }
}
