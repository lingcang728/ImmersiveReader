//! EPUB 2/3 support: import, parse, sanitize, and chapter serving.
//!
//! Public API contract — consumed by `lib.rs` commands and `reader_db.rs`.
//! DO NOT change these signatures without updating the command layer:
//! another workstream wires `begin_import`/`get_readable_chapter`/`search_book`
//! against these exact types.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::ZipArchive;

use crate::contracts::{
    is_safe_path_segment, is_safe_relative_path, validate_manifest, Chapter, Manifest,
};
use crate::library::LibraryIssue;

/// One navigation entry (EPUB 3 nav.xhtml or EPUB 2 NCX).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavItem {
    pub title: String,
    /// Chapter id in `manifest.chapters` this entry opens.
    pub chapter_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NavItem>,
}

/// `publication.json` — versioned sidecar describing a non-Markdown book.
/// Books lacking this file keep being treated as Markdown.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publication {
    // Same numeric-1 rule as manifest/reading schemaVersion (1.0 is legal).
    #[serde(deserialize_with = "crate::contracts::deserialize_schema_version")]
    pub schema_version: u32,
    /// Always "epub" for this revision.
    pub format: String,
    /// "2" or "3" as declared by the OPF package version.
    pub epub_version: String,
    pub title: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::contracts::deserialize_optional_string"
    )]
    pub creator: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::contracts::deserialize_optional_string"
    )]
    pub language: Option<String>,
    /// Safe-relative path to the cover image inside the book dir, if any.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::contracts::deserialize_optional_string"
    )]
    pub cover: Option<String>,
    #[serde(default)]
    pub nav: Vec<NavItem>,
    /// Spine order as chapter ids; mirrors `manifest.chapters` order.
    #[serde(default)]
    pub spine: Vec<String>,
    /// Format-specific extras (resource map id→path etc.).
    #[serde(default)]
    pub resources: HashMap<String, String>,
    /// True when the package declares fixed-layout / scripted content we
    /// refuse to render; import still succeeds but flags unsupported bits.
    #[serde(default)]
    pub unsupported: Vec<String>,
}

impl Publication {
    pub const FORMAT_EPUB: &'static str = "epub";
    pub const SCHEMA_VERSION: u32 = 1;
}

/// Absolute or book-relative location of the publication sidecar.
pub fn publication_path(book_root: &Path) -> PathBuf {
    book_root.join("publication.json")
}

/// Load `publication.json`; `Ok(None)` means this is a Markdown book.
pub fn load_publication(book_root: &Path) -> Result<Option<Publication>, String> {
    let path = publication_path(book_root);
    let bytes = match fs::read(crate::atomic_file::long_path(&path)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("EPUB_PUBLICATION_READ: {error}")),
    };
    let publication: Publication = serde_json::from_slice(&bytes)
        .map_err(|error| format!("EPUB_PUBLICATION_PARSE: {error}"))?;
    crate::contracts::validate_publication(&publication)?;
    Ok(Some(publication))
}

/// Result of importing one `.epub` file into the library.
/// `manifest`/`issues` mirror `importer::ImportOutcome`'s wire shape.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubImportOutcome {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub issues: Vec<crate::library::LibraryIssue>,
}

// ---------------------------------------------------------------------------
// Limits and error codes (txt spec §4)
// ---------------------------------------------------------------------------

/// Compressed `.epub` on disk.
const MAX_EPUB_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
/// Sum of uncompressed entry sizes — a zip-bomb cap enforced before writing.
const MAX_EPUB_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EPUB_ENTRIES: usize = 10_000;
/// Single uncompressed entry.
const MAX_EPUB_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// Staging dir for in-flight imports, hidden from the library scan — mirrors
/// `importer::IMPORT_STAGING_DIR` (`手动/.incoming`, swept at startup).
const IMPORT_STAGING_DIR: &str = ".incoming";

fn issue(issues: &mut Vec<LibraryIssue>, path: &str, message: impl Into<String>) {
    issues.push(LibraryIssue {
        path: path.to_string(),
        message: message.into(),
    });
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Lowercased local name (namespace prefix stripped) of an element/attribute
/// name, e.g. `xhtml:body` → `body`, `xlink:href` → `href`.
fn local_lower(raw: &[u8]) -> String {
    let raw = match raw.iter().rposition(|b| *b == b':') {
        Some(index) => &raw[index + 1..],
        None => raw,
    };
    String::from_utf8_lossy(raw).to_lowercase()
}

/// Decode XML bytes to text. EPUB mandates UTF-8/UTF-16; a BOM wins, then an
/// XML declaration `encoding="…"` label, else UTF-8 with lossy fallback.
fn decode_xml_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return text.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return text.into_owned();
    }
    // Peek at the XML declaration for a charset label (`<?xml … encoding="x"`).
    let head_len = bytes.len().min(256);
    if let Ok(head) = std::str::from_utf8(&bytes[..head_len]) {
        if let Some(decl_start) = head.find("encoding") {
            let tail = &head[decl_start + "encoding".len()..];
            let tail = tail.trim_start_matches(['=', ' ', '\t', '\'', '"']);
            let label: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
                .collect();
            if !label.is_empty() {
                if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
                    let (text, _, _) = encoding.decode(bytes);
                    return text.into_owned();
                }
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Refuse documents that declare entities — quick-xml never expands external
/// entities, but an explicit `<!ENTITY` guard makes the refusal deliberate.
fn contains_entity_decl(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"<!ENTITY";
    bytes
        .windows(NEEDLE.len())
        .any(|window| window.eq_ignore_ascii_case(NEEDLE))
}

/// Resolve an OPF/nav href (`href` is URL-escaped relative to `base_dir`, a
/// `/`-separated archive directory or "") into a normalized archive path.
/// Returns `None` on escapes or undecodable input.
fn resolve_href(base_dir: &str, href: &str) -> Option<String> {
    let href = href.split(['#', '?']).next()?;
    if href.is_empty() {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8()
        .ok()?;
    let mut segments: Vec<String> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').map(|s| s.to_string()).collect()
    };
    for segment in decoded.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other.to_string()),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// An archive member name must become a safe forward-slash relative path:
/// every `/`-separated segment passes `is_safe_path_segment` (this kills
/// `..`, `\`, drive prefixes, reserved device names, trailing dots).
fn safe_archive_path(name: &str) -> Option<String> {
    if name.is_empty() || name.ends_with('/') {
        return None;
    }
    let mut segments = Vec::new();
    for segment in name.split('/') {
        if !is_safe_path_segment(segment) {
            return None;
        }
        segments.push(segment);
    }
    Some(segments.join("/"))
}

fn is_xml_like(name: &str, media_type: &str) -> bool {
    let media = media_type.to_ascii_lowercase();
    if media == "text/html"
        || media == "application/xhtml+xml"
        || media == "application/xml"
        || media == "text/xml"
        || media == "image/svg+xml"
        || media == "application/x-dtbncx+xml"
        || media == "application/oebps-package+xml"
        || media.ends_with("+xml")
    {
        return true;
    }
    matches!(
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("xhtml" | "html" | "htm" | "xml" | "svg" | "ncx" | "opf" | "smil")
    )
}

/// Non-whitespace code point count — canonical wordCount (P3-29).
fn word_count(text: &str) -> u64 {
    text.chars().filter(|c| !c.is_whitespace()).count() as u64
}

// ---------------------------------------------------------------------------
// XHTML sanitizer — streaming rewrite via quick-xml events
// ---------------------------------------------------------------------------

/// Elements removed together with their whole subtree.
const DROPPED_TAGS: &[&str] = &[
    "script",
    "iframe",
    "object",
    "embed",
    "form",
    "input",
    "button",
    "textarea",
    "select",
    "audio",
    "video",
    "base",
    "foreignobject",
    "noscript",
];

/// Elements allowed inside `<svg>`; unknown SVG-context tags are unwrapped
/// (tag dropped, children kept) — banned tags still drop the subtree.
const SVG_ALLOWED: &[&str] = &[
    "svg",
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "defs",
    "lineargradient",
    "radialgradient",
    "stop",
    "use",
    "symbol",
    "title",
    "desc",
    "image",
    "a",
];

/// Attribute names whose value is a URL. Checked by local name so
/// `xlink:href` is covered by `href`.
const URL_ATTRS: &[&str] = &[
    "href",
    "src",
    "poster",
    "data",
    "action",
    "formaction",
    "background",
    "cite",
    "longdesc",
];

/// `style=""` whitelist — exact names plus `margin-*`/`padding-*`/`border-*`/
/// `list-style-*`/`text-decoration-*`/`overflow-*` prefixes.
const STYLE_PROPS_EXACT: &[&str] = &[
    "color",
    "background-color",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "text-align",
    "text-indent",
    "line-height",
    "vertical-align",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "display",
    "float",
    "clear",
    "overflow",
    "width",
    "height",
    "max-width",
    "max-height",
];
const STYLE_PROP_PREFIXES: &[&str] = &[
    "margin",
    "padding",
    "border",
    "list-style",
    "text-decoration",
    "overflow",
];

/// Substrings that poison a CSS declaration value (checked lowercased).
const CSS_VALUE_BANNED: &[&str] = &[
    "url(",
    "expression(",
    "javascript:",
    "//",
    "@import",
    "behavior",
    "-moz-binding",
];

enum UrlClass {
    /// Relative path / fragment inside the book — kept verbatim.
    Internal,
    /// http(s) or protocol-relative — dropped; the URL is preserved in
    /// `data-ir-remote-src` for the frontend's placeholder (kept on `<a>`).
    Remote,
    /// javascript/vbscript/file/data(non-image)/scheme-relative root paths —
    /// dropped outright.
    Dangerous,
}

fn classify_url(decoded: &str) -> UrlClass {
    let trimmed = decoded.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return UrlClass::Internal;
    }
    if trimmed.starts_with("//") {
        return UrlClass::Remote;
    }
    if trimmed.starts_with('/') {
        return UrlClass::Dangerous;
    }
    // Scheme detection: `ALPHA *(ALPHA|DIGIT|"+"|"-"|".") ":"`.
    let bytes = trimmed.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b':' {
            break;
        }
        if matches!(byte, b'/' | b'#' | b'?') {
            index = 0;
            break;
        }
        let valid = (index == 0 && byte.is_ascii_alphabetic())
            || (index > 0 && (byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')));
        if !valid {
            index = 0;
            break;
        }
        index += 1;
    }
    if index == 0 || index >= bytes.len() || bytes[index] != b':' {
        return UrlClass::Internal;
    }
    let scheme = trimmed[..index].to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" => UrlClass::Remote,
        "data" if trimmed[index..].to_ascii_lowercase().starts_with(":image/") => {
            UrlClass::Internal
        }
        _ => UrlClass::Dangerous,
    }
}

/// Decode an attribute value for classification: entities resolved when
/// possible, raw bytes as fallback (lossy — classification errs safe).
fn attr_value_decoded(
    attr: &quick_xml::events::attributes::Attribute<'_>,
    decoder: quick_xml::Decoder,
) -> String {
    attr.decode_and_unescape_value(decoder)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned())
}

