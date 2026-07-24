use crate::reader_http::{ReaderSession, Sessions};
use crate::settings::AppSettings;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tiny_http::Server;
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSessionDescriptor {
    pub session_id: String,
    pub url: String,
}

pub struct ReaderServiceState {
    inner: Mutex<Option<ReaderService>>,
}

impl Default for ReaderServiceState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

struct ReaderService {
    origin: String,
    sessions: Sessions,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn load_reader_html() -> Result<String, String> {
    let path = crate::settings::runtime_root()?.join(r"zhihu\app\dist\reader-template.html");
    if !path.is_file() {
        return Err(format!(
            "浏览器 Reader 尚未编译。请先运行 npm run compile-reader：{}",
            path.to_string_lossy()
        ));
    }
    let template = fs::read_to_string(path).map_err(|error| error.to_string())?;
    Ok(template
        .replace("/* ARTICLES_JSON_PLACEHOLDER */", "[]")
        .replace("<!-- ARTICLES_DOM_PLACEHOLDER -->", ""))
}

impl ReaderService {
    fn start(reader_html: String) -> Result<Self, String> {
        let server = Arc::new(Server::http("127.0.0.1:0").map_err(|error| error.to_string())?);
        let address = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| "Reader server did not bind to an IP address".to_string())?;
        let origin = format!("http://127.0.0.1:{}", address.port());
        let sessions: Sessions = Arc::new(RwLock::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_server = Arc::clone(&server);
        let thread_sessions = Arc::clone(&sessions);
        let thread_stop = Arc::clone(&stop);
        let thread_origin = origin.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match thread_server.recv_timeout(Duration::from_millis(200)) {
                    Ok(Some(request)) => {
                        crate::reader_http::handle(
                            request,
                            &thread_origin,
                            &thread_sessions,
                            &reader_html,
                        );
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            origin,
            sessions,
            stop,
            thread: Some(thread),
        })
    }

    fn add_session(&self, session: ReaderSession) -> Result<ReaderSessionDescriptor, String> {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        crate::reader_http::insert_session(&self.sessions, token.clone(), session)?;
        Ok(ReaderSessionDescriptor {
            session_id: token.clone(),
            url: format!("{}/s/{}/reader", self.origin, token),
        })
    }

    fn close_session(&self, session_id: &str) -> Result<bool, String> {
        crate::reader_http::close_session(&self.sessions, session_id)
    }
}

impl Drop for ReaderService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start_session(
    state: &ReaderServiceState,
    settings: &AppSettings,
    book_id: &str,
) -> Result<ReaderSessionDescriptor, String> {
    let (book_root, manifest, _) =
        crate::library::book_context(Path::new(&settings.library_root), book_id)?;
    let mut service = state
        .inner
        .lock()
        .map_err(|_| "Reader service state is unavailable".to_string())?;
    if service.is_none() {
        let html = load_reader_html()?;
        *service = Some(ReaderService::start(html)?);
    }
    service
        .as_ref()
        .ok_or_else(|| "Reader service failed to start".to_string())?
        .add_session(ReaderSession {
            book_root,
            manifest,
        })
}

pub fn close_session(state: &ReaderServiceState, session_id: &str) -> Result<bool, String> {
    let service = state
        .inner
        .lock()
        .map_err(|_| "Reader service state is unavailable".to_string())?;
    service
        .as_ref()
        .ok_or_else(|| "Reader service is not running".to_string())?
        .close_session(session_id)
}

#[cfg(test)]
mod tests {
    use super::ReaderService;
    use crate::contracts::Manifest;
    use crate::reader_http::ReaderSession;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn request(origin: &str, path: &str, headers: &str, body: &str) -> String {
        let address = origin.trim_start_matches("http://");
        let mut stream = TcpStream::connect(address).expect("test server must accept connections");
        let value = format!(
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n{headers}\r\n{body}"
        );
        stream
            .write_all(value.as_bytes())
            .expect("request must write");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response must read");
        response
    }

    fn put(origin: &str, path: &str, include_origin: bool, body: &str) -> String {
        let address = origin.trim_start_matches("http://");
        let origin_header = if include_origin {
            format!("Origin: {origin}\r\n")
        } else {
            String::new()
        };
        let mut stream = TcpStream::connect(address).expect("test server must accept connections");
        let value = format!(
            "PUT {path} HTTP/1.1\r\nHost: {address}\r\n{origin_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(value.as_bytes())
            .expect("request must write");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response must read");
        response
    }

    #[test]
    fn binds_only_to_loopback_on_a_random_port() {
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        assert!(service.origin.starts_with("http://127.0.0.1:"));
        assert!(!service.origin.ends_with(":0"));
    }

    #[test]
    fn caps_open_reader_sessions() {
        let root = std::env::temp_dir().join(format!(
            "immersive-reader-session-cap-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("book root must be created");
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        for _ in 0..crate::reader_http::MAX_READER_SESSIONS {
            service
                .add_session(ReaderSession {
                    book_root: root.clone(),
                    manifest: manifest.clone(),
                })
                .expect("session must fit within the cap");
        }
        assert_eq!(
            service
                .add_session(ReaderSession {
                    book_root: root.clone(),
                    manifest,
                })
                .expect_err("session cap must reject the next session"),
            "READER_SESSION_LIMIT"
        );
        fs::remove_dir_all(root).expect("book root must be removed");
    }

    #[test]
    fn rejects_invalid_tokens_traversal_and_cross_origin_writes() {
        let root = std::env::temp_dir().join(format!("immersive-http-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("book root must be created");
        fs::write(root.join("001.md"), "chapter").expect("chapter must be written");
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        let descriptor = service
            .add_session(ReaderSession {
                book_root: root.clone(),
                manifest,
            })
            .expect("session must start");
        let token = descriptor.session_id;
        assert!(request(&service.origin, "/s/invalid/manifest", "", "").starts_with("HTTP/1.1 403"));
        assert!(request(
            &service.origin,
            &format!("/s/{token}/heartbeat"),
            "",
            ""
        )
        .starts_with("HTTP/1.1 204"));
        assert!(request(
            &service.origin,
            &format!("/s/{token}/content/%2e%2e/settings.json"),
            "",
            ""
        )
        .starts_with("HTTP/1.1 403"));
        let progress = include_str!("../../../../packages/contracts/fixtures/reading.valid.json");
        assert!(put(
            &service.origin,
            &format!("/s/{token}/progress"),
            false,
            progress
        )
        .starts_with("HTTP/1.1 403"));
        assert!(put(
            &service.origin,
            &format!("/s/{token}/progress"),
            true,
            progress
        )
        .starts_with("HTTP/1.1 204"));
        assert!(service.close_session(&token).expect("session must close"));
        assert!(
            request(&service.origin, &format!("/s/{token}/manifest"), "", "")
                .starts_with("HTTP/1.1 403")
        );
        assert!(root.join(".reading.json").exists());
        fs::remove_dir_all(root).expect("book root must be removed");
    }
}
