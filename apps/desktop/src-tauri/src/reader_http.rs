use crate::contracts::{is_safe_relative_path, Manifest, ReadingProgress};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const MAX_PROGRESS_BODY: usize = 64 * 1024;
/// Largest request body the server will buffer. `PUT /progress` is the only
/// route that carries a body; anything larger is rejected before the body is
/// ever read.
pub(crate) const MAX_REQUEST_BODY: u64 = MAX_PROGRESS_BODY as u64;
pub(crate) const MAX_READER_SESSIONS: usize = 16;
pub(crate) const READER_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// P2-23: the reader document is a compiled single-file bundle whose whole
/// resource surface is inline <script>/<style> blocks, same-origin fetches
/// (`manifest`/`progress`/`content`/`heartbeat`) and same-origin or https
/// images. Everything else is denied. `frame-ancestors` names every parent
/// origin that legitimately embeds the reader iframe: the vite dev origin,
/// and the Tauri production origins (Windows uses http://tauri.localhost).
const READER_DOCUMENT_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors http://localhost:1420 http://tauri.localhost https://tauri.localhost tauri://localhost";
/// Non-document routes (JSON, progress, chapter bytes) get the strictest
/// policy — it also defangs scriptable payloads such as image/svg+xml when
/// a content URL is navigated to directly instead of loaded via <img>.
const CONTENT_CSP: &str = "default-src 'none'; frame-ancestors 'none'";
/// P3-20: SVG can carry script when served same-origin and navigated to as a
/// document. On top of the strict content CSP, `sandbox` pins it into a
/// scriptless opaque origin; <img>-embedded SVG is unaffected (a response CSP
/// only applies to document/worker contexts).
const SVG_CSP: &str = "sandbox; default-src 'none'; frame-ancestors 'none'";

#[derive(Clone)]
pub struct ReaderSession {
    pub book_root: PathBuf,
    pub manifest: Manifest,
}

pub type Sessions = Arc<RwLock<HashMap<String, (ReaderSession, Instant)>>>;

/// A fully parsed HTTP request. All socket IO lives in `reader_server`; this
/// module only routes and builds responses, which keeps every handler pure
/// and unit-testable.
pub(crate) struct ReaderRequest {
    /// Uppercase method token from the request line ("GET", "PUT", ...).
    pub method: String,
    /// Raw request target, e.g. `/s/<token>/progress?x=1`.
    pub target: String,
    /// (name, value) header pairs with lower-cased names.
    pub headers: Vec<(String, String)>,
    /// Advertised Content-Length when the header was present and parseable —
    /// lets the progress route reject oversized bodies that were never read.
    pub content_length: Option<u64>,
    /// Body bytes actually read (server-capped at [`MAX_REQUEST_BODY`]).
    pub body: Vec<u8>,
}

impl ReaderRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

pub(crate) enum ReaderBody {
    Bytes(Vec<u8>),
    /// Shared immutable payload (the compiled reader HTML) — cloned by
    /// reference so each response does not copy the template.
    Shared(Arc<Vec<u8>>),
    File(fs::File),
}

impl ReaderBody {
    pub(crate) fn len(&self) -> Option<u64> {
        match self {
            Self::Bytes(body) => Some(body.len() as u64),
            Self::Shared(body) => Some(body.len() as u64),
            Self::File(file) => file.metadata().ok().map(|meta| meta.len()),
        }
    }
}

pub(crate) struct ReaderResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ReaderBody,
}

fn prune_expired_sessions(sessions: &mut HashMap<String, (ReaderSession, Instant)>, now: Instant) {
    sessions.retain(|_, (_, last_access)| now.duration_since(*last_access) < READER_SESSION_TTL);
}

/// P3-20: insert-time sweep — a crashed webview stops heartbeating, so its
/// orphaned session is dropped once it is past the TTL and can never pin one
/// of the MAX_READER_SESSIONS slots forever. `now` is a parameter so the
/// expiry boundary is unit-testable.
fn insert_session_at(
    sessions: &Sessions,
    token: String,
    session: ReaderSession,
    now: Instant,
) -> Result<(), String> {
    let mut sessions = sessions
        .write()
        .map_err(|_| "Reader session store is unavailable".to_string())?;
    prune_expired_sessions(&mut sessions, now);
    if sessions.len() >= MAX_READER_SESSIONS {
        return Err("READER_SESSION_LIMIT".to_string());
    }
    sessions.insert(token, (session, now));
    Ok(())
}

