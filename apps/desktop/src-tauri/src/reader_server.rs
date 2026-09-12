use crate::reader_http::{ReaderBody, ReaderRequest, ReaderResponse, ReaderSession, Sessions};
use crate::settings::AppSettings;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// P2-15/P2-23: the reader binds only to loopback and only to this fixed
/// candidate list. CSP has no wildcard port ranges, so every port here is
/// named individually in `tauri.conf.json` `frame-src` — keep the two lists
/// in sync. A port that is already taken falls through to the next entry.
pub(crate) const READER_PORTS: &[u16] = &[
    21743, 21744, 21745, 21746, 21747, 21748, 21749, 21750,
];

/// Accept-loop poll interval when the nonblocking listener reports
/// `WouldBlock`; also bounds how quickly `stop` is observed on drop.
const ACCEPT_POLL: Duration = Duration::from_millis(25);
/// Concurrent connections being parsed/handled/written at once. Each holds
/// exactly one worker thread for the life of the connection.
const WORKER_THREADS: usize = 8;
/// Accepted sockets waiting for a free worker. Past this cap new connections
/// are dropped immediately — the kernel still accepts them, the client sees
/// a reset and retries; nothing queues unboundedly in-process.
const PENDING_CONNECTIONS: usize = 32;
/// Per-syscall bounds. A client that stalls mid-read or stops draining the
/// response can pin one worker for at most one timeout window per op.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Whole-connection budget — covers header drip-feeds and slow readers that
/// stay under each per-call timeout but would otherwise pin a worker
/// indefinitely.
const CONNECTION_BUDGET: Duration = Duration::from_secs(60);
/// Hard caps on the head block. Reader traffic is tiny; these exist only to
/// keep a hostile or broken peer from buffering forever.
const MAX_REQUEST_LINE: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
/// Bounded wait for the accept thread during `Drop` — it polls the stop flag
/// every ACCEPT_POLL, so this never approaches the timeout in practice.
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);

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

/// Bounded hand-off between the accept loop and the worker pool. A closed
/// queue drains its backlog then lets workers exit; a full queue rejects new
/// connections instead of growing memory or threads without limit.
struct WorkQueue {
    pending: Mutex<VecDeque<TcpStream>>,
    available: Condvar,
    closed: AtomicBool,
    capacity: usize,
}

impl WorkQueue {
    fn new(capacity: usize) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            available: Condvar::new(),
            closed: AtomicBool::new(false),
            capacity,
        }
    }

    /// Takes ownership of the socket; a full or closed queue drops it, which
    /// closes the connection without a response.
    fn push(&self, stream: TcpStream) {
        let accepted = !self.closed.load(Ordering::SeqCst)
            && self
                .pending
                .lock()
                .map(|mut pending| {
                    if pending.len() >= self.capacity {
                        false
                    } else {
                        pending.push_back(stream);
                        true
                    }
                })
                .unwrap_or(false);
        if accepted {
            self.available.notify_one();
        }
    }

    fn pop(&self) -> Option<TcpStream> {
        let mut pending = self.pending.lock().ok()?;
        loop {
            if let Some(stream) = pending.pop_front() {
                return Some(stream);
            }
            if self.closed.load(Ordering::SeqCst) {
                return None;
            }
            pending = self.available.wait(pending).ok()?;
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.available.notify_all();
    }
}

struct ReaderService {
    origin: String,
    sessions: Sessions,
    stop: Arc<AtomicBool>,
    work: Arc<WorkQueue>,
    accept_thread: Option<JoinHandle<()>>,
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

/// Reads one line (terminated by `\n`) with a byte cap and a wall-clock
/// deadline. `Ok(None)` means the peer closed before sending anything —
/// an idle accepted connection ends quietly instead of holding a worker.
fn read_line_bounded(
    reader: &mut BufReader<TcpStream>,
    cap: usize,
    deadline: Instant,
) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "connection budget"));
        }
        if line.len() > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "header line exceeds limit",
            ));
        }
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed mid-line",
                ))
            };
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                line.extend_from_slice(&available[..=index]);
                reader.consume(index + 1);
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                return Ok(Some(line));
            }
            None => {
                let consumed = available.len();
                line.extend_from_slice(available);
                reader.consume(consumed);
            }
        }
    }
}

