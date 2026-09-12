use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The shared schema (`const: 1`) and the TS validator (`value !== 1`) accept
/// any numeric 1 — including the float form `1.0` — while serde's `u32`
/// rejects floats. Deserialize the same set here so all three implementations
/// agree on `schemaVersion`.
fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value.as_f64() {
        Some(1.0) => Ok(1),
        _ => Err(serde::de::Error::custom("unsupported schema version")),
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
    // write always validates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    // `voteCount`/`wordCount` are `required` in manifest.schema.json — do not
    // silently default what the other implementations treat as an error.
    pub vote_count: u64,
    pub word_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
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
    if !is_rfc3339_date_time(&manifest.generated_at) || !is_rfc3339_date_time(&manifest.updated_at)
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

/// Mirrors `requireRelativePath` in packages/contracts/src/index.ts and the
/// `path` pattern in manifest.schema.json: forward-slash relative paths only —
/// non-blank, no leading `/`, no drive prefix, no `\`, no NUL, and no empty /
/// `.` / `..` segments. Keep all implementations in lockstep.
pub fn is_safe_relative_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive_prefixed = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    !value.trim().is_empty()
        && !value.starts_with('/')
        && !drive_prefixed
        && !value.contains('\\')
        && !value.contains('\0')
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
        ] {
            assert!(
                !super::is_safe_relative_path(path),
                "path must be rejected: {path:?}"
            );
        }
        for path in ["001.md", "sub/002.md", ".hidden/001.md", "第一篇 .md"] {
            assert!(
                super::is_safe_relative_path(path),
                "path must be accepted: {path:?}"
            );
        }
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
            "2026-07-10",               // date only
            "2026-07-10 00:00:00Z",     // space separator
            "2026-07-10t00:00:00Z",     // lowercase t
            "2026-07-10T00:00:00z",     // lowercase z
            "2026-07-10T23:59:60Z",     // leap second
            "2026-07-10T24:00:00Z",     // hour 24
            "2026-07-10T00:00:00",      // missing offset
            "2026-07-10T00:00:00+24:00", // out-of-range offset
            "2026-07-10T00:00:00.Z",    // empty fraction
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