pub fn insert_session(
    sessions: &Sessions,
    token: String,
    session: ReaderSession,
) -> Result<(), String> {
    insert_session_at(sessions, token, session, Instant::now())
}

pub fn close_session(sessions: &Sessions, session_id: &str) -> Result<bool, String> {
    Ok(sessions
        .write()
        .map_err(|_| "Reader session store is unavailable".to_string())?
        .remove(session_id)
        .is_some())
}

fn session_for(sessions: &Sessions, session_id: &str) -> Option<ReaderSession> {
    let mut sessions = sessions.write().ok()?;
    let now = Instant::now();
    prune_expired_sessions(&mut sessions, now);
    let (session, last_access) = sessions.get_mut(session_id)?;
    *last_access = now;
    Some(session.clone())
}

fn common_headers(content_type: &str, csp: &str) -> Vec<(String, String)> {
    vec![
        ("Content-Type".to_string(), content_type.to_string()),
        ("X-Content-Type-Options".to_string(), "nosniff".to_string()),
        ("Referrer-Policy".to_string(), "no-referrer".to_string()),
        ("Content-Security-Policy".to_string(), csp.to_string()),
    ]
}

/// P3-20: headers for non-document routes. Everything here is only ever
/// consumed by the same-origin reader document, so `Cross-Origin-Resource-
/// Policy: same-origin` is cheap defense in depth — it keeps another origin
/// from embedding book bytes (e.g. a page hot-linking a leaked session URL
/// on the loopback port). The document route must NOT carry it: the reader
/// iframe is embedded cross-origin from the tauri://localhost page.
fn content_headers(content_type: &str, csp: &str) -> Vec<(String, String)> {
    let mut headers = common_headers(content_type, csp);
    headers.push((
        "Cross-Origin-Resource-Policy".to_string(),
        "same-origin".to_string(),
    ));
    headers
}

fn response(status: u16, body: impl Into<Vec<u8>>, content_type: &str) -> ReaderResponse {
    ReaderResponse {
        status,
        headers: content_headers(content_type, CONTENT_CSP),
        body: ReaderBody::Bytes(body.into()),
    }
}

/// Pre-routing error responses (e.g. a malformed request line) built by
/// reader_server — they must carry the same hardening headers as routed
/// responses, not a bare Content-Type.
pub(crate) fn error_response(status: u16, message: &str) -> ReaderResponse {
    response(status, message.as_bytes(), "text/plain; charset=utf-8")
}

/// The one document route (`GET /s/<token>/reader`) — the only response that
/// is ever rendered as a page, so it carries the document-level CSP.
fn document_response(status: u16, body: Arc<Vec<u8>>, content_type: &str) -> ReaderResponse {
    ReaderResponse {
        status,
        headers: common_headers(content_type, READER_DOCUMENT_CSP),
        body: ReaderBody::Shared(body),
    }
}

fn file_response(file: fs::File, content_type: &str) -> ReaderResponse {
    let csp = if content_type == "image/svg+xml" {
        SVG_CSP
    } else {
        CONTENT_CSP
    };
    ReaderResponse {
        status: 200,
        headers: content_headers(content_type, csp),
        body: ReaderBody::File(file),
    }
}

fn json<T: serde::Serialize>(value: &T) -> ReaderResponse {
    match serde_json::to_vec(value) {
        Ok(body) => response(200, body, "application/json; charset=utf-8"),
        Err(error) => response(500, error.to_string(), "text/plain; charset=utf-8"),
    }
}

fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "css" => "text/css; charset=utf-8",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn is_book_resource(relative: &str, manifest: &Manifest) -> bool {
    if manifest
        .chapters
        .iter()
        .any(|chapter| chapter.path == relative)
    {
        return true;
    }
    matches!(
        Path::new(relative)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif" | "svg" | "css" | "woff" | "woff2"
    )
}