/// Parses the request line and headers, honors `Expect: 100-continue`, then
/// reads the Content-Length body when it fits the cap. Oversized or absent
/// bodies are not consumed — every response ends with `Connection: close`,
/// so unread bytes die with the socket.
fn read_request(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    deadline: Instant,
) -> io::Result<Option<ReaderRequest>> {
    let Some(request_line) = read_line_bounded(reader, MAX_REQUEST_LINE, deadline)? else {
        return Ok(None);
    };
    let request_line = String::from_utf8_lossy(&request_line);
    let mut parts = request_line.split_whitespace();
    let (method, target, version) = match (parts.next(), parts.next(), parts.next()) {
        (Some(method), Some(target), Some(version)) => {
            (method.to_string(), target.to_string(), version.to_string())
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed request line",
            ))
        }
    };
    if !version.starts_with("HTTP/1") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported HTTP version",
        ));
    }

    let mut headers = Vec::new();
    let mut header_bytes = request_line.len();
    loop {
        let Some(line) = read_line_bounded(reader, MAX_REQUEST_LINE, deadline)? else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed inside headers",
            ));
        };
        header_bytes += line.len();
        if header_bytes > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "header block exceeds limit",
            ));
        }
        if line.is_empty() {
            break;
        }
        if let Some(separator) = line.iter().position(|byte| *byte == b':') {
            let name = String::from_utf8_lossy(&line[..separator])
                .trim()
                .to_ascii_lowercase();
            let value = String::from_utf8_lossy(&line[separator + 1..])
                .trim()
                .to_string();
            if !name.is_empty() {
                headers.push((name, value));
            }
        }
    }

    let header_value = |name: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if header_value("expect").is_some_and(|value| value.eq_ignore_ascii_case("100-continue")) {
        // Unblocks fetch clients that wait for the interim response before
        // streaming the body. Write failures just end the connection.
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let content_length = match header_value("content-length") {
        Some(raw) => Some(
            raw.parse::<u64>()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad content-length"))?,
        ),
        None => None,
    };
    let mut body = Vec::new();
    if let Some(length) = content_length {
        // Only read a body that fits the cap; larger advertised bodies are
        // left unread and the route still sees the declared length (→ 413).
        if length <= crate::reader_http::MAX_REQUEST_BODY {
            body.reserve(length as usize);
            while (body.len() as u64) < length {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "connection budget"));
                }
                let mut chunk = [0_u8; 16 * 1024];
                let want = (length - body.len() as u64).min(chunk.len() as u64) as usize;
                let read = reader.read(&mut chunk[..want])?;
                if read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed inside body",
                    ));
                }
                body.extend_from_slice(&chunk[..read]);
            }
        }
    }
    Ok(Some(ReaderRequest {
        method,
        target,
        headers,
        content_length,
        body,
    }))
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

/// Writes the full response under the socket write timeout and the
/// per-connection budget — a peer that stops draining can pin this worker
/// for at most [`CONNECTION_BUDGET`], never the whole server.
fn write_response(
    writer: &mut TcpStream,
    response: ReaderResponse,
    deadline: Instant,
) -> io::Result<()> {
    let body_length = response
        .body
        .len()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "body length unavailable"))?;
    let mut head = String::with_capacity(256);
    head.push_str(&format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason_phrase(response.status)
    ));
    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str(&format!("Content-Length: {body_length}\r\n"));
    head.push_str("Connection: close\r\n\r\n");
    writer.write_all(head.as_bytes())?;
    match response.body {
        ReaderBody::Bytes(body) => writer.write_all(&body)?,
        ReaderBody::Shared(body) => writer.write_all(&body)?,
        ReaderBody::File(mut file) => {
            let mut chunk = [0_u8; 64 * 1024];
            loop {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "connection budget"));
                }
                let read = file.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                writer.write_all(&chunk[..read])?;
            }
        }
    }
    writer.flush()
}

/// One worker turn: parse a single request, route it, respond, close.
/// `Connection: close` on every reply keeps workers un-pinned — no
/// keep-alive connection can hold a worker between requests.
fn serve_connection(
    stream: TcpStream,
    origin: &str,
    sessions: &Sessions,
    reader_html: &Arc<Vec<u8>>,
) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let deadline = Instant::now() + CONNECTION_BUDGET;
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let request = match read_request(&mut reader, &mut writer, deadline) {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(_) => {
            let _ = write_response(
                &mut writer,
                ReaderResponse {
                    status: 400,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/plain; charset=utf-8".to_string(),
                    )],
                    body: ReaderBody::Bytes(b"Bad request".to_vec()),
                },
                deadline,
            );
            return;
        }
    };
    let response = crate::reader_http::handle(&request, origin, sessions, reader_html);
    let _ = write_response(&mut writer, response, deadline);
}

