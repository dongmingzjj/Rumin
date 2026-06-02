//! Request recording / interceptor
//!
//! Provides [`RecordedRequest`] for serialisable request+response snapshots,
//! and [`RequestLog`] for collecting, serialising, and replaying them as curl
//! commands.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::request::{HttpRequest, Method};
use crate::response::HttpResponse;

// ---------------------------------------------------------------------------
// RecordedRequest
// ---------------------------------------------------------------------------

/// A single recorded HTTP transaction (request + response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub request_headers: HashMap<String, String>,
    #[serde(default)]
    pub request_body_size: usize,
    #[serde(default)]
    pub request_body_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "request_body")]
    pub request_body_full: Option<Vec<u8>>,
    pub response_status: u16,
    pub response_headers: HashMap<String, String>,
    /// Size of the response body in bytes (always recorded)
    #[serde(default)]
    pub response_body_size: usize,
    /// Hash fingerprint of the response body (always recorded)
    #[serde(default)]
    pub response_body_hash: String,
    /// Full response body (only recorded in --record-full mode, or loaded from legacy format)
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "response_body")]
    pub response_body_full: Option<Vec<u8>>,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
}

/// Wrap a string in single quotes for safe shell use.
/// Internal single quotes are escaped using the standard `'\''` idiom.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Compute a compact hash fingerprint for a byte slice (std-library only, no external deps).
fn body_fingerprint(data: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Convert an [`http::HeaderMap`] to a plain `HashMap<String, String>`.
/// Multi-valued headers are joined with `, `.
fn headermap_to_hashmap(headers: &http::HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for key in headers.keys() {
        let values: Vec<&str> = headers
            .get_all(key)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        if !values.is_empty() {
            map.insert(key.as_str().to_string(), values.join(", "));
        }
    }
    map
}

impl RecordedRequest {
    /// Build a [`RecordedRequest`] from an [`HttpRequest`] / [`HttpResponse`] pair.
    pub fn from_parts(req: &HttpRequest, resp: &HttpResponse, record_full_body: bool) -> Self {
        // Response body: hash + size (always), full only if requested
        let resp_body_bytes = &resp.body;
        let resp_body_size = resp_body_bytes.len();
        let resp_body_hash = body_fingerprint(resp_body_bytes);
        let resp_body_full = if record_full_body {
            Some(resp_body_bytes.to_vec())
        } else {
            None
        };
        // Request body: hash from reference (no clone), full only if requested
        let req_body_size = req.body.len();
        let req_body_hash = body_fingerprint(&req.body);
        let req_body_full = if record_full_body {
            Some(req.body.clone())
        } else {
            None
        };
        Self {
            method: req.method.as_str().to_string(),
            url: req.url.clone(),
            request_headers: req.headers.clone(),
            request_body_size: req_body_size,
            request_body_hash: req_body_hash,
            request_body_full: req_body_full,
            response_status: resp.status.as_u16(),
            response_headers: headermap_to_hashmap(&resp.headers),
            response_body_size: resp_body_size,
            response_body_hash: resp_body_hash,
            response_body_full: resp_body_full,
            timestamp: now_millis(),
        }
    }

    /// Pre-extract request data from a reference without cloning the body.
    /// Returns (method, url, headers, body_size, body_hash) — all cheap to clone.
    pub fn extract_request_meta(req: &HttpRequest) -> (String, String, HashMap<String, String>, usize, String) {
        (
            req.method.as_str().to_string(),
            req.url.clone(),
            req.headers.clone(),
            req.body.len(),
            body_fingerprint(&req.body),
        )
    }

    /// Build from pre-extracted request metadata + response.
    /// Avoids cloning the request body (hash + size only).
    pub fn from_precomputed(
        method: String,
        url: String,
        headers: HashMap<String, String>,
        body_size: usize,
        body_hash: String,
        resp: &HttpResponse,
        record_full_body: bool,
    ) -> Self {
        let resp_body_bytes = &resp.body;
        let resp_body_full = if record_full_body {
            Some(resp_body_bytes.to_vec())
        } else {
            None
        };
        Self {
            method,
            url,
            request_headers: headers,
            request_body_size: body_size,
            request_body_hash: body_hash,
            request_body_full: None, // request body never stored in compact mode
            response_status: resp.status.as_u16(),
            response_headers: headermap_to_hashmap(&resp.headers),
            response_body_size: resp_body_bytes.len(),
            response_body_hash: body_fingerprint(resp_body_bytes),
            response_body_full: resp_body_full,
            timestamp: now_millis(),
        }
    }

    /// Reconstruct an [`HttpRequest`] from this recorded entry.
    pub fn to_http_request(&self) -> Result<HttpRequest> {
        let method = match self.method.as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            "PATCH" => Method::Patch,
            other => anyhow::bail!("unsupported HTTP method: {other}"),
        };
        let mut req = HttpRequest::new(method, &self.url);
        for (k, v) in &self.request_headers {
            req = req.header(k.clone(), v.clone());
        }
        if let Some(ref body) = self.request_body_full {
            if !body.is_empty() {
                req = req.body(body.clone());
            }
        }
        Ok(req)
    }

    /// Render this request as a `curl` command string.
    pub fn to_curl(&self) -> String {
        let mut parts: Vec<String> = vec!["curl".to_string()];

        // Method (omit for default GET without body)
        if self.method != "GET" || self.request_body_size > 0 {
            parts.push(format!("-X {}", self.method));
        }

        // Headers
        for (k, v) in &self.request_headers {
            // Skip pseudo / auto-managed headers
            let lk = k.to_ascii_lowercase();
            if lk == "host" || lk == "content-length" || lk == "transfer-encoding" {
                continue;
            }
            parts.push(format!("-H {}", shell_single_quote(&format!("{}: {}", k, v))));
        }

        // Body
        if self.request_body_size > 0 {
            if let Some(ref body) = self.request_body_full {
                if let Ok(body_str) = std::str::from_utf8(body) {
                    parts.push(format!("--data {}", shell_single_quote(body_str)));
                } else {
                    parts.push(format!("--data-binary '<{} bytes binary>'", body.len()));
                }
            } else {
                parts.push(format!("<{} bytes, hash: {}>", self.request_body_size, self.request_body_hash));
            }
        }

        // URL (always last, quoted)
        parts.push(shell_single_quote(&self.url));

        parts.join(" \\\n  ")
    }
}