/// Escape a computed value into a double-quoted attribute body.
fn escape_attr_value(value: &str, out: &mut Vec<u8>) {
    for ch in value.chars() {
        match ch {
            '&' => out.extend_from_slice(b"&amp;"),
            '"' => out.extend_from_slice(b"&quot;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            _ => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

/// Emit a verbatim attribute (`name="raw"`); the raw value already survived
/// XML parsing, so only `"` (which cannot appear in a `"`-quoted source but
/// can in a `'`-quoted one) needs normalizing to `&quot;`.
fn emit_raw_attr(out: &mut Vec<u8>, name: &[u8], raw: &[u8]) {
    out.push(b' ');
    out.extend_from_slice(name);
    out.extend_from_slice(b"=\"");
    if raw.contains(&b'"') {
        for byte in raw {
            if *byte == b'"' {
                out.extend_from_slice(b"&quot;");
            } else {
                out.push(*byte);
            }
        }
    } else {
        out.extend_from_slice(raw);
    }
    out.push(b'"');
}

fn style_prop_allowed(name: &str) -> bool {
    if STYLE_PROPS_EXACT.contains(&name) {
        return true;
    }
    STYLE_PROP_PREFIXES
        .iter()
        .any(|prefix| name == *prefix || name.starts_with(&format!("{prefix}-")))
}

/// Whitelisted `style=""` properties; values carrying `url(`, `expression(`,
/// `javascript:` or `//` are dropped entirely.
fn sanitize_inline_style(value: &str) -> String {
    value
        .split(';')
        .filter_map(|decl| {
            let (name, val) = decl.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            if name.is_empty() || !style_prop_allowed(&name) {
                return None;
            }
            let lowered = val.to_ascii_lowercase();
            if CSS_VALUE_BANNED.iter().any(|bad| lowered.contains(bad)) {
                return None;
            }
            Some(format!("{name}:{}", val.trim()))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// `<style>` / CSS text sanitization: strip comments, remove `@import` rules,
/// neutralize remote `url(…)` (keep `#fragment` and relative references),
/// drop `expression(…)` and `javascript:`.
fn sanitize_css_block(css: &str) -> String {
    // Strip `/* … */` comments first so they cannot smuggle tokens.
    let mut cleaned = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        cleaned.push_str(&rest[..start]);
        rest = match rest[start + 2..].find("*/") {
            Some(end) => &rest[start + 2 + end + 2..],
            None => "",
        };
    }
    cleaned.push_str(rest);
    let css = cleaned.as_str();
    let lower = css.to_ascii_lowercase();

    let mut out = String::with_capacity(css.len());
    let mut cursor = 0_usize;
    let len = css.len();
    while cursor < len {
        // Find the next interesting token in the lowercased view.
        let tail = &lower[cursor..];
        let import_at = tail.find("@import").map(|i| cursor + i);
        let url_at = tail.find("url(").map(|i| cursor + i);
        let expr_at = tail.find("expression(").map(|i| cursor + i);
        let js_at = tail.find("javascript:").map(|i| cursor + i);
        let next = [import_at, url_at, expr_at, js_at]
            .iter()
            .flatten()
            .min()
            .copied();
        let Some(at) = next else {
            out.push_str(&css[cursor..]);
            break;
        };
        out.push_str(&css[cursor..at]);
        if Some(at) == import_at {
            // Drop the `@import` statement up to `;` or `}`.
            let rest = &css[at + "@import".len()..];
            let end = rest
                .find([';', '}'])
                .map(|i| at + "@import".len() + i)
                .unwrap_or(len);
            cursor = end.min(len);
        } else if Some(at) == url_at {
            // `url(` … matching `)` honoring quotes.
            let inner_start = at + "url(".len();
            let bytes = css.as_bytes();
            let mut i = inner_start;
            let mut quote: Option<u8> = None;
            let mut close = len;
            while i < len {
                let b = bytes[i];
                match quote {
                    Some(q) if b == q => quote = None,
                    Some(_) => {}
                    None if b == b'\'' || b == b'"' => quote = Some(b),
                    None if b == b')' => {
                        close = i;
                        break;
                    }
                    None => {}
                }
                i += 1;
            }
            if close == len && quote.is_none() && i >= len {
                // Unterminated url( — drop the rest.
                cursor = len;
                continue;
            }
            let inner = css[inner_start..close]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"')
                .trim()
                .to_string();
            let keep = inner.starts_with('#') || matches!(classify_url(&inner), UrlClass::Internal);
            out.push_str(if keep {
                &css[at..=close.min(len - 1)]
            } else {
                "url(\"\")"
            });
            cursor = close + 1;
        } else if Some(at) == expr_at {
            // `expression(` … matching `)` — drop the whole call.
            let bytes = css.as_bytes();
            let mut depth = 0_i32;
            let mut i = at;
            while i < len {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            cursor = (i + 1).min(len);
        } else {
            // `javascript:` — remove the scheme token only.
            cursor = at + "javascript:".len();
        }
    }
    out
}

/// Sanitize a `srcset` attribute: keep relative candidates, drop remote ones;
/// `None` when nothing survives.
fn sanitize_srcset(decoded: &str) -> Option<String> {
    let kept: Vec<String> = decoded
        .split(',')
        .filter_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return None;
            }
            let url = candidate.split_whitespace().next().unwrap_or("");
            match classify_url(url) {
                UrlClass::Internal => Some(candidate.to_string()),
                _ => None,
            }
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(", "))
    }
}

enum TagDecision {
    Emit,
    Drop,   // element and its subtree
    Unwrap, // drop the tag, keep children (SVG-context only)
}

fn is_remote_stylesheet_link(e: &BytesStart<'_>, decoder: quick_xml::Decoder) -> bool {
    let mut is_stylesheet = false;
    let mut remote = false;
    for attr in e.attributes().flatten() {
        let name = local_lower(attr.key.as_ref());
        if name == "rel" {
            is_stylesheet = attr_value_decoded(&attr, decoder)
                .split_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"));
        } else if name == "href" {
            remote = matches!(
                classify_url(&attr_value_decoded(&attr, decoder)),
                UrlClass::Remote | UrlClass::Dangerous
            );
        }
    }
    is_stylesheet && remote
}

fn classify_tag(
    local: &str,
    in_svg: bool,
    e: &BytesStart<'_>,
    decoder: quick_xml::Decoder,
) -> TagDecision {
    if DROPPED_TAGS.contains(&local) {
        return TagDecision::Drop;
    }
    if local == "meta" {
        let http_equiv = e
            .attributes()
            .flatten()
            .any(|attr| local_lower(attr.key.as_ref()) == "http-equiv");
        return if http_equiv {
            TagDecision::Drop
        } else {
            TagDecision::Emit
        };
    }
    if local == "link" && is_remote_stylesheet_link(e, decoder) {
        return TagDecision::Drop;
    }
    if in_svg && !SVG_ALLOWED.contains(&local) {
        return TagDecision::Unwrap;
    }
    TagDecision::Emit
}

/// Write `<name attrs>` / `<name attrs/>` with the sanitized attribute set.
/// Returns `Ok(true)` when the tag emitted a `data-ir-remote-src` marker.
fn emit_tag(
    out: &mut Vec<u8>,
    e: &BytesStart<'_>,
    tag_local: &str,
    decoder: quick_xml::Decoder,
    empty: bool,
) -> Result<(), String> {
    out.push(b'<');
    out.extend_from_slice(e.name().as_ref());
    let mut remote_marker: Option<Vec<u8>> = None;
    for attr in e.attributes() {
        let attr = attr.map_err(|error| format!("attribute parse: {error}"))?;
        let name_raw = attr.key.as_ref();
        let name = local_lower(name_raw);
        let raw: &[u8] = &attr.value;
        // Event handlers: any `on*` attribute.
        if name.starts_with("on") {
            continue;
        }
        if name == "xml:base" {
            continue;
        }
        if name == "style" {
            let decoded = attr_value_decoded(&attr, decoder);
            let cleaned = sanitize_inline_style(&decoded);
            if !cleaned.is_empty() {
                out.extend_from_slice(b" ");
                out.extend_from_slice(name_raw);
                out.extend_from_slice(b"=\"");
                escape_attr_value(&cleaned, out);
                out.push(b'"');
            }
            continue;
        }
        if name == "srcset" {
            let decoded = attr_value_decoded(&attr, decoder);
            if let Some(cleaned) = sanitize_srcset(&decoded) {
                out.extend_from_slice(b" ");
                out.extend_from_slice(name_raw);
                out.extend_from_slice(b"=\"");
                escape_attr_value(&cleaned, out);
                out.push(b'"');
            }
            continue;
        }
        if URL_ATTRS.contains(&name.as_str()) {
            let decoded = attr_value_decoded(&attr, decoder);
            match classify_url(&decoded) {
                UrlClass::Internal => emit_raw_attr(out, name_raw, raw),
                UrlClass::Remote => {
                    // External links stay explicit on anchors; everywhere
                    // else the URL is preserved for the frontend placeholder.
                    if (tag_local == "a" || tag_local == "area") && name == "href" {
                        emit_raw_attr(out, name_raw, raw);
                    } else if remote_marker.is_none() {
                        remote_marker = Some(raw.to_vec());
                    }
                }
                UrlClass::Dangerous => {}
            }
            continue;
        }
        emit_raw_attr(out, name_raw, raw);
    }
    if let Some(remote) = remote_marker {
        out.extend_from_slice(b" data-ir-remote-src=\"");
        if remote.contains(&b'"') {
            for byte in &remote {
                if *byte == b'"' {
                    out.extend_from_slice(b"&quot;");
                } else {
                    out.push(*byte);
                }
            }
        } else {
            out.extend_from_slice(&remote);
        }
        out.push(b'"');
    }
    out.extend_from_slice(if empty { b"/>" } else { b">" });
    Ok(())
}

enum FrameKind {
    Emitted,
    Dropped,
    Unwrapped,
}

struct Frame {
    kind: FrameKind,
    is_style: bool,
    is_svg: bool,
}

/// Streaming XHTML/XML sanitizer. Output keeps the source's tag/attribute
/// spelling (entities preserved); rejected markup is dropped or rewritten.
/// Errors on malformed XML — callers decide whether that aborts the import.
fn sanitize_xml(input: &[u8]) -> Result<Vec<u8>, String> {
    if contains_entity_decl(input) {
        return Err("document declares <!ENTITY> — refused".to_string());
    }
    let mut reader = Reader::from_reader(input);
    let decoder = reader.decoder();
    let mut out: Vec<u8> = Vec::with_capacity(input.len() + input.len() / 8);
    let mut stack: Vec<Frame> = Vec::new();
    let mut dropped_depth = 0_usize;
    let mut style_depth = 0_usize;
    let mut svg_depth = 0_usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("xml parse: {error}"))?;
        match event {
            Event::Start(e) => {
                let local = local_lower(e.name().as_ref());
                let decision = if dropped_depth > 0 {
                    TagDecision::Drop
                } else {
                    classify_tag(&local, svg_depth > 0, &e, decoder)
                };
                match decision {
                    TagDecision::Emit => {
                        emit_tag(&mut out, &e, &local, decoder, false)?;
                        let is_style = local == "style";
                        let is_svg = local == "svg";
                        if is_style {
                            style_depth += 1;
                        }
                        if is_svg {
                            svg_depth += 1;
                        }
                        stack.push(Frame {
                            kind: FrameKind::Emitted,
                            is_style,
                            is_svg,
                        });
                    }
                    TagDecision::Drop => {
                        dropped_depth += 1;
                        stack.push(Frame {
                            kind: FrameKind::Dropped,
                            is_style: false,
                            is_svg: false,
                        });
                    }
                    TagDecision::Unwrap => stack.push(Frame {
                        kind: FrameKind::Unwrapped,
                        is_style: false,
                        is_svg: false,
                    }),
                }
            }
            Event::Empty(e) => {
                let local = local_lower(e.name().as_ref());
                let decision = if dropped_depth > 0 {
                    TagDecision::Drop
                } else {
                    classify_tag(&local, svg_depth > 0, &e, decoder)
                };
                if matches!(decision, TagDecision::Emit) {
                    emit_tag(&mut out, &e, &local, decoder, true)?;
                }
            }
            Event::End(e) => {
                if let Some(frame) = stack.pop() {
                    if matches!(frame.kind, FrameKind::Emitted) {
                        out.extend_from_slice(b"</");
                        out.extend_from_slice(e.name().as_ref());
                        out.push(b'>');
                    }
                    if matches!(frame.kind, FrameKind::Dropped) {
                        dropped_depth = dropped_depth.saturating_sub(1);
                    }
                    if frame.is_style {
                        style_depth = style_depth.saturating_sub(1);
                    }
                    if frame.is_svg {
                        svg_depth = svg_depth.saturating_sub(1);
                    }
                }
            }
            Event::Text(e) => {
                if dropped_depth > 0 {
                    continue;
                }
                if style_depth > 0 {
                    let decoded = e.decode().map_err(|error| error.to_string())?;
                    out.extend_from_slice(sanitize_css_block(&decoded).as_bytes());
                } else {
                    out.extend_from_slice(&e);
                }
            }
            Event::CData(e) => {
                if dropped_depth > 0 {
                    continue;
                }
                if style_depth > 0 {
                    let decoded = e.decode().map_err(|error| error.to_string())?;
                    out.extend_from_slice(sanitize_css_block(&decoded).as_bytes());
                } else {
                    out.extend_from_slice(b"<![CDATA[");
                    out.extend_from_slice(&e.into_inner()[..]);
                    out.extend_from_slice(b"]]>");
                }
            }
            Event::GeneralRef(e) => {
                // `&name;` references are emitted verbatim — the source was
                // well-formed XML so the name is a legal reference.
                if dropped_depth == 0 {
                    out.push(b'&');
                    out.extend_from_slice(&e.into_inner()[..]);
                    out.push(b';');
                }
            }
            Event::Comment(e) => {
                if dropped_depth == 0 {
                    out.extend_from_slice(b"<!--");
                    out.extend_from_slice(&e);
                    out.extend_from_slice(b"-->");
                }
            }
            // The XML declaration is dropped on purpose: the emitted document
            // is always UTF-8, while the source may have declared another
            // encoding — re-emitting it would mislabel the output. Processing
            // instructions and DOCTYPE are dropped as well.
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    Ok(out)
}

/// Strip markup to plain text for the FTS index: text events + HTML entity
/// references resolved, whitespace collapsed.
fn xhtml_to_text(input: &[u8]) -> Result<String, String> {
    const BLOCK_TAGS: &[&str] = &[
        "address",
        "article",
        "aside",
        "blockquote",
        "br",
        "dd",
        "div",
        "dl",
        "dt",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hr",
        "li",
        "main",
        "nav",
        "ol",
        "p",
        "pre",
        "section",
        "table",
        "td",
        "th",
        "tr",
        "ul",
    ];
    let mut reader = Reader::from_reader(input);
    let mut text = String::with_capacity(input.len() / 2);
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(e) | Event::Empty(e) => {
                if BLOCK_TAGS.contains(&local_lower(e.name().as_ref()).as_str()) {
                    text.push(' ');
                }
            }
            Event::End(e) => {
                if BLOCK_TAGS.contains(&local_lower(e.name().as_ref()).as_str()) {
                    text.push(' ');
                }
            }
            Event::Text(e) => {
                text.push_str(&e.decode().map_err(|error| error.to_string())?);
            }
            Event::GeneralRef(e) => {
                if let Ok(content) = e.html_content() {
                    text.push_str(&content);
                }
            }
            Event::CData(e) => {
                if let Ok(content) = e.decode() {
                    text.push_str(&content);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut collapsed = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !collapsed.is_empty();
        } else {
            if pending_space {
                collapsed.push(' ');
                pending_space = false;
            }
            collapsed.push(ch);
        }
    }
    Ok(collapsed)
}

// ---------------------------------------------------------------------------
// OPF / container / navigation parsing
// ---------------------------------------------------------------------------

struct OpfItem {
    id: String,
    href: String,
    media_type: String,
    properties: String,
    has_media_overlay: bool,
}

#[derive(Default)]
struct OpfData {
    version: String,
    title: Option<String>,
    creator: Option<String>,
    language: Option<String>,
    /// All `dc:identifier` values as (id-attr, text).
    identifiers: Vec<(Option<String>, String)>,
    unique_identifier: Option<String>,
    items: Vec<OpfItem>,
    spine: Vec<String>,
    spine_toc: Option<String>,
    cover_item_id: Option<String>,
    fixed_layout: bool,
    media_overlay: bool,
}

impl OpfData {
    fn identifier(&self) -> Option<&str> {
        if let Some(unique) = &self.unique_identifier {
            if let Some((_, text)) = self
                .identifiers
                .iter()
                .find(|(id, _)| id.as_deref() == Some(unique.as_str()))
            {
                return Some(text.as_str());
            }
        }
        self.identifiers
            .first()
            .map(|(_, text)| text.as_str())
            .filter(|text| !text.trim().is_empty())
    }

    fn item(&self, id: &str) -> Option<&OpfItem> {
        self.items.iter().find(|item| item.id == id)
    }

    fn nav_item(&self) -> Option<&OpfItem> {
        self.items
            .iter()
            .find(|item| item.properties.split_whitespace().any(|prop| prop == "nav"))
    }

    fn ncx_item(&self) -> Option<&OpfItem> {
        if let Some(toc) = &self.spine_toc {
            if let Some(item) = self.item(toc) {
                return Some(item);
            }
        }
        self.items.iter().find(|item| {
            item.media_type
                .eq_ignore_ascii_case("application/x-dtbncx+xml")
        })
    }

    fn cover_item(&self) -> Option<&OpfItem> {
        if let Some(id) = &self.cover_item_id {
            if let Some(item) = self.item(id) {
                return Some(item);
            }
        }
        self.items.iter().find(|item| {
            item.properties
                .split_whitespace()
                .any(|prop| prop == "cover-image")
        })
    }

    fn scripted(&self) -> bool {
        self.items.iter().any(|item| {
            item.properties
                .split_whitespace()
                .any(|prop| prop == "scripted")
        })
    }
}

fn attr_str(e: &BytesStart<'_>, name: &str, decoder: quick_xml::Decoder) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            return Some(attr_value_decoded(&attr, decoder));
        }
    }
    None
}

/// `META-INF/container.xml` → rootfile full-path.
fn parse_container(bytes: &[u8]) -> Result<String, String> {
    if contains_entity_decl(bytes) {
        return Err("EPUB_MALFORMED_CONTAINER: entity declarations refused".to_string());
    }
    let mut reader = Reader::from_reader(bytes);
    let decoder = reader.decoder();
    loop {
        match reader
            .read_event()
            .map_err(|error| format!("EPUB_MALFORMED_CONTAINER: {error}"))?
        {
            Event::Start(e) | Event::Empty(e) => {
                if local_lower(e.name().as_ref()) == "rootfile" {
                    if let Some(path) = attr_str(&e, "full-path", decoder) {
                        let path = path.trim().to_string();
                        if !path.is_empty() {
                            return Ok(path);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Err("EPUB_MALFORMED_CONTAINER: no rootfile".to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum OpfCap {
    Title,
    Creator,
    Language,
    Identifier,
    MetaLayout,
}

/// One element-open during OPF parse. `stack` holds the already-open
/// ancestors (not yet including `e`); `capture` is the in-flight text
/// capture at a fixed stack depth; `pending_identifier_id` carries the
/// `id` attribute of the `<dc:identifier>` currently being captured.
fn opf_on_element(
    e: &BytesStart<'_>,
    empty: bool,
    stack: &mut Vec<String>,
    data: &mut OpfData,
    capture: &mut Option<(OpfCap, usize, String)>,
    pending_identifier_id: &mut Option<String>,
    decoder: quick_xml::Decoder,
) {
    let local = local_lower(e.name().as_ref());
    let parent = stack.last().map(String::as_str);
    let in_metadata =
        parent == Some("metadata") || stack.iter().rev().nth(1).is_some_and(|n| n == "metadata");
    match local.as_str() {
        "package" if stack.is_empty() => {
            data.version = attr_str(e, "version", decoder).unwrap_or_default();
            data.unique_identifier = attr_str(e, "unique-identifier", decoder);
        }
        "item" if parent == Some("manifest") => {
            let media_overlay_attr = e
                .attributes()
                .flatten()
                .any(|attr| local_lower(attr.key.as_ref()) == "media-overlay");
            if media_overlay_attr {
                data.media_overlay = true;
            }
            let properties = attr_str(e, "properties", decoder).unwrap_or_default();
            if properties
                .split_whitespace()
                .any(|p| p == "rendition:layout-pre-paginated")
            {
                data.fixed_layout = true;
            }
            data.items.push(OpfItem {
                id: attr_str(e, "id", decoder).unwrap_or_default(),
                href: attr_str(e, "href", decoder).unwrap_or_default(),
                media_type: attr_str(e, "media-type", decoder).unwrap_or_default(),
                properties,
                has_media_overlay: media_overlay_attr,
            });
        }
        "itemref" if parent == Some("spine") => {
            if let Some(idref) = attr_str(e, "idref", decoder) {
                let idref = idref.trim().to_string();
                if !idref.is_empty() {
                    data.spine.push(idref);
                }
            }
        }
        "spine" => {
            if let Some(toc) = attr_str(e, "toc", decoder) {
                let toc = toc.trim().to_string();
                if !toc.is_empty() {
                    data.spine_toc = Some(toc);
                }
            }
        }
        "meta" if in_metadata => {
            let name = attr_str(e, "name", decoder).unwrap_or_default();
            let property = attr_str(e, "property", decoder).unwrap_or_default();
            if name == "cover" {
                if let Some(content) = attr_str(e, "content", decoder) {
                    let content = content.trim().to_string();
                    if !content.is_empty() {
                        data.cover_item_id = Some(content);
                    }
                }
            }
            if name.to_ascii_lowercase().contains("media-overlay") || property.starts_with("media:")
            {
                data.media_overlay = true;
            }
            if property == "rendition:layout" && !empty && capture.is_none() {
                *capture = Some((OpfCap::MetaLayout, stack.len() + 1, String::new()));
            }
        }
        "title" if in_metadata && capture.is_none() && !empty => {
            *capture = Some((OpfCap::Title, stack.len() + 1, String::new()));
        }
        "creator" if in_metadata && capture.is_none() && !empty => {
            *capture = Some((OpfCap::Creator, stack.len() + 1, String::new()));
        }
        "language" if in_metadata && capture.is_none() && !empty => {
            *capture = Some((OpfCap::Language, stack.len() + 1, String::new()));
        }
        "identifier" if in_metadata && !empty => {
            *capture = Some((OpfCap::Identifier, stack.len() + 1, String::new()));
            *pending_identifier_id = attr_str(e, "id", decoder);
        }
        _ => {}
    }
    if !empty {
        stack.push(local);
    }
}

/// OPF package document parse — metadata, manifest, spine, feature flags.
fn parse_opf(bytes: &[u8]) -> Result<OpfData, String> {
    if contains_entity_decl(bytes) {
        return Err("EPUB_MALFORMED_OPF: entity declarations refused".to_string());
    }
    let mut data = OpfData::default();
    let mut reader = Reader::from_reader(bytes);
    let decoder = reader.decoder();
    let mut stack: Vec<String> = Vec::new();
    let mut capture: Option<(OpfCap, usize, String)> = None;
    let mut pending_identifier_id: Option<String> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("EPUB_MALFORMED_OPF: {error}"))?;
        match event {
            Event::Start(e) => opf_on_element(
                &e,
                false,
                &mut stack,
                &mut data,
                &mut capture,
                &mut pending_identifier_id,
                decoder,
            ),
            Event::Empty(e) => opf_on_element(
                &e,
                true,
                &mut stack,
                &mut data,
                &mut capture,
                &mut pending_identifier_id,
                decoder,
            ),
            Event::Text(e) => {
                if let Some((_, _, buffer)) = capture.as_mut() {
                    if let Ok(text) = e.decode() {
                        buffer.push_str(&text);
                    }
                }
            }
            Event::GeneralRef(e) => {
                if let Some((_, _, buffer)) = capture.as_mut() {
                    if let Ok(text) = e.html_content() {
                        buffer.push_str(&text);
                    }
                }
            }
            Event::End(_) => {
                if stack.len()
                    == capture
                        .as_ref()
                        .map(|(_, depth, _)| *depth)
                        .unwrap_or(usize::MAX)
                {
                    if let Some((kind, _, buffer)) = capture.take() {
                        let text = buffer.trim().to_string();
                        match kind {
                            OpfCap::Title if !text.is_empty() && data.title.is_none() => {
                                data.title = Some(text)
                            }
                            OpfCap::Creator if !text.is_empty() && data.creator.is_none() => {
                                data.creator = Some(text)
                            }
                            OpfCap::Language if !text.is_empty() && data.language.is_none() => {
                                data.language = Some(text)
                            }
                            OpfCap::Identifier => {
                                if !text.is_empty() {
                                    data.identifiers.push((pending_identifier_id.take(), text));
                                } else {
                                    pending_identifier_id = None;
                                }
                            }
                            OpfCap::MetaLayout if text.eq_ignore_ascii_case("pre-paginated") => {
                                data.fixed_layout = true;
                            }
                            _ => {}
                        }
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if data.items.is_empty() {
        return Err("EPUB_MALFORMED_OPF: manifest has no items".to_string());
    }
    if data.spine.is_empty() {
        return Err("EPUB_MALFORMED_SPINE: spine is empty".to_string());
    }
    Ok(data)
}

// ---------------------------------------------------------------------------
// Navigation trees (EPUB 3 nav.xhtml, EPUB 2 NCX)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct NavTmp {
    title: String,
    href: Option<String>,
    label_seen: bool,
    children: Vec<NavTmp>,
}

/// EPUB 3 `nav epub:type="toc"` → label/href tree.
fn parse_nav_xhtml(bytes: &[u8]) -> Vec<NavTmp> {
    if contains_entity_decl(bytes) {
        return Vec::new();
    }
    enum Mark {
        Nav,
        Li,
        Label, // <a>/<span> capturing into the current li's title
        Other,
    }
    let mut reader = Reader::from_reader(bytes);
    let decoder = reader.decoder();
    let mut marks: Vec<Mark> = Vec::new();
    let mut in_nav = false;
    let mut lis: Vec<NavTmp> = Vec::new();
    let mut roots: Vec<NavTmp> = Vec::new();
    let mut label_depth: Option<usize> = None; // marks.len() when label opened
    while let Ok(event) = reader.read_event() {
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let local = local_lower(e.name().as_ref());
                if !in_nav {
                    if local == "nav" {
                        let is_toc = e.attributes().flatten().any(|attr| {
                            let name = local_lower(attr.key.as_ref());
                            if name != "type" {
                                return false;
                            }
                            attr_value_decoded(&attr, decoder)
                                .split_whitespace()
                                .any(|token| token.eq_ignore_ascii_case("toc"))
                        });
                        if is_toc {
                            in_nav = true;
                            marks.push(Mark::Nav);
                        }
                    }
                    continue;
                }
                match local.as_str() {
                    "li" => {
                        lis.push(NavTmp::default());
                        if !is_empty {
                            marks.push(Mark::Li);
                        } else {
                            let tmp = lis.pop().expect("just pushed");
                            match lis.last_mut() {
                                Some(parent) => parent.children.push(tmp),
                                None => roots.push(tmp),
                            }
                        }
                    }
                    "a" | "span" => {
                        if let Some(top) = lis.last_mut() {
                            if !top.label_seen {
                                top.label_seen = true;
                                if local == "a" {
                                    top.href = attr_str(&e, "href", decoder);
                                }
                                if !is_empty {
                                    marks.push(Mark::Label);
                                    label_depth = Some(marks.len());
                                    continue;
                                }
                            }
                        }
                        if !is_empty {
                            marks.push(Mark::Other);
                        }
                    }
                    _ => {
                        if !is_empty {
                            marks.push(Mark::Other);
                        }
                    }
                }
            }
            Event::End(_) => {
                if !in_nav {
                    continue;
                }
                match marks.pop() {
                    Some(Mark::Nav) => in_nav = false,
                    Some(Mark::Li) => {
                        if let Some(tmp) = lis.pop() {
                            match lis.last_mut() {
                                Some(parent) => parent.children.push(tmp),
                                None => roots.push(tmp),
                            }
                        }
                    }
                    Some(Mark::Label) => label_depth = None,
                    _ => {}
                }
            }
            Event::Text(e) => {
                if label_depth.is_some() {
                    if let (Some(top), Ok(text)) = (lis.last_mut(), e.decode()) {
                        top.title.push_str(&text);
                    }
                }
            }
            Event::GeneralRef(e) => {
                if label_depth.is_some() {
                    if let (Some(top), Ok(text)) = (lis.last_mut(), e.html_content()) {
                        top.title.push_str(&text);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    roots
}

/// EPUB 2 NCX `navMap/navPoint` → label/src tree.
fn parse_ncx(bytes: &[u8]) -> Vec<NavTmp> {
    if contains_entity_decl(bytes) {
        return Vec::new();
    }
    enum Mark {
        NavPoint,
        Text,
        Other,
    }
    let mut reader = Reader::from_reader(bytes);
    let decoder = reader.decoder();
    let mut marks: Vec<Mark> = Vec::new();
    let mut points: Vec<NavTmp> = Vec::new();
    let mut roots: Vec<NavTmp> = Vec::new();
    while let Ok(event) = reader.read_event() {
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let local = local_lower(e.name().as_ref());
                match local.as_str() {
                    "navpoint" => {
                        points.push(NavTmp::default());
                        if !is_empty {
                            marks.push(Mark::NavPoint);
                        } else {
                            let tmp = points.pop().expect("just pushed");
                            match points.last_mut() {
                                Some(parent) => parent.children.push(tmp),
                                None => roots.push(tmp),
                            }
                        }
                    }
                    "content" => {
                        if let Some(top) = points.last_mut() {
                            if top.href.is_none() {
                                top.href = attr_str(&e, "src", decoder);
                            }
                        }
                        if !is_empty {
                            marks.push(Mark::Other);
                        }
                    }
                    "text" => {
                        if !is_empty {
                            marks.push(Mark::Text);
                        }
                    }
                    _ => {
                        if !is_empty {
                            marks.push(Mark::Other);
                        }
                    }
                }
            }
            Event::End(_) => {
                if let Some(Mark::NavPoint) = marks.pop() {
                    if let Some(tmp) = points.pop() {
                        match points.last_mut() {
                            Some(parent) => parent.children.push(tmp),
                            None => roots.push(tmp),
                        }
                    }
                }
            }
            Event::Text(e) => {
                if matches!(marks.last(), Some(Mark::Text)) {
                    if let (Some(top), Ok(text)) = (points.last_mut(), e.decode()) {
                        if top.title.is_empty() {
                            top.title.push_str(&text);
                        }
                    }
                }
            }
            Event::GeneralRef(e) => {
                if matches!(marks.last(), Some(Mark::Text)) {
                    if let (Some(top), Ok(text)) = (points.last_mut(), e.html_content()) {
                        if top.title.is_empty() {
                            top.title.push_str(&text);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    roots
}

/// Convert the parsed tree into `NavItem`s; entries whose href cannot be
/// resolved to a spine chapter are removed (children are promoted).
fn build_nav(temps: Vec<NavTmp>, resolve: &dyn Fn(&str) -> Option<String>) -> Vec<NavItem> {
    temps
        .into_iter()
        .filter_map(|tmp| {
            let children = build_nav(tmp.children, resolve);
            let chapter_id = tmp.href.as_deref().and_then(resolve).unwrap_or_default();
            let title = {
                let title = tmp.title.trim().to_string();
                if title.is_empty() {
                    tmp.href
                        .as_deref()
                        .and_then(|href| Path::new(href).file_stem()?.to_str().map(str::to_string))
                        .unwrap_or_default()
                } else {
                    title
                }
            };
            if chapter_id.is_empty() && children.is_empty() {
                return None;
            }
            if chapter_id.is_empty() {
                // Unresolvable grouping node — promote children.
                return Some(NavItem {
                    title,
                    chapter_id: String::new(),
                    children,
                });
            }
            Some(NavItem {
                title,
                chapter_id,
                children,
            })
        })
        .collect()
}

/// Flatten nav promotion: `validate_publication` requires non-empty
/// chapterIds, so unresolvable grouping nodes fold their children into the
/// parent's list after `build_nav`.
fn promote_unresolved_nav(items: Vec<NavItem>) -> Vec<NavItem> {
    let mut out = Vec::new();
    for mut item in items {
        item.children = promote_unresolved_nav(item.children);
        if item.chapter_id.is_empty() {
            out.extend(item.children);
        } else {
            out.push(item);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Book-dir dedup — mirrors `importer::unique_target` (`title`, `title (2)`…).
fn unique_book_dir(manual_root: &Path, title: &str) -> PathBuf {
    let mut base = crate::contracts::sanitize_shelf_name(title, "未命名书目");
    if crate::contracts::is_reserved_device_name(&base) {
        base.push('_');
    }
    let direct = manual_root.join(&base);
    if !crate::atomic_file::long_path(&direct).exists() {
        return direct;
    }
    for suffix in 2..10_000 {
        let candidate = manual_root.join(format!("{base} ({suffix})"));
        if !crate::atomic_file::long_path(&candidate).exists() {
            return candidate;
        }
    }
    manual_root.join(format!("{base}-{}", Uuid::new_v4()))
}

/// Read one archive member to memory. XML-family entries (XHTML/OPF/NCX/SVG
/// — detected by media type and extension) are decoded and re-sanitized on
/// the way out; CSS is run through the CSS sanitizer; everything else is
/// extracted verbatim. The entry cap is re-enforced while reading so a
/// lied-about uncompressed size cannot balloon memory.
fn read_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    media_type: &str,
) -> Result<Vec<u8>, String> {
    let entry = archive
        .by_name(name)
        .map_err(|error| format!("open: {error}"))?;
    let mut bytes = Vec::with_capacity(entry.size().min(MAX_EPUB_ENTRY_BYTES) as usize);
    entry
        .take(MAX_EPUB_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read: {error}"))?;
    if bytes.len() as u64 > MAX_EPUB_ENTRY_BYTES {
        return Err(format!("exceeds {} byte cap", MAX_EPUB_ENTRY_BYTES));
    }
    if is_xml_like(name, media_type) {
        let decoded = decode_xml_bytes(&bytes);
        if contains_entity_decl(decoded.as_bytes()) {
            return Err("entity declarations are not allowed".to_string());
        }
        return sanitize_xml(decoded.as_bytes()).map_err(|error| format!("sanitize: {error}"));
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".css") || media_type.eq_ignore_ascii_case("text/css") {
        let decoded = decode_xml_bytes(&bytes);
        return Ok(sanitize_css_block(&decoded).into_bytes());
    }
    Ok(bytes)
}

/// Import `src` (an .epub path already on local disk) into
/// `library_root`. Writes the book dir (original `.epub` preserved at the
/// book root as `source.epub`, unpacked sanitized content under `content/`,
/// `manifest.json`, `publication.json`). Atomic-ish: stages in a temp dir
/// inside the library and renames on success.
pub fn import_epub_file(
    src: &Path,
    library_root: &Path,
    title_hint: Option<&str>,
) -> Result<EpubImportOutcome, String> {
    let src = &crate::atomic_file::long_path(src);
    let metadata = fs::metadata(src)
        .map_err(|error| format!("EPUB_READ: cannot stat {}: {error}", src.display()))?;
    if !metadata.is_file() {
        return Err("EPUB_NOT_A_ZIP: source is not a file".to_string());
    }
    if metadata.len() > MAX_EPUB_ARCHIVE_BYTES {
        return Err(format!(
            "EPUB_TOO_LARGE: archive {} bytes exceeds {} byte cap",
            metadata.len(),
            MAX_EPUB_ARCHIVE_BYTES
        ));
    }
    // Stable id basis: sha256 of the file bytes (used when the OPF carries no
    // identifier). Streaming hash — the archive is read again via zip below.
    let file_sha256 = {
        let mut file = fs::File::open(src)
            .map_err(|error| format!("EPUB_READ: {}: {error}", src.display()))?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|error| format!("EPUB_READ: {}: {error}", src.display()))?;
        format!("{:x}", hasher.finalize())
    };

    let file =
        fs::File::open(src).map_err(|error| format!("EPUB_READ: {}: {error}", src.display()))?;
    let mut archive = ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|_| "EPUB_NOT_A_ZIP: not a readable zip archive".to_string())?;
    if archive.len() > MAX_EPUB_ENTRIES {
        return Err(format!(
            "EPUB_TOO_LARGE: {} entries exceeds {} entry cap",
            archive.len(),
            MAX_EPUB_ENTRIES
        ));
    }

    // --- First pass: entry table, size caps, DRM / container / iBooks flags.
    struct EntryMeta {
        name: String,
    }
    let mut entries: Vec<EntryMeta> = Vec::with_capacity(archive.len().min(512));
    let mut expanded_total = 0_u64;
    let mut entry_names: HashSet<String> = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("EPUB_MALFORMED_ZIP: entry {index}: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let size = entry.size();
        if size > MAX_EPUB_ENTRY_BYTES {
            return Err(format!(
                "EPUB_TOO_LARGE: entry {name} is {size} bytes (> {} byte cap)",
                MAX_EPUB_ENTRY_BYTES
            ));
        }
        expanded_total = expanded_total.saturating_add(size);
        if expanded_total > MAX_EPUB_EXPANDED_BYTES {
            return Err(format!(
                "EPUB_TOO_LARGE: expanded size exceeds {} byte cap",
                MAX_EPUB_EXPANDED_BYTES
            ));
        }
        entry_names.insert(name.clone());
        entries.push(EntryMeta { name });
    }

    let has_entry = |name: &str| entry_names.iter().any(|n| n == name);
    let encryption_entry = entry_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("meta-inf/encryption.xml"));
    if encryption_entry {
        return Err("EPUB_DRM_UNSUPPORTED: META-INF/encryption.xml present".to_string());
    }
    let apple_fixed_layout = entry_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("meta-inf/com.apple.ibooks.display-options"));

    if !has_entry("META-INF/container.xml") {
        return Err("EPUB_MALFORMED_CONTAINER: META-INF/container.xml missing".to_string());
    }
    let container_bytes = {
        let mut entry = archive
            .by_name("META-INF/container.xml")
            .map_err(|error| format!("EPUB_MALFORMED_CONTAINER: {error}"))?;
        let mut bytes = Vec::with_capacity(entry.size().min(64 * 1024) as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("EPUB_MALFORMED_CONTAINER: {error}"))?;
        bytes
    };
    let rootfile = parse_container(&container_bytes)?;

    let opf_name = resolve_href("", &rootfile)
        .ok_or_else(|| "EPUB_MALFORMED_CONTAINER: unsafe rootfile path".to_string())?;
    if !entry_names.contains(&opf_name) {
        return Err(format!("EPUB_MALFORMED_OPF: {opf_name} not in archive"));
    }
    let opf_bytes = {
        let mut entry = archive
            .by_name(&opf_name)
            .map_err(|error| format!("EPUB_MALFORMED_OPF: {error}"))?;
        if entry.size() > MAX_EPUB_ENTRY_BYTES {
            return Err("EPUB_TOO_LARGE: OPF document".to_string());
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("EPUB_MALFORMED_OPF: {error}"))?;
        bytes
    };
    let opf = parse_opf(&opf_bytes)?;
    let opf_dir = opf_name
        .rfind('/')
        .map(|index| opf_name[..index].to_string())
        .unwrap_or_default();
    let epub_version = match opf.version.trim().chars().next() {
        Some('3') => "3",
        _ => "2",
    }
    .to_string();

    // --- Spine resolution: slot index → archive path. Spine problems abort
    // the whole import — a book with a missing chapter is not importable.
    let mut issues: Vec<LibraryIssue> = Vec::new();
    let mut spine_archive: Vec<String> = Vec::with_capacity(opf.spine.len());
    let mut spine_names: HashSet<String> = HashSet::new();
    for idref in &opf.spine {
        let Some(item) = opf.item(idref) else {
            return Err(format!(
                "EPUB_MALFORMED_SPINE: itemref {idref} missing from manifest"
            ));
        };
        let Some(archive_path) = resolve_href(&opf_dir, &item.href) else {
            return Err(format!(
                "EPUB_MALFORMED_SPINE: unresolvable href {:?} for {idref}",
                item.href
            ));
        };
        if !entry_names.contains(&archive_path) {
            return Err(format!(
                "EPUB_MALFORMED_SPINE: {archive_path} not in archive"
            ));
        }
        spine_names.insert(archive_path.clone());
        spine_archive.push(archive_path);
    }

    // --- Extraction plan.
    // `default_targets`: archive name → `content/…` for every safely-named
    // member (case-folded collisions drop the loser with an issue). Unsafe
    // names drop with an issue — except spine documents, which still extract
    // under a synthesized `content/chapter-<i>.xhtml`.
    let mut default_targets: HashMap<String, String> = HashMap::new();
    let mut used_targets: HashSet<String> = HashSet::new(); // case-folded
    for entry in &entries {
        match safe_archive_path(&entry.name) {
            Some(rel) => {
                let target = format!("content/{rel}");
                if !used_targets.insert(target.to_lowercase()) {
                    issue(
                        &mut issues,
                        &entry.name,
                        "EPUB_ENTRY_COLLISION: duplicate archive path — dropped",
                    );
                    continue;
                }
                default_targets.insert(entry.name.clone(), target);
            }
            None => {
                if spine_names.contains(&entry.name) {
                    issue(
                        &mut issues,
                        &entry.name,
                        "EPUB_ENTRY_RENAMED: unsafe archive path — extracted under a synthesized name",
                    );
                } else {
                    issue(
                        &mut issues,
                        &entry.name,
                        "EPUB_ENTRY_DROPPED: unsafe archive path",
                    );
                }
            }
        }
    }
    // Slot index → book-relative chapter path. The first spine slot claiming
    // a safely-named member reuses its `content/…` target; later repeats of
    // the same document and unsafe names each get a synthesized target,
    // written by `extra_writes` after the main pass.
    let mut slot_targets: Vec<String> = Vec::with_capacity(opf.spine.len());
    let mut claimed_spine: HashSet<String> = HashSet::new();
    let mut extra_writes: Vec<(String, String)> = Vec::new();
    for (index, archive_path) in spine_archive.iter().enumerate() {
        match default_targets.get(archive_path) {
            Some(target) if claimed_spine.insert(archive_path.clone()) => {
                slot_targets.push(target.clone());
            }
            _ => {
                let mut candidate = format!("content/chapter-{index}.xhtml");
                let mut counter = 2_u32;
                while !used_targets.insert(candidate.to_lowercase()) {
                    candidate = format!("content/chapter-{index}-{counter}.xhtml");
                    counter += 1;
                }
                extra_writes.push((archive_path.clone(), candidate.clone()));
                slot_targets.push(candidate);
            }
        }
    }

    // --- Staging + extraction.
    let manual_root = crate::atomic_file::long_path(&library_root.join("手动"));
    fs::create_dir_all(&manual_root).map_err(|error| error.to_string())?;
    let staging_root = crate::atomic_file::long_path(&manual_root.join(IMPORT_STAGING_DIR));
    fs::create_dir_all(&staging_root).map_err(|error| error.to_string())?;
    let staging = crate::atomic_file::long_path(&staging_root.join(Uuid::new_v4().to_string()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

    let title = title_hint
        .map(|hint| hint.trim())
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
        .or_else(|| opf.title.clone().filter(|title| !title.trim().is_empty()))
        .or_else(|| {
            src.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
                .filter(|stem| !stem.trim().is_empty())
        })
        .unwrap_or_else(|| "未命名书目".to_string());
    let target = unique_book_dir(&manual_root, &title);

    // Manifest-item id ↔ archive path (resource map + media-type lookup).
    let mut item_by_path: HashMap<String, &OpfItem> = HashMap::new();
    for item in &opf.items {
        if let Some(path) = resolve_href(&opf_dir, &item.href) {
            item_by_path.insert(path, item);
        }
    }

    let staged = (|| -> Result<EpubImportOutcome, String> {
        // `source.epub` — verbatim copy of the archive.
        crate::atomic_file::copy_file_synced(src, &staging.join("source.epub"))
            .map_err(|error| format!("EPUB_WRITE: source.epub: {error}"))?;

        // Extract every member that has a planned target. Spine documents
        // abort on failure; junk resources degrade to an issue.
        for entry_meta in &entries {
            let Some(target) = default_targets.get(&entry_meta.name).cloned() else {
                continue;
            };
            let media_type = item_by_path
                .get(&entry_meta.name)
                .map(|item| item.media_type.clone())
                .unwrap_or_default();
            let is_spine = spine_names.contains(&entry_meta.name);
            match read_entry(&mut archive, &entry_meta.name, &media_type) {
                Ok(payload) => {
                    if let Err(error) = crate::atomic_file::write(&staging.join(&target), &payload)
                    {
                        if is_spine {
                            return Err(format!("EPUB_WRITE: {}: {error}", entry_meta.name));
                        }
                        issue(
                            &mut issues,
                            &entry_meta.name,
                            format!("EPUB_ENTRY_WRITE: {error}"),
                        );
                    }
                }
                Err(error) => {
                    if is_spine {
                        return Err(format!(
                            "EPUB_MALFORMED_SPINE: {}: {error}",
                            entry_meta.name
                        ));
                    }
                    issue(
                        &mut issues,
                        &entry_meta.name,
                        format!("EPUB_ENTRY_EXTRACT: {error}"),
                    );
                }
            }
        }
        // Spine slots needing their own target (unsafe name / repeat ref).
        for (archive_name, target) in &extra_writes {
            let media_type = item_by_path
                .get(archive_name)
                .map(|item| item.media_type.clone())
                .unwrap_or_default();
            let payload = read_entry(&mut archive, archive_name, &media_type)
                .map_err(|error| format!("EPUB_MALFORMED_SPINE: {archive_name}: {error}"))?;
            crate::atomic_file::write(&staging.join(target), &payload)
                .map_err(|error| format!("EPUB_WRITE: {target}: {error}"))?;
        }

        // --- Navigation: EPUB 3 nav document, else EPUB 2 NCX.
        let nav_raw: Option<(String, Vec<u8>)> = if let Some(item) = opf.nav_item() {
            resolve_href(&opf_dir, &item.href).and_then(|path| {
                archive.by_name(&path).ok().and_then(|mut entry| {
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes).ok().map(|_| (path, bytes))
                })
            })
        } else {
            opf.ncx_item().and_then(|item| {
                resolve_href(&opf_dir, &item.href).and_then(|path| {
                    archive.by_name(&path).ok().and_then(|mut entry| {
                        let mut bytes = Vec::new();
                        entry.read_to_end(&mut bytes).ok().map(|_| (path, bytes))
                    })
                })
            })
        };
        let mut nav: Vec<NavItem> = Vec::new();
        let mut chapter_titles: HashMap<usize, String> = HashMap::new();
        if let Some((nav_path, nav_bytes)) = nav_raw {
            let decoded = decode_xml_bytes(&nav_bytes);
            let temps = if opf.nav_item().is_some() {
                parse_nav_xhtml(decoded.as_bytes())
            } else {
                parse_ncx(decoded.as_bytes())
            };
            let nav_dir = nav_path
                .rfind('/')
                .map(|i| nav_path[..i].to_string())
                .unwrap_or_default();
            // Archive path → first spine slot referencing it (`.rev()` so a
            // repeated itemref keeps the earliest index for nav resolution).
            let archive_to_spine: HashMap<String, usize> = spine_archive
                .iter()
                .enumerate()
                .rev()
                .map(|(index, path)| (path.clone(), index))
                .collect();
            let resolve = |href: &str| -> Option<String> {
                let archive = resolve_href(&nav_dir, href)?;
                archive_to_spine
                    .get(&archive)
                    .map(|index| format!("epub-ch-{index}"))
            };
            // Record first-seen titles per spine index for chapter naming.
            fn collect_titles(
                temps: &[NavTmp],
                nav_dir: &str,
                archive_to_spine: &HashMap<String, usize>,
                out: &mut HashMap<usize, String>,
            ) {
                for tmp in temps {
                    if let (Some(href), title) = (tmp.href.as_deref(), tmp.title.trim()) {
                        if !title.is_empty() {
                            if let Some(archive) = resolve_href(nav_dir, href) {
                                if let Some(index) = archive_to_spine.get(&archive) {
                                    out.entry(*index).or_insert_with(|| title.to_string());
                                }
                            }
                        }
                    }
                    collect_titles(&tmp.children, nav_dir, archive_to_spine, out);
                }
            }
            collect_titles(&temps, &nav_dir, &archive_to_spine, &mut chapter_titles);
            nav = promote_unresolved_nav(build_nav(temps, &resolve));
        }

        // --- manifest.json
        let mut chapters = Vec::with_capacity(opf.spine.len());
        for (index, idref) in opf.spine.iter().enumerate() {
            let path = slot_targets[index].clone();
            let item = opf.item(idref).expect("spine resolved above");
            let chapter_title = chapter_titles
                .get(&index)
                .cloned()
                .or_else(|| {
                    Path::new(&item.href)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(|stem| {
                            percent_encoding::percent_decode_str(stem)
                                .decode_utf8()
                                .map(|s| s.into_owned())
                                .unwrap_or_else(|_| stem.to_string())
                        })
                        .filter(|stem| !stem.trim().is_empty())
                })
                .unwrap_or_else(|| format!("章节 {}", index + 1));
            let words = fs::read(staging.join(&path))
                .ok()
                .and_then(|bytes| {
                    let decoded = decode_xml_bytes(&bytes);
                    xhtml_to_text(decoded.as_bytes()).ok()
                })
                .map(|text| word_count(&text))
                .unwrap_or(0);
            chapters.push(Chapter {
                id: format!("epub-ch-{index}"),
                path,
                title: chapter_title,
                date: None,
                vote_count: 0,
                word_count: words,
                metadata_status: None,
            });
        }
        if chapters.is_empty() {
            return Err("EPUB_MALFORMED_SPINE: no chapters extracted".to_string());
        }
        let now = Utc::now().to_rfc3339();
        let book_id = match opf.identifier().filter(|id| !id.trim().is_empty()) {
            Some(identifier) => {
                let digest = Sha256::digest(format!("{identifier}\u{1f}{title}").as_bytes());
                format!("epub:{:x}", digest)[..18].to_string()
            }
            None => format!("epub:{}", &file_sha256[..16]),
        };
        let source_id = opf
            .identifier()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .or_else(|| {
                src.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            });
        let manifest = Manifest {
            schema_version: 1,
            book_id,
            title: title.clone(),
            source: "manual".to_string(),
            source_id,
            generated_at: now.clone(),
            updated_at: now,
            chapters,
        };
        validate_manifest(&manifest).map_err(|error| format!("EPUB_MANIFEST_INVALID: {error}"))?;

        // --- publication.json
        let mut unsupported: Vec<String> = Vec::new();
        if opf.fixed_layout || apple_fixed_layout {
            unsupported.push("fixed-layout".to_string());
        }
        if opf.media_overlay || opf.items.iter().any(|item| item.has_media_overlay) {
            unsupported.push("media-overlay".to_string());
        }
        if opf.scripted() {
            unsupported.push("scripted".to_string());
        }
        let cover = opf.cover_item().and_then(|item| {
            resolve_href(&opf_dir, &item.href).and_then(|archive_path| {
                default_targets
                    .get(&archive_path)
                    .cloned()
                    .filter(|target| staging.join(target).exists())
            })
        });
        let resources: HashMap<String, String> = opf
            .items
            .iter()
            .filter_map(|item| {
                let archive_path = resolve_href(&opf_dir, &item.href)?;
                default_targets
                    .get(&archive_path)
                    .cloned()
                    .filter(|target| staging.join(target).exists())
                    .map(|target| (item.id.clone(), target))
            })
            .collect();
        let publication = Publication {
            schema_version: Publication::SCHEMA_VERSION,
            format: Publication::FORMAT_EPUB.to_string(),
            epub_version,
            title: title.clone(),
            creator: opf.creator.clone(),
            language: opf.language.clone(),
            cover,
            nav,
            spine: manifest.chapters.iter().map(|c| c.id.clone()).collect(),
            resources,
            unsupported,
        };
        crate::contracts::validate_publication(&publication)
            .map_err(|error| format!("EPUB_PUBLICATION_INVALID: {error}"))?;

        let manifest_json =
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        crate::atomic_file::write(&staging.join("manifest.json"), &manifest_json)
            .map_err(|error| format!("EPUB_WRITE: manifest.json: {error}"))?;
        let publication_json =
            serde_json::to_vec_pretty(&publication).map_err(|error| error.to_string())?;
        crate::atomic_file::write(&staging.join("publication.json"), &publication_json)
            .map_err(|error| format!("EPUB_WRITE: publication.json: {error}"))?;

        Ok(EpubImportOutcome { manifest, issues })
    })();

    match staged {
        Ok(outcome) => {
            let final_target = crate::atomic_file::long_path(&target);
            if let Err(error) = fs::rename(&staging, &final_target) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("EPUB_FINALIZE: {error}"));
            }
            Ok(outcome)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// A chapter ready for the reader surface.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadableChapter {
    /// "xhtml" for EPUB chapters, "markdown" for legacy books.
    pub format: String,
    pub chapter_id: String,
    pub title: String,
    /// Sanitized XHTML (format=xhtml) or decoded Markdown source.
    pub content: String,
    /// Book-relative directory the chapter lives in — the frontend resolves
    /// relative resource URLs against `bookDir/<resourceDir>/`.
    pub resource_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_chapter_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_chapter_id: Option<String>,
}

/// prev/next chapter ids from the book's manifest.
fn chapter_neighbors(book_root: &Path, chapter_id: &str) -> (Option<String>, Option<String>) {
    let bytes = match fs::read(crate::atomic_file::long_path(
        &book_root.join("manifest.json"),
    )) {
        Ok(bytes) => bytes,
        Err(_) => return (None, None),
    };
    let manifest: Manifest = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(_) => return (None, None),
    };
    let Some(index) = manifest
        .chapters
        .iter()
        .position(|chapter| chapter.id == chapter_id)
    else {
        return (None, None);
    };
    let prev = index
        .checked_sub(1)
        .and_then(|i| manifest.chapters.get(i))
        .map(|chapter| chapter.id.clone());
    let next = manifest
        .chapters
        .get(index + 1)
        .map(|chapter| chapter.id.clone());
    (prev, next)
}

/// Read + sanitize one chapter of an EPUB book.
/// `chapter` must come from the book's own manifest.
pub fn read_epub_chapter(book_root: &Path, chapter: &Chapter) -> Result<ReadableChapter, String> {
    if !is_safe_relative_path(&chapter.path) {
        return Err(format!("EPUB_UNSAFE_CHAPTER_PATH: {}", chapter.path));
    }
    let path = crate::atomic_file::long_path(
        &book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
    );
    let bytes = fs::read(&path).map_err(|error| format!("EPUB_CHAPTER_READ: {error}"))?;
    let decoded = decode_xml_bytes(&bytes);
    // Defense-in-depth: content was sanitized at import; re-run the pass so a
    // tampered book file still reaches the reader sanitized.
    let content = match sanitize_xml(decoded.as_bytes()) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => decoded,
    };
    let resource_dir = Path::new(&chapter.path)
        .parent()
        .map(|parent| parent.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let (prev_chapter_id, next_chapter_id) = chapter_neighbors(book_root, &chapter.id);
    Ok(ReadableChapter {
        format: "xhtml".to_string(),
        chapter_id: chapter.id.clone(),
        title: chapter.title.clone(),
        content,
        resource_dir,
        prev_chapter_id,
        next_chapter_id,
    })
}

/// Plain text of an EPUB chapter for the FTS index (tags stripped).
pub fn chapter_plain_text(book_root: &Path, chapter: &Chapter) -> Result<String, String> {
    if !is_safe_relative_path(&chapter.path) {
        return Err(format!("EPUB_UNSAFE_CHAPTER_PATH: {}", chapter.path));
    }
    let path = crate::atomic_file::long_path(
        &book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
    );
    let bytes = fs::read(&path).map_err(|error| format!("EPUB_CHAPTER_READ: {error}"))?;
    let decoded = decode_xml_bytes(&bytes);
    xhtml_to_text(decoded.as_bytes())
}

/// Lexical normalization of an absolute-ish path: fold `.`/`..` over
/// components; `..` escaping the root prefix returns `None`.
fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(segment) => out.push(segment),
        }
    }
    Some(out)
}

/// Resolve a resource href relative to a chapter inside the book dir,
/// refusing escapes. Returns the absolute path when contained.
pub fn resolve_book_resource(book_root: &Path, chapter_path: &Path, href: &str) -> Option<PathBuf> {
    let href = href.split(['#', '?']).next()?;
    if href.is_empty() {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8()
        .ok()?;
    let decoded = decoded.as_ref();
    if decoded.is_empty() || decoded.starts_with('/') || decoded.starts_with('\\') {
        return None;
    }
    // Reject scheme / drive-letter prefixes before joining.
    let head = decoded.split(['/', '\\']).next().unwrap_or(decoded);
    if head.contains(':') {
        return None;
    }
    let chapter_abs = if chapter_path.is_absolute() {
        chapter_path.to_path_buf()
    } else {
        book_root.join(chapter_path)
    };
    let base = chapter_abs.parent()?.to_path_buf();
    let candidate = normalize_lexical(&base.join(decoded))?;
    let root_norm = normalize_lexical(book_root)?;
    // Canonical containment check when both sides exist (catches symlink
    // escapes); otherwise the lexical containment is authoritative — the
    // book dir is app-managed with no junctions inside.
    let contained = match (candidate.canonicalize(), root_norm.canonicalize()) {
        (Ok(canonical_candidate), Ok(canonical_root)) => {
            canonical_candidate.starts_with(&canonical_root)
        }
        _ => candidate.starts_with(&root_norm),
    };
    if contained && candidate != root_norm {
        Some(candidate)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests — synthetic epubs built in memory via zip::ZipWriter
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "immersive-epub-{tag}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn build_epub(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            writer.start_file(*name, options).expect("zip entry");
            writer.write_all(bytes).expect("zip write");
        }
        writer.finish().expect("zip finish").into_inner()
    }

    fn write_epub(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        fs::create_dir_all(root).expect("root");
        let path = root.join(name);
        fs::write(&path, bytes).expect("epub write");
        path
    }

    const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    const OPF3: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id"
  xmlns:dc="http://purl.org/dc/elements/1.1/">
 <metadata>
  <dc:identifier id="pub-id">urn:uuid:test-epub3</dc:identifier>
  <dc:title>测试书</dc:title>
  <dc:creator>作者</dc:creator>
  <dc:language>zh-CN</dc:language>
 </metadata>
 <manifest>
  <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  <item id="ch2" href="dir/ch2.xhtml" media-type="application/xhtml+xml"/>
  <item id="img" href="images/a.png" media-type="image/png" properties="cover-image"/>
 </manifest>
 <spine><itemref idref="ch1"/><itemref idref="ch2"/></spine>
</package>"#;

    const NAV: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol>
 <li><a href="ch1.xhtml">第一章</a><ol><li><a href="ch1.xhtml#s1">1.1 节</a></li></ol></li>
 <li><a href="dir/ch2.xhtml">第二章</a></li>
</ol></nav></body></html>"#;

    const OPF2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="pub-id"
  xmlns:dc="http://purl.org/dc/elements/1.1/">
 <metadata>
  <dc:identifier id="pub-id">test-epub2-id</dc:identifier>
  <dc:title>旧书</dc:title>
 </metadata>
 <manifest>
  <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
 </manifest>
 <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#;

    const NCX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
 <navMap><navPoint id="n1" playOrder="1"><navLabel><text>第一章 NCX</text></navLabel>
 <content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#;

    const CH1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head>
<body><p id="s1">正文 第一章</p></body></html>"#;

    const CH2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p>第二 章</p></body></html>"#;

    fn epub3_files() -> Vec<(&'static str, &'static [u8])> {
        vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", OPF3.as_bytes()),
            ("OEBPS/nav.xhtml", NAV.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("OEBPS/dir/ch2.xhtml", CH2.as_bytes()),
            ("OEBPS/images/a.png", &[0x89, 0x50, 0x4E, 0x47][..]),
        ]
    }

    #[test]
    fn imports_epub3_with_nav_document() {
        let root = temp_root("epub3");
        let library = root.join("library");
        let epub = write_epub(&root, "book3.epub", &build_epub(&epub3_files()));
        let outcome = import_epub_file(&epub, &library, None).expect("import");
        assert_eq!(outcome.manifest.chapters.len(), 2);
        assert_eq!(outcome.manifest.chapters[0].id, "epub-ch-0");
        assert_eq!(outcome.manifest.chapters[0].title, "第一章");
        assert_eq!(
            outcome.manifest.chapters[1].path,
            "content/OEBPS/dir/ch2.xhtml"
        );
        assert!(outcome.manifest.book_id.starts_with("epub:"));
        assert_eq!(outcome.manifest.source, "manual");
        validate_manifest(&outcome.manifest).expect("manifest valid");

        let book_dir = library.join("手动/测试书");
        assert!(book_dir.join("source.epub").is_file());
        assert!(book_dir.join("content/OEBPS/ch1.xhtml").is_file());
        assert!(book_dir.join("content/OEBPS/images/a.png").is_file());
        let publication = load_publication(&book_dir)
            .expect("load")
            .expect("sidecar exists");
        assert_eq!(publication.epub_version, "3");
        assert_eq!(publication.title, "测试书");
        assert_eq!(publication.creator.as_deref(), Some("作者"));
        assert_eq!(publication.nav.len(), 2);
        assert_eq!(publication.nav[0].children.len(), 1);
        assert_eq!(publication.nav[0].children[0].chapter_id, "epub-ch-0");
        assert_eq!(publication.spine, vec!["epub-ch-0", "epub-ch-1"]);
        assert_eq!(
            publication.cover.as_deref(),
            Some("content/OEBPS/images/a.png")
        );
        assert_eq!(
            publication.resources.get("img").map(String::as_str),
            Some("content/OEBPS/images/a.png")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn imports_epub2_with_ncx() {
        let root = temp_root("epub2");
        let library = root.join("library");
        let files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", OPF2.as_bytes()),
            ("OEBPS/toc.ncx", NCX.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
        ];
        let epub = write_epub(&root, "book2.epub", &build_epub(&files));
        let outcome = import_epub_file(&epub, &library, None).expect("import");
        assert_eq!(outcome.manifest.chapters.len(), 1);
        assert_eq!(outcome.manifest.chapters[0].title, "第一章 NCX");
        let book_dir = library.join("手动/旧书");
        let publication = load_publication(&book_dir).expect("load").expect("sidecar");
        assert_eq!(publication.epub_version, "2");
        assert_eq!(publication.nav.len(), 1);
        assert_eq!(publication.nav[0].chapter_id, "epub-ch-0");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn encrypted_epub_is_rejected() {
        let root = temp_root("drm");
        let library = root.join("library");
        let files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            (
                "META-INF/encryption.xml",
                r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><EncryptedData/></encryption>"#.as_bytes(),
            ),
            ("OEBPS/content.opf", OPF3.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("OEBPS/dir/ch2.xhtml", CH2.as_bytes()),
        ];
        let epub = write_epub(&root, "drm.epub", &build_epub(&files));
        let error = import_epub_file(&epub, &library, None).expect_err("drm must fail");
        assert!(error.starts_with("EPUB_DRM_UNSUPPORTED"), "got {error}");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn non_zip_input_is_rejected() {
        let root = temp_root("notzip");
        let library = root.join("library");
        let epub = write_epub(&root, "fake.epub", b"this is not a zip file");
        let error = import_epub_file(&epub, &library, None).expect_err("must fail");
        assert!(error.starts_with("EPUB_NOT_A_ZIP"), "got {error}");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn oversized_archive_is_rejected() {
        let root = temp_root("oversize");
        let library = root.join("library");
        fs::create_dir_all(&root).expect("root");
        let epub = root.join("big.epub");
        let file = fs::File::create(&epub).expect("create");
        file.set_len(MAX_EPUB_ARCHIVE_BYTES + 1)
            .expect("sparse set_len");
        drop(file);
        let error = import_epub_file(&epub, &library, None).expect_err("must fail");
        assert!(error.starts_with("EPUB_TOO_LARGE"), "got {error}");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn unsafe_entry_is_dropped_but_import_succeeds() {
        let root = temp_root("traversal");
        let library = root.join("library");
        let files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", OPF2.as_bytes()),
            ("OEBPS/toc.ncx", NCX.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
            ("../evil.txt", b"escape"[..].as_ref()),
            ("OEBPS/sub\\bad.txt", b"backslash"[..].as_ref()),
        ];
        let epub = write_epub(&root, "trav.epub", &build_epub(&files));
        let outcome = import_epub_file(&epub, &library, None).expect("import must succeed");
        assert!(
            outcome
                .issues
                .iter()
                .any(|issue| issue.message.contains("EPUB_ENTRY_DROPPED")),
            "issues: {:?}",
            outcome.issues
        );
        assert_eq!(outcome.manifest.chapters.len(), 1);
        let book_dir = library.join("手动/旧书");
        assert!(!book_dir.join("evil.txt").exists());
        assert!(!root.join("evil.txt").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn fixed_layout_is_flagged_unsupported() {
        let root = temp_root("fxl");
        let library = root.join("library");
        let opf_fxl: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" xmlns:dc="http://purl.org/dc/elements/1.1/">
 <metadata>
  <dc:identifier>fxl-id</dc:identifier><dc:title>固定版式</dc:title>
  <meta property="rendition:layout">pre-paginated</meta>
 </metadata>
 <manifest>
  <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml" properties="scripted"/>
 </manifest>
 <spine><itemref idref="ch1"/></spine>
</package>"#;
        let files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", opf_fxl.as_bytes()),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
        ];
        let epub = write_epub(&root, "fxl.epub", &build_epub(&files));
        import_epub_file(&epub, &library, None).expect("import");
        let publication = load_publication(&library.join("手动/固定版式"))
            .expect("load")
            .expect("sidecar");
        assert!(publication
            .unsupported
            .contains(&"fixed-layout".to_string()));
        assert!(publication.unsupported.contains(&"scripted".to_string()));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn sanitizer_removes_scripts_handlers_and_remote_refs() {
        let dirty: &[u8] = r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head>
<script type="text/javascript">alert(1)</script>
<meta http-equiv="refresh" content="0;url=https://evil.example/"/>
<link rel="stylesheet" href="https://evil.example/x.css"/>
<style>p { color: red; background: url(https://evil.example/bg.png) } @import "https://evil.example/y.css";</style>
</head><body>
<p onclick="steal()" style="color:blue;behavior:url(x);font-weight:bold" id="keep">hi &amp; bye</p>
<img src="https://evil.example/p.png" alt="pic"/>
<img src="data:image/png;base64,AAAA" alt="ok"/>
<a href="javascript:alert(1)">bad</a>
<a href="https://example.com/ok">ext</a>
<a href="#frag">frag</a>
<iframe src="https://evil.example"></iframe>
<svg><rect width="5" height="5"/><foreignObject><p>bad</p></foreignObject><script>x()</script></svg>
</body></html>"##
            .as_bytes();
        let out = sanitize_xml(dirty).expect("sanitize");
        let text = String::from_utf8(out).expect("utf8");
        assert!(!text.contains("<script"), "{text}");
        assert!(!text.contains("onclick"), "{text}");
        assert!(!text.contains("http-equiv"), "{text}");
        assert!(!text.contains("evil.example/x.css"), "{text}");
        assert!(!text.contains("<iframe"), "{text}");
        assert!(!text.contains("foreignObject"), "{text}");
        assert!(!text.contains("javascript:"), "{text}");
        assert!(text.contains("id=\"keep\""), "{text}");
        assert!(
            text.contains("data-ir-remote-src=\"https://evil.example/p.png\""),
            "{text}"
        );
        assert!(text.contains("data:image/png;base64,AAAA"), "{text}");
        assert!(text.contains("href=\"#frag\""), "{text}");
        // Anchors keep explicit https hrefs (external links allowed).
        assert!(text.contains("href=\"https://example.com/ok\""), "{text}");
        // Inline style kept the whitelisted props, dropped behavior/url.
        assert!(text.contains("font-weight:bold"), "{text}");
        assert!(!text.contains("behavior"), "{text}");
        assert!(!text.contains("@import"), "{text}");
        assert!(!text.contains("url(https"), "{text}");
        // Entities survive verbatim.
        assert!(text.contains("&amp;"), "{text}");
    }

    #[test]
    fn read_chapter_and_plain_text_roundtrip() {
        let root = temp_root("read");
        let library = root.join("library");
        let epub = write_epub(&root, "book3.epub", &build_epub(&epub3_files()));
        let outcome = import_epub_file(&epub, &library, None).expect("import");
        let book_dir = library.join("手动/测试书");
        let chapter = &outcome.manifest.chapters[0];
        let readable = read_epub_chapter(&book_dir, chapter).expect("read");
        assert_eq!(readable.format, "xhtml");
        assert_eq!(readable.resource_dir, "content/OEBPS");
        assert_eq!(readable.prev_chapter_id, None);
        assert_eq!(readable.next_chapter_id.as_deref(), Some("epub-ch-1"));
        assert!(readable.content.contains("正文 第一章"));

        let text = chapter_plain_text(&book_dir, chapter).expect("text");
        assert!(text.contains("正文 第一章"));
        assert!(!text.contains('<'), "{text}");

        let second = read_epub_chapter(&book_dir, &outcome.manifest.chapters[1]).expect("read");
        assert_eq!(second.prev_chapter_id.as_deref(), Some("epub-ch-0"));
        assert_eq!(second.next_chapter_id, None);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn resolve_resource_stays_inside_book() {
        let root = temp_root("resolve");
        let book = root.join("book");
        fs::create_dir_all(book.join("content/OEBPS/dir")).expect("dirs");
        fs::write(book.join("content/OEBPS/img.png"), b"png").expect("img");
        let chapter = Path::new("content/OEBPS/dir/ch.xhtml");
        let resolved =
            resolve_book_resource(&book, chapter, "../img.png").expect("must resolve inside");
        assert!(resolved.ends_with("img.png"), "{resolved:?}");
        // `../../outside.png` folds to `content/outside.png` — still inside
        // the book root, so it resolves. Four `..` segments escape.
        assert!(resolve_book_resource(&book, chapter, "../../outside.png").is_some());
        assert!(resolve_book_resource(&book, chapter, "../../../../outside.png").is_none());
        assert!(resolve_book_resource(&book, chapter, "https://x/y.png").is_none());
        assert!(resolve_book_resource(&book, chapter, "img.png#frag").is_some());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn empty_spine_is_a_hard_error() {
        let root = temp_root("empty-spine");
        let library = root.join("library");
        let opf: &[u8] = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" xmlns:dc="http://purl.org/dc/elements/1.1/">
 <metadata><dc:title>x</dc:title></metadata>
 <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
 <spine></spine>
</package>"#
            .as_bytes();
        let files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/content.opf", opf),
            ("OEBPS/ch1.xhtml", CH1.as_bytes()),
        ];
        let epub = write_epub(&root, "empty.epub", &build_epub(&files));
        let error = import_epub_file(&epub, &library, None).expect_err("must fail");
        assert!(error.starts_with("EPUB_MALFORMED"), "got {error}");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
