//! Request recording / interceptor
//!
//! Provides [`RecordedRequest`] for serialisable request+response snapshots,
//! and [`RequestLog`] for collecting, serialising, and replaying them as curl
//! commands.

use std::collections::HashMap;
use std::fs;
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
    pub request_body: Vec<u8>,
    pub response_status: u16,
    pub response_headers: HashMap<String, String>,
    pub response_body: Vec<u8>,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    pub fn from_parts(req: &HttpRequest, resp: &HttpResponse) -> Self {
        Self {
            method: req.method.as_str().to_string(),
            url: req.url.clone(),
            request_headers: req.headers.clone(),
            request_body: req.body.clone(),
            response_status: resp.status.as_u16(),
            response_headers: headermap_to_hashmap(&resp.headers),
            response_body: resp.body.to_vec(),
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
        if !self.request_body.is_empty() {
            req = req.body(self.request_body.clone());
        }
        Ok(req)
    }

    /// Render this request as a `curl` command string.
    pub fn to_curl(&self) -> String {
        let mut parts: Vec<String> = vec!["curl".to_string()];

        // Method (omit for default GET without body)
        if self.method != "GET" || !self.request_body.is_empty() {
            parts.push(format!("-X {}", self.method));
        }

        // Headers
        for (k, v) in &self.request_headers {
            // Skip pseudo / auto-managed headers
            let lk = k.to_ascii_lowercase();
            if lk == "host" || lk == "content-length" || lk == "transfer-encoding" {
                continue;
            }
            parts.push(format!("-H '{}: {}'", k, v));
        }

        // Body
        if !self.request_body.is_empty() {
            if let Ok(body_str) = std::str::from_utf8(&self.request_body) {
                parts.push(format!("--data '{}'", body_str.replace('\'', "'\\''")));
            } else {
                // Binary body – use base64 (informational)
                parts.push(format!(
                    "--data-binary '<{} bytes binary>'",
                    self.request_body.len()
                ));
            }
        }

        // URL (always last, quoted)
        parts.push(format!("'{}'", self.url));

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
}

impl RequestLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record an HTTP request / response pair.
    pub fn record(&mut self, req: &HttpRequest, resp: &HttpResponse) {
        self.entries.push(RecordedRequest::from_parts(req, resp));
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
        Ok(Self { entries })
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
}
