//! SQLite storage backend for cookies

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// CookieStore — SQLite-backed persistent cookie storage
// ---------------------------------------------------------------------------

/// A cookie stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    pub expires: Option<i64>,
    pub created_at: i64,
}

/// Thread-safe cookie store backed by SQLite.
pub struct CookieStore {
    conn: Mutex<Connection>,
}

impl CookieStore {
    /// Open (or create) a SQLite database at the given path.
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory SQLite database (useful for tests).
    pub fn new_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cookies (
                id INTEGER PRIMARY KEY,
                domain TEXT NOT NULL,
                path TEXT NOT NULL DEFAULT '/',
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                expires INTEGER,
                secure INTEGER DEFAULT 0,
                http_only INTEGER DEFAULT 0,
                same_site TEXT DEFAULT 'None',
                created_at INTEGER NOT NULL,
                UNIQUE(domain, path, name)
            );
            CREATE INDEX IF NOT EXISTS idx_cookies_domain ON cookies(domain);",
        )?;
        Ok(())
    }

    /// Insert or update a cookie.
    pub fn store_cookie(
        &self,
        name: &str,
        value: &str,
        domain: &str,
        path: &str,
        secure: bool,
        http_only: bool,
        expires: Option<i64>,
        same_site: &str,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        conn.execute(
            "INSERT INTO cookies (domain, path, name, value, expires, secure, http_only, same_site, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(domain, path, name) DO UPDATE SET
                value = excluded.value,
                expires = excluded.expires,
                secure = excluded.secure,
                http_only = excluded.http_only,
                same_site = excluded.same_site",
            params![domain, path, name, value, expires, secure as i32, http_only as i32, same_site, now],
        )?;
        Ok(())
    }

    /// Retrieve all cookies matching a domain and path.
    pub fn get_cookies(&self, domain: &str, path: &str) -> Result<Vec<StoredCookie>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT name, value, domain, path, secure, http_only, same_site, expires, created_at
             FROM cookies
             WHERE domain = ?1 AND ?2 LIKE path || '%'",
        )?;
        let rows = stmt.query_map(params![domain, path], |row| {
            Ok(StoredCookie {
                name: row.get(0)?,
                value: row.get(1)?,
                domain: row.get(2)?,
                path: row.get(3)?,
                secure: row.get::<_, i32>(4)? != 0,
                http_only: row.get::<_, i32>(5)? != 0,
                same_site: row.get(6)?,
                expires: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        let mut cookies = Vec::new();
        for row in rows {
            cookies.push(row?);
        }
        Ok(cookies)
    }

    /// Remove a cookie by domain and name.
    pub fn remove_cookie(&self, domain: &str, name: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        conn.execute(
            "DELETE FROM cookies WHERE domain = ?1 AND name = ?2",
            params![domain, name],
        )?;
        Ok(())
    }

    /// Remove all cookies.
    pub fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        conn.execute_batch("DELETE FROM cookies;")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Storage — JSON-based snapshot storage (kept for BrowserSnapshot compatibility)
// ---------------------------------------------------------------------------

/// Simple key-value storage backed by a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub cookies: Vec<StorageCookie>,
    pub local_storage: Vec<(String, String, String)>, // (domain, key, value)
    pub session_storage: Vec<(String, String, String)>,
}

/// Cookie representation used in JSON snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires: Option<i64>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
            local_storage: Vec::new(),
            session_storage: Vec::new(),
        }
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let json = std::fs::read_to_string(path)?;
        let storage: Self = serde_json::from_str(&json)?;
        Ok(storage)
    }
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}
