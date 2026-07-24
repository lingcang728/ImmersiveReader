use crate::contracts::{is_safe_relative_path, Manifest, ReadingProgress};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, ResponseBox, StatusCode};

const MAX_PROGRESS_BODY: usize = 64 * 1024;
pub(crate) const MAX_READER_SESSIONS: usize = 16;
pub(crate) const READER_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub struct ReaderSession {
    pub book_root: PathBuf,
    pub manifest: Manifest,
}

pub type Sessions = Arc<RwLock<HashMap<String, (ReaderSession, Instant)>>>;
type HttpResponse = ResponseBox;

fn prune_expired_sessions(sessions: &mut HashMap<String, (ReaderSession, Instant)>, now: Instant) {
    sessions.retain(|_, (_, last_access)| now.duration_since(*last_access) < READER_SESSION_TTL);
}

pub fn insert_session(
    sessions: &Sessions,
    token: String,
    session: ReaderSession,
) -> Result<(), String> {
    let mut sessions = sessions
        .write()
        .map_err(|_| "Reader session store is unavailable".to_string())?;
    let now = Instant::now();
    prune_expired_sessions(&mut sessions, now);
    if sessions.len() >= MAX_READER_SESSIONS {
        return Err("READER_SESSION_LIMIT".to_string());
    }
    sessions.insert(token, (session, now));
    Ok(())
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

fn add_common_headers<R: Read>(mut value: Response<R>, content_type: &str) -> Response<R> {
    if let Ok(header) = Header::from_bytes("Content-Type", content_type) {
        value.add_header(header);
    }
    if let Ok(header) = Header::from_bytes("X-Content-Type-Options", "nosniff") {
        value.add_header(header);
    }
    if let Ok(header) = Header::from_bytes("Referrer-Policy", "no-referrer") {
        value.add_header(header);
    }
    value
}

fn response(status: u16, body: impl Into<Vec<u8>>, content_type: &str) -> HttpResponse {
    add_common_headers(
        Response::from_data(body).with_status_code(StatusCode(status)),
        content_type,
    )
    .boxed()
}

fn file_response(file: fs::File, content_type: &str) -> HttpResponse {
    add_common_headers(Response::from_file(file), content_type).boxed()
}

fn json<T: serde::Serialize>(value: &T) -> HttpResponse {
    match serde_json::to_vec(value) {
        Ok(body) => response(200, body, "application/json; charset=utf-8"),
        Err(error) => response(500, error.to_string(), "text/plain; charset=utf-8"),
    }
}

fn request_origin(request: &Request) -> Option<&str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Origin"))
        .map(|header| header.value.as_str())
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

fn content_response(session: &ReaderSession, raw_relative: &str) -> HttpResponse {
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

fn progress_put(request: &mut Request, origin: &str, session: &ReaderSession) -> HttpResponse {
    if request_origin(request) != Some(origin) {
        return response(
            403,
            "Cross-origin progress writes are rejected",
            "text/plain; charset=utf-8",
        );
    }
    if request.body_length().unwrap_or(0) > MAX_PROGRESS_BODY {
        return response(
            413,
            "Progress request is too large",
            "text/plain; charset=utf-8",
        );
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_PROGRESS_BODY + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return response(
            400,
            "Progress request could not be read",
            "text/plain; charset=utf-8",
        );
    }
    if body.len() > MAX_PROGRESS_BODY {
        return response(
            413,
            "Progress request is too large",
            "text/plain; charset=utf-8",
        );
    }
    let progress: ReadingProgress = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => return response(400, error.to_string(), "text/plain; charset=utf-8"),
    };
    match crate::progress::save_progress(&session.book_root, &session.manifest, &progress) {
        Ok(()) => response(204, Vec::new(), "text/plain; charset=utf-8"),
        Err(error) => response(400, error, "text/plain; charset=utf-8"),
    }
}

pub fn handle(mut request: Request, origin: &str, sessions: &Sessions, reader_html: &str) {
    let path = request.url().split('?').next().unwrap_or("");
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 3 || parts[0] != "s" {
        let _ = request.respond(response(404, "Not found", "text/plain; charset=utf-8"));
        return;
    }
    let session = session_for(sessions, parts[1]);
    let Some(session) = session else {
        let _ = request.respond(response(
            403,
            "Invalid reader session",
            "text/plain; charset=utf-8",
        ));
        return;
    };
    let route = parts[2..].join("/");
    let value = match (request.method(), route.as_str()) {
        (&Method::Get, "reader") => {
            response(200, reader_html.as_bytes(), "text/html; charset=utf-8")
        }
        (&Method::Get, "manifest") => json(&session.manifest),
        (&Method::Get, "progress") => {
            match crate::progress::load_progress(&session.book_root, &session.manifest) {
                Ok(progress) => json(&progress),
                Err(error) => response(500, error, "text/plain; charset=utf-8"),
            }
        }
        (&Method::Get, "heartbeat") => response(204, Vec::new(), "text/plain; charset=utf-8"),
        (&Method::Put, "progress") => progress_put(&mut request, origin, &session),
        (&Method::Get, value) if value.starts_with("content/") => {
            content_response(&session, value.trim_start_matches("content/"))
        }
        _ => response(404, "Not found", "text/plain; charset=utf-8"),
    };
    let _ = request.respond(value);
}

#[cfg(test)]
mod tests {
    use super::{is_book_resource, prune_expired_sessions, ReaderSession, READER_SESSION_TTL};
    use crate::contracts::Manifest;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Instant;

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
}