fn content_response(session: &ReaderSession, raw_relative: &str) -> ReaderResponse {
    let decoded = match percent_decode_str(raw_relative).decode_utf8() {
        Ok(value) => value.into_owned(),
        Err(_) => return response(400, "Invalid encoded path", "text/plain; charset=utf-8"),
    };
    if !is_safe_relative_path(&decoded) || !is_book_resource(&decoded, &session.manifest) {
        return response(
            403,
            "Content path is not allowed",
            "text/plain; charset=utf-8",
        );
    }
    let candidate = session
        .book_root
        .join(decoded.replace('/', std::path::MAIN_SEPARATOR_STR));
    let canonical_root = match session.book_root.canonicalize() {
        Ok(path) => path,
        Err(error) => return response(500, error.to_string(), "text/plain; charset=utf-8"),
    };
    let canonical_file = match candidate.canonicalize() {
        Ok(path) => path,
        Err(_) => return response(404, "Content file not found", "text/plain; charset=utf-8"),
    };
    if !canonical_file.starts_with(canonical_root) || !canonical_file.is_file() {
        return response(
            403,
            "Content resolves outside the book",
            "text/plain; charset=utf-8",
        );
    }
    match fs::File::open(&canonical_file) {
        Ok(file) => file_response(file, mime_type(&canonical_file)),
        Err(error) => response(500, error.to_string(), "text/plain; charset=utf-8"),
    }
}

/// P2-21 read-merge-write — mirrors `library::merge_progress` exactly so both
/// writers (the Svelte 精读 workspace via `save_book_progress` and this
/// reader via PUT /progress) converge on the same `.reading.json` instead of
/// losing each other's update. The `read` set is a union; the cursor
/// (current/position/updated) goes to whichever side saved most recently,
/// ordered by the RFC-3339 `updated` stamp.
fn merge_progress(existing: &ReadingProgress, incoming: &ReadingProgress) -> ReadingProgress {
    let mut merged = incoming.clone();
    for id in &existing.read {
        if !merged.read.iter().any(|known| known == id) {
            merged.read.push(id.clone());
        }
    }
    if existing.updated > merged.updated {
        merged.current = existing.current.clone();
        merged.position = existing.position;
        merged.updated = existing.updated.clone();
    }
    merged
}

fn progress_put(request: &ReaderRequest, origin: &str, session: &ReaderSession) -> ReaderResponse {
    if request.header("origin") != Some(origin) {
        return response(
            403,
            "Cross-origin progress writes are rejected",
            "text/plain; charset=utf-8",
        );
    }
    // The declared length is checked even when the body was never read, so a
    // chunked/oversized write gets the same answer as a small one.
    if request.content_length.unwrap_or(0) > MAX_PROGRESS_BODY as u64
        || request.body.len() > MAX_PROGRESS_BODY
    {
        return response(
            413,
            "Progress request is too large",
            "text/plain; charset=utf-8",
        );
    }
    let progress: ReadingProgress = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(error) => return response(400, error.to_string(), "text/plain; charset=utf-8"),
    };
    // P2-21: read-merge-write. If the disk state is unreadable (load_progress
    // already quarantined it), keep the writer's payload rather than dropping
    // the update entirely.
    let merged = match crate::progress::load_progress(&session.book_root, &session.manifest) {
        Ok(existing) => merge_progress(&existing, &progress),
        Err(_) => progress,
    };
    match crate::progress::save_progress(&session.book_root, &session.manifest, &merged) {
        Ok(()) => response(204, Vec::new(), "text/plain; charset=utf-8"),
        Err(error) => response(400, error, "text/plain; charset=utf-8"),
    }
}