impl ReaderService {
    fn start(reader_html: String) -> Result<Self, String> {
        let mut listener = None;
        for port in READER_PORTS {
            match TcpListener::bind(("127.0.0.1", *port)) {
                Ok(bound) => {
                    listener = Some(bound);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        let listener = listener.ok_or_else(|| {
            "READER_PORT_BUSY: every 连读 port in the fixed list is taken".to_string()
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        if !address.ip().is_loopback() {
            return Err("Reader server did not bind to loopback".to_string());
        }
        let origin = format!("http://127.0.0.1:{}", address.port());
        let sessions: Sessions = Arc::new(RwLock::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let work = Arc::new(WorkQueue::new(PENDING_CONNECTIONS));
        let reader_html = Arc::new(reader_html.into_bytes());

        // Fixed worker pool: one connection per worker at a time, so N
        // stalled or hostile peers can never consume more than N threads.
        for index in 0..WORKER_THREADS {
            let worker_work = Arc::clone(&work);
            let worker_sessions = Arc::clone(&sessions);
            let worker_html = Arc::clone(&reader_html);
            let worker_origin = origin.clone();
            if let Err(error) = thread::Builder::new()
                .name(format!("reader-worker-{index}"))
                .spawn(move || {
                    while let Some(stream) = worker_work.pop() {
                        serve_connection(stream, &worker_origin, &worker_sessions, &worker_html);
                    }
                })
            {
                // A half-spawned pool must not leak workers blocked in pop().
                work.close();
                return Err(error.to_string());
            }
        }

        let accept_work = Arc::clone(&work);
        let accept_stop = Arc::clone(&stop);
        let accept_thread = thread::Builder::new()
            .name("reader-accept".to_string())
            .spawn(move || {
                while !accept_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Bounded queue: on overflow the socket is
                            // dropped inside push and the client sees a
                            // reset instead of an unbounded backlog.
                            accept_work.push(stream);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(ACCEPT_POLL);
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            origin,
            sessions,
            stop,
            work,
            accept_thread: Some(accept_thread),
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
        // Workers finish their in-flight connection (bounded by the socket
        // timeouts + connection budget) then exit on the closed queue — they
        // are deliberately not joined here.
        self.work.close();
        if let Some(thread) = self.accept_thread.take() {
            // Bounded join: the nonblocking accept loop observes `stop`
            // within ACCEPT_POLL, but a scheduler hiccup must never freeze
            // the dropping thread.
            let (done_tx, done_rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = thread.join();
                let _ = done_tx.send(());
            });
            let _ = done_rx.recv_timeout(JOIN_TIMEOUT);
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
    use std::time::Duration;

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

    fn fixture_manifest() -> Manifest {
        serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize")
    }

    #[test]
    fn binds_only_to_loopback_on_a_listed_port() {
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        assert!(service.origin.starts_with("http://127.0.0.1:"));
        let port: u16 = service
            .origin
            .rsplit(':')
            .next()
            .and_then(|part| part.parse().ok())
            .expect("origin must carry a port");
        assert!(
            super::READER_PORTS.contains(&port),
            "reader port {port} must come from the CSP-listed candidates"
        );
    }

    #[test]
    fn stalled_connection_does_not_starve_other_requests() {
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        let address = service.origin.trim_start_matches("http://").to_string();
        // Hold a connection with a half-written request — it pins one worker
        // until the read timeout, but the pool must keep answering.
        let mut stalled = TcpStream::connect(&address).expect("stalled client must connect");
        stalled
            .write_all(b"GET /s/whatever/reader HTTP/1.1\r\nHost: x\r\n")
            .expect("partial request must write");
        stalled
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout must set");
        let response = request(&service.origin, "/s/invalid/manifest", "", "");
        assert!(response.starts_with("HTTP/1.1 403"));
        drop(stalled);
    }

    #[test]
    fn malformed_request_gets_400_and_close() {
        let service =
            ReaderService::start("<html></html>".to_string()).expect("reader service must start");
        let address = service.origin.trim_start_matches("http://").to_string();
        let mut stream = TcpStream::connect(&address).expect("client must connect");
        stream
            .write_all(b"GARBAGE\r\n\r\n")
            .expect("garbage request must write");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout must set");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response must read");
        assert!(response.starts_with("HTTP/1.1 400"));
    }

    #[test]
    fn reader_document_carries_a_csp_header() {
        let root = std::env::temp_dir().join(format!("immersive-csp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("book root must be created");
        let service = ReaderService::start("<html><body>reader</body></html>".to_string())
            .expect("reader service must start");
        let descriptor = service
            .add_session(ReaderSession {
                book_root: root.clone(),
                manifest: fixture_manifest(),
            })
            .expect("session must start");
        let token = descriptor.session_id;
        let page = request(&service.origin, &format!("/s/{token}/reader"), "", "");
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("Content-Security-Policy: default-src 'none'"));
        assert!(page.contains("frame-ancestors"));
        assert!(page.contains("<body>reader</body>"));
        fs::remove_dir_all(root).expect("book root must be removed");
    }

    #[test]
    fn caps_open_reader_sessions() {
        let root = std::env::temp_dir().join(format!(
            "immersive-reader-session-cap-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("book root must be created");
        let manifest = fixture_manifest();
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
        let manifest = fixture_manifest();
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