// ---------------------------------------------------------------------------
// RequestLog
// ---------------------------------------------------------------------------

/// An ordered collection of [`RecordedRequest`] entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    entries: Vec<RecordedRequest>,
    /// Whether to store full response bodies (not serialized, runtime-only)
    #[serde(skip, default)]
    record_full_body: bool,
}

impl RequestLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            record_full_body: false,
        }
    }

    /// Set whether to record full response bodies.
    pub fn with_full_body(mut self, full: bool) -> Self {
        self.record_full_body = full;
        self
    }

    /// Get whether full body recording is enabled.
    pub fn record_full_body(&self) -> bool {
        self.record_full_body
    }

    /// Record an HTTP request / response pair.
    pub fn record(&mut self, req: &HttpRequest, resp: &HttpResponse) {
        self.entries.push(RecordedRequest::from_parts(req, resp, self.record_full_body));
    }

    /// Record a pre-built entry directly.
    pub fn record_entry(&mut self, entry: RecordedRequest) {
        self.entries.push(entry);
    }

    /// Serialise the entire log to a JSON string.
    pub fn to_json(&self) -> String {
        // Unwrap is safe: serialisation of our own types should never fail.
        serde_json::to_string_pretty(&self.entries).unwrap_or_else(|_| "[]".to_string())
    }

    /// Deserialise a [`RequestLog`] from a JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        let entries: Vec<RecordedRequest> =
            serde_json::from_str(json).context("failed to parse RequestLog JSON")?;
        Ok(Self {
            entries,
            record_full_body: false,
        })
    }

    /// Render every entry as a `curl` command.
    pub fn to_curl_commands(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.to_curl()).collect()
    }

    /// Borrow the recorded entries.
    pub fn entries(&self) -> &[RecordedRequest] {
        &self.entries
    }

    /// Persist the log as JSON to `path`.
    pub fn save(&self, path: &str) -> Result<()> {
        let json = self.to_json();
        fs::write(path, json).with_context(|| format!("failed to write RequestLog to {path}"))
    }

    /// Load a log previously saved with [`save`](Self::save).
    pub fn load(path: &str) -> Result<Self> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("failed to read RequestLog from {path}"))?;
        Self::from_json(&json)
    }
}