pub(crate) fn handle(
    request: &ReaderRequest,
    origin: &str,
    sessions: &Sessions,
    reader_html: &Arc<Vec<u8>>,
) -> ReaderResponse {
    let path = request.target.split('?').next().unwrap_or("");
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 3 || parts[0] != "s" {
        return response(404, "Not found", "text/plain; charset=utf-8");
    }
    let session = session_for(sessions, parts[1]);
    let Some(session) = session else {
        return response(403, "Invalid reader session", "text/plain; charset=utf-8");
    };
    let route = parts[2..].join("/");
    match (request.method.as_str(), route.as_str()) {
        ("GET", "reader") => document_response(
            200,
            Arc::clone(reader_html),
            "text/html; charset=utf-8",
        ),
        ("GET", "manifest") => json(&session.manifest),
        ("GET", "progress") => {
            match crate::progress::load_progress(&session.book_root, &session.manifest) {
                Ok(progress) => json(&progress),
                Err(error) => response(500, error, "text/plain; charset=utf-8"),
            }
        }
        ("GET", "heartbeat") => response(204, Vec::new(), "text/plain; charset=utf-8"),
        ("PUT", "progress") => progress_put(request, origin, &session),
        ("GET", value) if value.starts_with("content/") => {
            content_response(&session, value.trim_start_matches("content/"))
        }
        _ => response(404, "Not found", "text/plain; charset=utf-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        insert_session_at, is_book_resource, merge_progress, prune_expired_sessions,
        ReaderSession, Sessions, MAX_READER_SESSIONS, READER_SESSION_TTL,
    };
    use crate::contracts::{Manifest, ReadingProgress};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};

    #[test]
    fn permits_manifest_chapters_and_assets_only() {
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        assert!(is_book_resource("001.md", &manifest));
        assert!(is_book_resource("assets/cover.png", &manifest));
        assert!(!is_book_resource("manifest.json", &manifest));
        assert!(!is_book_resource("private.exe", &manifest));
    }

    #[test]
    fn expires_idle_reader_sessions() {
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        let inserted_at = Instant::now();
        let now = inserted_at + READER_SESSION_TTL + std::time::Duration::from_secs(1);
        let mut sessions = HashMap::new();
        sessions.insert(
            "expired".to_string(),
            (
                ReaderSession {
                    book_root: PathBuf::from("book"),
                    manifest,
                },
                inserted_at,
            ),
        );
        prune_expired_sessions(&mut sessions, now);
        assert!(sessions.is_empty());
    }

    #[test]
    fn insert_sweeps_orphaned_sessions_before_enforcing_the_cap() {
        // P3-20: a crashed webview stops heartbeating — its session must free
        // its slot at the TTL on the next insert instead of wedging the cap.
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        let session = ReaderSession {
            book_root: PathBuf::from("book"),
            manifest,
        };
        let sessions: Sessions = Arc::new(RwLock::new(HashMap::new()));
        let stale = Instant::now();
        for index in 0..MAX_READER_SESSIONS {
            sessions
                .write()
                .expect("session store must lock")
                .insert(format!("stale-{index}"), (session.clone(), stale));
        }
        // A full map of live sessions still rejects the insert.
        assert!(insert_session_at(
            &sessions,
            "fresh".to_string(),
            session.clone(),
            Instant::now(),
        )
        .is_err());
        // Past the TTL the orphans are swept first and the insert succeeds.
        insert_session_at(
            &sessions,
            "fresh".to_string(),
            session,
            stale + READER_SESSION_TTL + Duration::from_secs(1),
        )
        .expect("expired sessions must free their slots");
    }

    #[test]
    fn merge_unions_read_marks_and_prefers_the_fresher_cursor() {
        let progress = |current: &str, position: f64, read: &[&str], updated: &str| {
            ReadingProgress {
                schema_version: 1,
                current: current.to_string(),
                position,
                read: read.iter().map(|id| id.to_string()).collect(),
                updated: updated.to_string(),
            }
        };
        // The reader surface saved later: its cursor wins, but the earlier
        // surface's `read` mark survives the merge.
        let existing = progress("ch-1", 0.2, &["ch-1"], "2026-07-15T10:00:00Z");
        let incoming = progress("ch-2", 0.7, &["ch-2"], "2026-07-15T11:00:00Z");
        let merged = merge_progress(&existing, &incoming);
        assert_eq!(merged.current, "ch-2");
        assert_eq!(merged.position, 0.7);
        assert!(merged.read.iter().any(|id| id == "ch-1"));
        assert!(merged.read.iter().any(|id| id == "ch-2"));
        // Stale cursor, fresh mark: cursor stays with `existing`, reads union.
        let incoming_stale = progress("ch-3", 0.9, &["ch-3"], "2026-07-15T09:00:00Z");
        let merged = merge_progress(&existing, &incoming_stale);
        assert_eq!(merged.current, "ch-1");
        assert_eq!(merged.position, 0.2);
        assert_eq!(merged.read.len(), 2);
    }
}
