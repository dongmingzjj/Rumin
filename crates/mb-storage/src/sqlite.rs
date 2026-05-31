//! SQLite storage backend for cookies and state

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Simple key-value storage backed by a JSON file (SQLite later)
/// For MVP, we use a flat file to avoid SQLite dependency complexity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub cookies: Vec<StoredCookie>,
    pub local_storage: Vec<(String, String, String)>, // (domain, key, value)
    pub session_storage: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCookie {
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
