//! Browser state snapshot — save and restore full browser state

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::sqlite::Storage;

/// Complete browser state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSnapshot {
    pub version: u32,
    pub url: String,
    pub title: String,
    pub html: String,
    pub storage: Storage,
    pub timestamp: i64,
}

impl BrowserSnapshot {
    pub fn new(url: &str, title: &str, html: &str, storage: Storage) -> Self {
        Self {
            version: 1,
            url: url.to_string(),
            title: title.to_string(),
            html: html.to_string(),
            storage,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let snapshot: Self = serde_json::from_str(&json)?;
        Ok(snapshot)
    }
}