impl Default for RequestLog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::HttpRequest;
    use crate::response::HttpResponse;
    use bytes::Bytes;
    use http::{HeaderMap, StatusCode};

    fn sample_pair() -> (HttpRequest, HttpResponse) {
        let req = HttpRequest::get("https://example.com/").header("accept", "text/html");
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-type", "text/html".parse().unwrap());
        let resp = HttpResponse {
            status: StatusCode::OK,
            headers: resp_headers,
            body: Bytes::from_static(b"<h1>Hello</h1>"),
            url: "https://example.com/".to_string(),
        };
        (req, resp)
    }

    #[test]
    fn record_and_retrieve() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new();
        log.record(&req, &resp);
        assert_eq!(log.entries().len(), 1);
        assert_eq!(log.entries()[0].method, "GET");
        assert_eq!(log.entries()[0].url, "https://example.com/");
        assert_eq!(log.entries()[0].response_status, 200);
    }

    #[test]
    fn json_round_trip() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new();
        log.record(&req, &resp);

        let json = log.to_json();
        let restored = RequestLog::from_json(&json).unwrap();
        assert_eq!(restored.entries().len(), 1);
        assert_eq!(restored.entries()[0].method, log.entries()[0].method);
        assert_eq!(restored.entries()[0].url, log.entries()[0].url);
    }

    #[test]
    fn save_and_load() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new();
        log.record(&req, &resp);

        let path = "/tmp/minibrowser_test_request_log.json";
        log.save(path).unwrap();
        let loaded = RequestLog::load(path).unwrap();
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()[0].response_status, 200);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn curl_output() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new();
        log.record(&req, &resp);
        let cmds = log.to_curl_commands();
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].contains("curl"));
        assert!(cmds[0].contains("https://example.com/"));
    }

    #[test]
    fn compact_mode_omits_full_body() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new(); // compact mode (default)
        log.record(&req, &resp);

        let entry = &log.entries()[0];
        assert_eq!(entry.response_body_size, 14); // b"<h1>Hello</h1>".len()
        assert!(!entry.response_body_hash.is_empty());
        assert!(entry.response_body_full.is_none());

        // JSON should not contain the full body array, only size + hash
        let json = log.to_json();
        assert!(!json.contains("response_body_full"));
        assert!(json.contains("response_body_size"));
        assert!(json.contains("response_body_hash"));
    }

    #[test]
    fn full_mode_stores_body() {
        let (req, resp) = sample_pair();
        let mut log = RequestLog::new().with_full_body(true);
        log.record(&req, &resp);

        let entry = &log.entries()[0];
        assert_eq!(entry.response_body_size, 14);
        assert!(entry.response_body_full.is_some());
        assert_eq!(entry.response_body_full.as_ref().unwrap(), b"<h1>Hello</h1>");
    }

    #[test]
    fn curl_url_with_single_quote() {
        let req = HttpRequest::get("https://example.com/it's").header("accept", "text/html");
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-type", "text/html".parse().unwrap());
        let resp = HttpResponse {
            status: StatusCode::OK,
            headers: resp_headers,
            body: Bytes::from_static(b"ok"),
            url: "https://example.com/it's".to_string(),
        };
        let mut log = RequestLog::new();
        log.record(&req, &resp);
        let cmd = &log.to_curl_commands()[0];
        // The single quote in the URL should be properly escaped
        assert!(cmd.contains("https://example.com/it'\\''s"), "got: {cmd}");
    }

    #[test]
    fn curl_header_value_with_single_quote() {
        let req = HttpRequest::get("https://example.com/")
            .header("x-custom", "it's a test");
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-type", "text/html".parse().unwrap());
        let resp = HttpResponse {
            status: StatusCode::OK,
            headers: resp_headers,
            body: Bytes::from_static(b"ok"),
            url: "https://example.com/".to_string(),
        };
        let mut log = RequestLog::new();
        log.record(&req, &resp);
        let cmd = &log.to_curl_commands()[0];
        // The single quote in the header should be properly escaped
        assert!(cmd.contains("it'\\''s a test"), "got: {cmd}");
    }

    #[test]
    fn backward_compat_old_format_with_response_body() {
        // Simulate old format JSON that has "response_body" field
        let old_json = r#"[
            {
                "method": "GET",
                "url": "https://example.com/",
                "request_headers": {},
                "request_body": [],
                "response_status": 200,
                "response_headers": {},
                "response_body": [60, 104, 49, 62, 72, 101, 108, 108, 111, 60, 47, 104, 49, 62],
                "timestamp": 1234567890
            }
        ]"#;
        let log = RequestLog::from_json(old_json).unwrap();
        assert_eq!(log.entries().len(), 1);
        let entry = &log.entries()[0];
        assert_eq!(entry.method, "GET");
        // Old "response_body" should be loaded into response_body_full via alias
        assert!(entry.response_body_full.is_some());
        assert_eq!(entry.response_body_full.as_ref().unwrap().len(), 14);
        // New fields default to empty/zero
        assert_eq!(entry.response_body_size, 0);
        assert!(entry.response_body_hash.is_empty());
    }
}
