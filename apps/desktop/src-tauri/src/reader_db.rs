//! Reader-local SQLite store (`Data\App\reader.db`).
//!
//! Holds the reading-side data that is per-book but not part of the shared
//! manifest/reading contracts: the book-id → library directory index (C5,
//! so `find_book` does not have to rescan the whole shelf), the last-read
//! locator per book, user bookmarks, and the FTS chapter index powering
//! `search_book`. Follows the `control.rs` playbook: WAL journal, foreign
//! keys on, schema gated by `PRAGMA user_version`, and a corrupt/NOTADB file
//! is quarantined to `reader.db.corrupt-<epoch>` (plus -wal/-shm sidecars)
//! then rebuilt empty instead of failing forever.

use rusqlite::{params, Connection, Error as SqliteError, ErrorCode, OptionalExtension};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Schema bootstrap runs only while `PRAGMA user_version` is below this; a
/// database stamped by a NEWER build is refused, never downgraded — the same
/// rule control.rs applies (`version <`, not `!=`).
pub(crate) const READER_SCHEMA_VERSION: u32 = 1;

/// One bookmark row as returned to the frontend (`list_bookmarks`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkRow {
    pub bookmark_id: String,
    pub locator_json: String,
    pub label: String,
    pub created_at: String,
}

/// One full-text search hit inside a book.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub chapter_id: String,
    pub title: String,
    pub snippet: String,
}

/// Merge counts reported by `import_state` (bundle restore).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderDbMergeReport {
    pub books: u32,
    pub locators: u32,
    pub bookmarks: u32,
}

pub struct ReaderDb {
    conn: Connection,
    /// True when the FTS5 module is present (always is under rusqlite's
    /// bundled SQLite, but a non-bundled build must still work — search then
    /// degrades to LIKE over `chapter_texts`).
    fts5: bool,
}

/// Mirrors `control.rs::is_corrupt_database` — NOTADB/CORRUPT primary codes
/// (extended SQLITE_CORRUPT_* codes map to the same primary) or the familiar
/// message strings. BUSY/LOCKED stay transient.
fn is_corrupt_database(error: &SqliteError) -> bool {
    let SqliteError::SqliteFailure(failure, message) = error else {
        return false;
    };
    if matches!(
        failure.code,
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
    ) {
        return false;
    }
    if matches!(
        failure.code,
        ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
    ) {
        return true;
    }
    let message = message.as_deref().unwrap_or_default().to_ascii_lowercase();
    message.contains("not a database")
        || message.contains("malformed")
        || message.contains("corrupt")
}

fn database_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Rename a damaged reader.db — plus its `-wal`/`-shm` sidecars — to
/// `*.corrupt-<epoch>` so the next open rebuilds an empty schema. Same
/// recipe as `control.rs::quarantine_corrupt_database`; a `-N` retry suffix
/// keeps repeated corruption from colliding on one name.
fn quarantine_corrupt_database(path: &Path) -> Result<(), String> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    for candidate in [
        path.to_path_buf(),
        database_sidecar_path(path, "-wal"),
        database_sidecar_path(path, "-shm"),
    ] {
        if !candidate.exists() {
            continue;
        }
        let file_name = candidate
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "reader.db".to_string());
        let mut renamed = false;
        for attempt in 0..10_u32 {
            let suffix = if attempt == 0 {
                format!("corrupt-{epoch}")
            } else {
                format!("corrupt-{epoch}-{attempt}")
            };
            let backup = candidate.with_file_name(format!("{file_name}.{suffix}"));
            match fs::rename(&candidate, &backup) {
                Ok(()) => {
                    renamed = true;
                    break;
                }
                Err(_) if backup.exists() => continue,
                Err(error) => {
                    return Err(format!(
                        "failed to quarantine corrupt database {}: {error}",
                        candidate.display()
                    ));
                }
            }
        }
        if !renamed {
            return Err(format!(
                "failed to quarantine corrupt database {}",
                candidate.display()
            ));
        }
    }
    Ok(())
}

fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl ReaderDb {
    /// Open the managed `Data\App\reader.db`. Component joins only — never a
    /// raw `r"App\..."` literal, so the same code path works on Android.
    pub fn open_current() -> Result<Self, String> {
        let locations = crate::storage::StorageLocations::current()?;
        Self::open_at(&locations.data_root.join("App").join("reader.db"))
    }

    /// Path-explicit open used by tests (never touches managed roots).
    pub(crate) fn open_at(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, true)
    }

    fn open_inner(path: &Path, quarantine_corrupt: bool) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Reader database has no parent directory".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let connection = match Connection::open(path) {
            Ok(connection) => connection,
            Err(error) => {
                if quarantine_corrupt && is_corrupt_database(&error) {
                    quarantine_corrupt_database(path)?;
                    crate::storage::app_log(
                        "reader_db",
                        "reader.db was corrupt; quarantined and rebuilt empty",
                    );
                    return Self::open_inner(path, false);
                }
                return Err(error.to_string());
            }
        };
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        // Per-connection pragmas (WAL is persistent but re-asserted;
        // foreign_keys is connection-scoped). This is also where "file is
        // not a database" first surfaces on a corrupt file.
        let preamble = r#"
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;
                "#;
        let schema = r#"
                CREATE TABLE IF NOT EXISTS books (
                  book_id TEXT PRIMARY KEY NOT NULL,
                  dir_rel TEXT NOT NULL,
                  source TEXT,
                  format TEXT,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS reader_locators (
                  book_id TEXT PRIMARY KEY NOT NULL,
                  locator_json TEXT,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS bookmarks (
                  book_id TEXT NOT NULL,
                  bookmark_id TEXT PRIMARY KEY NOT NULL,
                  locator_json TEXT,
                  label TEXT,
                  created_at TEXT NOT NULL
                );
                -- Canonical chapter text store: also the LIKE fallback when
                -- FTS5 is missing, and the audit copy the FTS index is
                -- rebuilt from.
                CREATE TABLE IF NOT EXISTS chapter_texts (
                  book_id TEXT NOT NULL,
                  chapter_id TEXT NOT NULL,
                  title TEXT,
                  body TEXT,
                  PRIMARY KEY (book_id, chapter_id)
                );
                "#;
        let bootstrap = (|| -> Result<u32, SqliteError> {
            connection.execute_batch(preamble)?;
            let version: u32 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if version < READER_SCHEMA_VERSION {
                connection.execute_batch(schema)?;
                connection
                    .execute_batch(&format!("PRAGMA user_version = {READER_SCHEMA_VERSION}"))?;
            }
            Ok(version)
        })();
        let version = match bootstrap {
            Ok(version) => version,
            Err(error) => {
                if quarantine_corrupt && is_corrupt_database(&error) {
                    drop(connection);
                    quarantine_corrupt_database(path)?;
                    crate::storage::app_log(
                        "reader_db",
                        "reader.db was corrupt; quarantined and rebuilt empty",
                    );
                    return Self::open_inner(path, false);
                }
                return Err(error.to_string());
            }
        };
        if version > READER_SCHEMA_VERSION {
            return Err(format!(
                "READER_SCHEMA_TOO_NEW: reader.db is schema {version}, newer than this build's {READER_SCHEMA_VERSION} — update the app instead of opening it"
            ));
        }
        // FTS5 is probed on every open (the IF NOT EXISTS keeps it cheap);
        // a build without the module degrades to the LIKE path. `trigram`
        // (SQLite ≥3.34, bundled) is deliberate: unicode61 indexes each CJK
        // run as one giant token, so `阅读` could never match — trigram
        // indexes 3-char substrings and answers real Chinese word queries
        // plus mid-word Latin matches. Terms shorter than 3 chars still
        // route to LIKE (see `fts_search`).
        let fts5 = match connection.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chapter_fts USING fts5(book_id UNINDEXED, chapter_id UNINDEXED, title, body, tokenize='trigram')",
        ) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("reader.db: FTS5 unavailable ({error}); search falls back to LIKE");
                crate::storage::app_log(
                    "reader_db",
                    &format!("FTS5 unavailable, search uses LIKE fallback: {error}"),
                );
                false
            }
        };
        Ok(Self {
            conn: connection,
            fts5,
        })
    }

    /// Index/refresh one book's id → library-relative directory mapping.
    /// `dir_rel` is the `/`-separated path from the library root to the book
    /// dir (the value `find_book_indexed` re-attaches to the root).
    pub fn upsert_book(
        &self,
        book_id: &str,
        dir_rel: &str,
        source: &str,
        format: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO books(book_id, dir_rel, source, format, updated_at) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(book_id) DO UPDATE SET dir_rel = excluded.dir_rel, source = excluded.source, format = excluded.format, updated_at = excluded.updated_at",
                params![book_id, dir_rel, source, format, now_stamp()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Drop every row the book owns across all four tables — index entry,
    /// locator, bookmarks and the chapter text/FTS bodies.
    pub fn remove_book(&self, book_id: &str) -> Result<(), String> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM books WHERE book_id = ?1", params![book_id])
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM reader_locators WHERE book_id = ?1",
                params![book_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM bookmarks WHERE book_id = ?1", params![book_id])
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM chapter_texts WHERE book_id = ?1",
                params![book_id],
            )
            .map_err(|error| error.to_string())?;
        if self.fts5 {
            transaction
                .execute(
                    "DELETE FROM chapter_fts WHERE book_id = ?1",
                    params![book_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    /// Drop the `books` index row plus the indexed chapter bodies — the
    /// trash path uses this so a recoverable book keeps its
    /// locator/bookmarks while `find_book_indexed` stops short-circuiting to
    /// a shelf dir that no longer exists and `search_book` no longer answers
    /// with stale text. One transaction so a crash mid-way cannot leave an
    /// indexed-but-unlisted book.
    pub fn unindex_book(&self, book_id: &str) -> Result<(), String> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM books WHERE book_id = ?1", params![book_id])
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM chapter_texts WHERE book_id = ?1",
                params![book_id],
            )
            .map_err(|error| error.to_string())?;
        if self.fts5 {
            transaction
                .execute(
                    "DELETE FROM chapter_fts WHERE book_id = ?1",
                    params![book_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    /// The indexed library-relative directory of `book_id`, if any.
    pub fn book_dir(&self, book_id: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT dir_rel FROM books WHERE book_id = ?1",
                params![book_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// `(book_id, dir_rel, source, format)` for every indexed book — used by
    /// the reconcile pass that drops index rows pointing at vanished dirs.
    pub fn all_books(&self) -> Result<Vec<(String, String, String, String)>, String> {
        let mut statement = self
            .conn
            .prepare("SELECT book_id, dir_rel, source, format FROM books")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn save_locator(&self, book_id: &str, locator_json: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO reader_locators(book_id, locator_json, updated_at) VALUES(?1, ?2, ?3)
                 ON CONFLICT(book_id) DO UPDATE SET locator_json = excluded.locator_json, updated_at = excluded.updated_at",
                params![book_id, locator_json, now_stamp()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn load_locator(&self, book_id: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT locator_json FROM reader_locators WHERE book_id = ?1",
                params![book_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn add_bookmark(
        &self,
        book_id: &str,
        bookmark_id: &str,
        locator_json: &str,
        label: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO bookmarks(book_id, bookmark_id, locator_json, label, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![book_id, bookmark_id, locator_json, label, now_stamp()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn remove_bookmark(&self, book_id: &str, bookmark_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM bookmarks WHERE book_id = ?1 AND bookmark_id = ?2",
                params![book_id, bookmark_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_bookmarks(&self, book_id: &str) -> Result<Vec<BookmarkRow>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT bookmark_id, locator_json, label, created_at FROM bookmarks WHERE book_id = ?1 ORDER BY created_at, rowid",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![book_id], |row| {
                Ok(BookmarkRow {
                    bookmark_id: row.get(0)?,
                    locator_json: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    label: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    created_at: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// Replace one chapter's searchable text (canonical store + FTS index).
    pub fn fts_replace_chapter(
        &self,
        book_id: &str,
        chapter_id: &str,
        title: &str,
        text: &str,
    ) -> Result<(), String> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO chapter_texts(book_id, chapter_id, title, body) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(book_id, chapter_id) DO UPDATE SET title = excluded.title, body = excluded.body",
                params![book_id, chapter_id, title, text],
            )
            .map_err(|error| error.to_string())?;
        if self.fts5 {
            transaction
                .execute(
                    "DELETE FROM chapter_fts WHERE book_id = ?1 AND chapter_id = ?2",
                    params![book_id, chapter_id],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO chapter_fts(book_id, chapter_id, title, body) VALUES(?1, ?2, ?3, ?4)",
                    params![book_id, chapter_id, title, text],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    /// True when at least one chapter body is stored for `book_id` — lets
    /// `search_book` distinguish "not indexed yet" from "no hits" so an
    /// already-indexed book does not get re-indexed on every empty search.
    pub fn fts_book_indexed(&self, book_id: &str) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT 1 FROM chapter_texts WHERE book_id = ?1 LIMIT 1",
                params![book_id],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|error| error.to_string())
    }

    /// Raw user text → FTS5 MATCH expression: each whitespace-separated term
    /// becomes a quoted phrase (embedded quotes doubled), joined by spaces —
    /// implicit AND over all terms, immune to FTS operator injection.
    fn fts_query(query: &str) -> String {
        query
            .split_whitespace()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn search_like(
        &self,
        book_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>, String> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT chapter_id, title, body FROM chapter_texts WHERE book_id = ?1 AND (title LIKE ?2 ESCAPE '\\' OR body LIKE ?2 ESCAPE '\\') LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let rows = statement
            .query_map(params![book_id, pattern, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut hits = Vec::new();
        for row in rows {
            let (chapter_id, title, body) = row.map_err(|error| error.to_string())?;
            hits.push(SearchHit {
                chapter_id,
                title,
                snippet: like_snippet(&body, query),
            });
        }
        Ok(hits)
    }

    /// FTS5 `MATCH` over title+body with a `snippet(...)` extract; degrades
    /// to LIKE when the module is missing or the query errors (paranoid —
    /// `fts_query` already neutralizes operator syntax).
    pub fn fts_search(
        &self,
        book_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>, String> {
        let trimmed = query.trim();
        if trimmed.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(200);
        // Trigram MATCH can only answer terms ≥3 chars — a shorter term
        // tokenizes to nothing under `trigram` and would either error or
        // match vacuously. LIKE's plain substring covers those queries.
        let fts_usable = self.fts5
            && trimmed
                .split_whitespace()
                .all(|term| term.chars().count() >= 3);
        if fts_usable {
            let result = (|| -> Result<Vec<SearchHit>, String> {
                let mut statement = self
                    .conn
                    .prepare(
                        "SELECT chapter_id, title, snippet(chapter_fts, 3, '<b>', '</b>', '…', 10) FROM chapter_fts WHERE chapter_fts MATCH ?1 AND book_id = ?2 ORDER BY rank LIMIT ?3",
                    )
                    .map_err(|error| error.to_string())?;
                let rows = statement
                    .query_map(params![Self::fts_query(trimmed), book_id, limit], |row| {
                        Ok(SearchHit {
                            chapter_id: row.get(0)?,
                            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            snippet: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        })
                    })
                    .map_err(|error| error.to_string())?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())
            })();
            match result {
                Ok(hits) => return Ok(hits),
                Err(error) => {
                    eprintln!("reader.db: FTS query failed ({error}); falling back to LIKE");
                }
            }
        }
        self.search_like(book_id, trimmed, limit)
    }

    /// Serialize books + locators + bookmarks as JSON for the export bundle.
    /// Chapter text bodies stay out — the FTS index is rebuilt on demand.
    pub fn export_state(&self) -> Result<serde_json::Value, String> {
        let mut books = Vec::new();
        for (book_id, dir_rel, source, format) in self.all_books()? {
            books.push(serde_json::json!({
                "bookId": book_id,
                "dirRel": dir_rel,
                "source": source,
                "format": format,
            }));
        }
        let mut locators = Vec::new();
        {
            let mut statement = self
                .conn
                .prepare("SELECT book_id, locator_json, updated_at FROM reader_locators")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "bookId": row.get::<_, String>(0)?,
                        "locatorJson": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        "updatedAt": row.get::<_, String>(2)?,
                    }))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                locators.push(row.map_err(|error| error.to_string())?);
            }
        }
        let mut bookmarks = Vec::new();
        {
            let mut statement = self
                .conn
                .prepare(
                    "SELECT book_id, bookmark_id, locator_json, label, created_at FROM bookmarks",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "bookId": row.get::<_, String>(0)?,
                        "bookmarkId": row.get::<_, String>(1)?,
                        "locatorJson": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        "label": row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        "createdAt": row.get::<_, String>(4)?,
                    }))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                bookmarks.push(row.map_err(|error| error.to_string())?);
            }
        }
        Ok(serde_json::json!({
            "schemaVersion": 1,
            "books": books,
            "locators": locators,
            "bookmarks": bookmarks,
        }))
    }

    /// Merge a `reader_db.json` dump back in — books/locators upsert by
    /// book_id, bookmarks insert-or-ignore by bookmark_id so a re-import
    /// never duplicates. Rows naming a book that is not in `known_book_ids`
    /// are skipped (a bundle can carry state for a book that failed to
    /// restore or already exists under a different id).
    pub fn import_state(
        &self,
        value: &serde_json::Value,
        known_book_ids: &std::collections::HashSet<String>,
    ) -> Result<ReaderDbMergeReport, String> {
        if value.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1) {
            return Err("BUNDLE_READER_DB_UNSUPPORTED".to_string());
        }
        let mut report = ReaderDbMergeReport::default();
        let empty = Vec::new();
        for row in value
            .get("books")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
        {
            let book_id = row.get("bookId").and_then(|v| v.as_str()).unwrap_or("");
            let dir_rel = row.get("dirRel").and_then(|v| v.as_str()).unwrap_or("");
            if book_id.is_empty() || dir_rel.is_empty() || !known_book_ids.contains(book_id) {
                continue;
            }
            self.upsert_book(
                book_id,
                dir_rel,
                row.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                row.get("format").and_then(|v| v.as_str()).unwrap_or(""),
            )?;
            report.books += 1;
        }
        for row in value
            .get("locators")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
        {
            let book_id = row.get("bookId").and_then(|v| v.as_str()).unwrap_or("");
            let locator = row
                .get("locatorJson")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if book_id.is_empty() || locator.is_empty() || !known_book_ids.contains(book_id) {
                continue;
            }
            self.save_locator(book_id, locator)?;
            report.locators += 1;
        }
        let fallback_created_at = now_stamp();
        for row in value
            .get("bookmarks")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
        {
            let book_id = row.get("bookId").and_then(|v| v.as_str()).unwrap_or("");
            let bookmark_id = row.get("bookmarkId").and_then(|v| v.as_str()).unwrap_or("");
            if book_id.is_empty() || bookmark_id.is_empty() || !known_book_ids.contains(book_id) {
                continue;
            }
            let created_at = row
                .get("createdAt")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .unwrap_or(fallback_created_at.as_str());
            let inserted = self
                .conn
                .execute(
                    "INSERT INTO bookmarks(book_id, bookmark_id, locator_json, label, created_at) VALUES(?1, ?2, ?3, ?4, ?5) ON CONFLICT(bookmark_id) DO NOTHING",
                    params![
                        book_id,
                        bookmark_id,
                        row.get("locatorJson").and_then(|v| v.as_str()).unwrap_or(""),
                        row.get("label").and_then(|v| v.as_str()).unwrap_or(""),
                        created_at,
                    ],
                )
                .map_err(|error| error.to_string())?;
            report.bookmarks += inserted as u32;
        }
        Ok(report)
    }
}

/// LIKE-fallback snippet: ~60 chars of context around the first
/// case-insensitive match, match wrapped in `<b></b>` like the FTS5 path.
fn like_snippet(body: &str, query: &str) -> String {
    const CONTEXT: usize = 60;
    let needle = query.to_lowercase();
    let haystack = body.to_lowercase();
    let Some(start) = haystack.find(&needle) else {
        return body.chars().take(CONTEXT * 2).collect();
    };
    let char_index = haystack[..start].chars().count();
    let needle_chars = needle.chars().count();
    let from = char_index.saturating_sub(CONTEXT);
    let to = (char_index + needle_chars + CONTEXT).min(body.chars().count());
    let mut snippet = String::new();
    if from > 0 {
        snippet.push('…');
    }
    // Slice on char boundaries only — collect positions first.
    let boundaries: Vec<usize> = body.char_indices().map(|(index, _)| index).collect();
    let byte_from = boundaries.get(from).copied().unwrap_or(body.len());
    let match_from = boundaries.get(char_index).copied().unwrap_or(body.len());
    let match_to = boundaries
        .get(char_index + needle_chars)
        .copied()
        .unwrap_or(body.len());
    let byte_to = boundaries.get(to).copied().unwrap_or(body.len());
    snippet.push_str(&body[byte_from..match_from]);
    snippet.push_str("<b>");
    snippet.push_str(&body[match_from..match_to]);
    snippet.push_str("</b>");
    snippet.push_str(&body[match_to..byte_to]);
    if to < body.chars().count() {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::ReaderDb;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(name: &str) -> (PathBuf, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ir-reader-db-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("reader.db");
        (root, path)
    }

    #[test]
    fn book_index_and_locator_round_trip() {
        let (root, path) = temp_db("roundtrip");
        let db = ReaderDb::open_at(&path).expect("open");

        db.upsert_book("book-1", "手动/书", "manual", "markdown")
            .expect("upsert");
        assert_eq!(
            db.book_dir("book-1").expect("lookup"),
            Some("手动/书".to_string())
        );
        assert!(db.book_dir("missing").expect("lookup").is_none());

        // Upsert updates, does not duplicate.
        db.upsert_book("book-1", "手动/书2", "manual", "epub")
            .expect("re-upsert");
        let all = db.all_books().expect("all_books");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, "手动/书2");
        assert_eq!(all[0].3, "epub");

        assert!(db.load_locator("book-1").expect("locator").is_none());
        db.save_locator("book-1", "{\"chapterId\":\"c1\"}")
            .expect("save locator");
        assert_eq!(
            db.load_locator("book-1").expect("locator"),
            Some("{\"chapterId\":\"c1\"}".to_string())
        );

        // remove_book drops the index + locator rows too.
        db.remove_book("book-1").expect("remove");
        assert!(db.book_dir("book-1").expect("lookup").is_none());
        assert!(db.load_locator("book-1").expect("locator").is_none());
        // Windows: the dir cannot go away while SQLite still holds handles.
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bookmarks_add_list_remove() {
        let (root, path) = temp_db("bookmarks");
        let db = ReaderDb::open_at(&path).expect("open");

        db.add_bookmark("book-1", "bm-2", "{\"chapterId\":\"c2\"}", "第二章")
            .expect("add");
        std::thread::sleep(std::time::Duration::from_millis(2));
        db.add_bookmark("book-1", "bm-1", "{\"chapterId\":\"c1\"}", "第一章")
            .expect("add");
        db.add_bookmark("other", "bm-9", "{}", "")
            .expect("other book");

        let list = db.list_bookmarks("book-1").expect("list");
        assert_eq!(list.len(), 2);
        // created_at order: bm-2 was written first.
        assert_eq!(list[0].bookmark_id, "bm-2");
        assert_eq!(list[1].label, "第一章");

        db.remove_bookmark("book-1", "bm-1").expect("remove");
        let list = db.list_bookmarks("book-1").expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].bookmark_id, "bm-2");

        // Removing the book drops its bookmarks.
        db.remove_book("book-1").expect("remove book");
        assert!(db.list_bookmarks("book-1").expect("list").is_empty());
        assert_eq!(db.list_bookmarks("other").expect("list").len(), 1);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fts_search_finds_and_snippets() {
        let (root, path) = temp_db("fts");
        let db = ReaderDb::open_at(&path).expect("open");
        db.fts_replace_chapter(
            "book-1",
            "ch1",
            "第一章 开始",
            "这是一个关于阅读和知识管理的长篇章节，讨论沉浸阅读的价值。",
        )
        .expect("index ch1");
        db.fts_replace_chapter("book-1", "ch2", "第二章 无关", "完全不同的内容，讲烹饪。")
            .expect("index ch2");
        db.fts_replace_chapter("other-book", "ch1", "别的书", "也谈阅读")
            .expect("index other");

        let hits = db.fts_search("book-1", "阅读", 10).expect("search");
        assert!(!hits.is_empty(), "expected hits, got none");
        assert!(hits.iter().any(|hit| hit.chapter_id == "ch1"));
        assert!(hits.iter().all(|hit| !hit.chapter_id.is_empty()));
        assert!(hits[0].snippet.contains("<b>"));

        // Scoped to the book: other-book's chapter must not leak in.
        assert!(hits
            .iter()
            .all(|hit| hit.chapter_id != "ch1" || hit.title != "别的书"));

        // Empty query and zero limit return nothing.
        assert!(db.fts_search("book-1", "  ", 10).expect("blank").is_empty());
        assert!(db.fts_search("book-1", "阅读", 0).expect("zero").is_empty());

        // Re-indexing replaces rather than duplicates.
        db.fts_replace_chapter("book-1", "ch1", "第一章 开始", "改后的正文不再提及旧词。")
            .expect("re-index");
        let hits = db
            .fts_search("book-1", "沉浸", 10)
            .expect("search after reindex");
        assert!(hits.is_empty());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fts_search_quotes_hostile_operators() {
        let (root, path) = temp_db("fts-hostile");
        let db = ReaderDb::open_at(&path).expect("open");
        db.fts_replace_chapter("b", "c1", "标题", "ordinary text body")
            .expect("index");
        // FTS operator syntax must not error or return everything.
        let hits = db
            .fts_search("b", "\" OR * NEAR(", 10)
            .expect("hostile query");
        assert!(hits.is_empty());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_database_is_quarantined_and_rebuilt() {
        let (root, path) = temp_db("corrupt");
        // Well over SQLite's 100-byte header so it is unambiguously NOTADB.
        fs::write(&path, vec![0x5a_u8; 4096]).expect("corrupt fixture");
        fs::write(root.join("reader.db-wal"), b"stale wal").expect("wal fixture");

        let db = ReaderDb::open_at(&path).expect("corrupt database must self-heal");
        db.upsert_book("b", "dir", "manual", "markdown")
            .expect("rebuilt db accepts writes");
        let quarantined: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".corrupt-"))
            .collect();
        assert!(
            quarantined
                .iter()
                .any(|name| name.starts_with("reader.db.corrupt-")),
            "damaged db must be quarantined: {quarantined:?}"
        );
        drop(db);
        let reopened = ReaderDb::open_at(&path).expect("rebuilt db reopens");
        assert_eq!(reopened.book_dir("b").unwrap(), Some("dir".to_string()));
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_then_import_state_merges() {
        let (root, path) = temp_db("export-import");
        let db = ReaderDb::open_at(&path).expect("open");
        db.upsert_book("b1", "手动/甲", "manual", "markdown")
            .unwrap();
        db.save_locator("b1", "{\"chapterId\":\"c1\"}").unwrap();
        db.add_bookmark("b1", "bm-1", "{\"chapterId\":\"c1\"}", "mark")
            .unwrap();
        let dump = db.export_state().expect("export");

        let other_path = root.join("other.db");
        let other = ReaderDb::open_at(&other_path).expect("other open");
        let known: std::collections::HashSet<String> = ["b1".to_string()].into_iter().collect();
        let report = other.import_state(&dump, &known).expect("import");
        assert_eq!(report.books, 1);
        assert_eq!(report.locators, 1);
        assert_eq!(report.bookmarks, 1);
        assert_eq!(other.book_dir("b1").unwrap(), Some("手动/甲".to_string()));
        assert_eq!(
            other.load_locator("b1").unwrap(),
            Some("{\"chapterId\":\"c1\"}".to_string())
        );
        assert_eq!(other.list_bookmarks("b1").unwrap().len(), 1);

        // Second import is idempotent — the bookmark id conflict is ignored.
        let report = other.import_state(&dump, &known).expect("re-import");
        assert_eq!(report.bookmarks, 0);

        // Rows for unknown books are skipped.
        let empty: std::collections::HashSet<String> = Default::default();
        let report = other.import_state(&dump, &empty).expect("import empty");
        assert_eq!(report.books, 0);
        drop(db);
        drop(other);
        fs::remove_dir_all(root).unwrap();
    }
}
