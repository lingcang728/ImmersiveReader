use crate::contracts::{is_safe_relative_path, Manifest, ReadingProgress};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tiny_http::{Header, Method, Request, Response, StatusCode};

const MAX_PROGRESS_BODY: usize = 64 * 1024;

#[derive(Clone)]
pub struct ReaderSession {
    pub book_root: PathBuf,
    pub manifest: Manifest,
}

pub type Sessions = Arc<RwLock<HashMap<String, ReaderSession>>>;
type HttpResponse = Response<Cursor<Vec<u8>>>;

fn response(status: u16, body: impl Into<Vec<u8>>, content_type: &str) -> HttpResponse {
    let mut value = Response::from_data(body).with_status_code(StatusCode(status));
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
    match fs::read(&canonical_file) {
        Ok(data) => response(200, data, mime_type(&canonical_file)),
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
    let session = sessions
        .read()
        .ok()
        .and_then(|items| items.get(parts[1]).cloned());
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
    use super::is_book_resource;
    use crate::contracts::Manifest;

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
}
