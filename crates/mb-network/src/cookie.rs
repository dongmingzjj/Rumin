//! Cookie management (RFC 6265)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single HTTP cookie
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: String,
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
}

/// Cookie jar: stores and retrieves cookies by domain/path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieJar {
    cookies: HashMap<String, Vec<Cookie>>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self {
            cookies: HashMap::new(),
        }
    }

    /// Insert a cookie into the jar, keyed by domain
    pub fn insert(&mut self, cookie: Cookie) {
        let domain = cookie
            .domain
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let entry = self.cookies.entry(domain).or_default();

        // Replace existing cookie with same name
        if let Some(pos) = entry.iter().position(|c| c.name == cookie.name) {
            entry[pos] = cookie;
        } else {
            entry.push(cookie);
        }
    }

    /// Get all cookies matching the given domain and path.
    /// Domain matching supports exact match and parent domain.
    pub fn get_matching(&self, domain: &str, path: &str) -> Vec<&Cookie> {
        let mut result = Vec::new();

        for (cookie_domain, cookies) in &self.cookies {
            // Domain must match exactly or be a parent domain
            if domain == cookie_domain || domain.ends_with(&format!(".{}", cookie_domain)) {
                for cookie in cookies {
                    // Path must start with cookie path
                    if path.starts_with(&cookie.path) {
                        result.push(cookie);
                    }
                }
            }
        }

        result
    }

    /// Get all cookies as a flat list
    pub fn all_cookies(&self) -> Vec<&Cookie> {
        self.cookies.values().flatten().collect()
    }

    /// Clear all cookies
    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    /// Parse a Set-Cookie header value and add to jar
    pub fn parse_set_cookie(&mut self, header: &str, request_domain: &str) {
        if let Some(cookie) = parse_set_cookie(header, request_domain) {
            self.insert(cookie);
        }
    }

    /// Parse multiple Set-Cookie headers
    pub fn parse_set_cookies(&mut self, headers: &[&str], request_domain: &str) {
        for header in headers {
            self.parse_set_cookie(header, request_domain);
        }
    }

    /// Build a Cookie header value for the given domain and path
    pub fn cookie_header(&self, domain: &str, path: &str) -> Option<String> {
        let matching = self.get_matching(domain, path);
        if matching.is_empty() {
            return None;
        }
        let pairs: Vec<String> = matching
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        Some(pairs.join("; "))
    }
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a single Set-Cookie header value into a Cookie
pub fn parse_set_cookie(header: &str, request_domain: &str) -> Option<Cookie> {
    let parts: Vec<&str> = header.split(';').map(|s| s.trim()).collect();
    let first = parts.first()?;

    let eq_pos = first.find('=')?;
    let name = first[..eq_pos].to_string();
    let value = first[eq_pos + 1..].to_string();

    let mut cookie = Cookie {
        name,
        value,
        domain: None,
        path: "/".to_string(),
        expires: None,
        secure: false,
        http_only: false,
    };

    // Parse attributes
    for part in &parts[1..] {
        let lower = part.to_lowercase();
        if lower.starts_with("domain=") {
            let domain_val = &part[7..];
            // Remove leading dot per RFC 6265
            let domain_val = domain_val.trim_start_matches('.');
            cookie.domain = Some(domain_val.to_string());
        } else if lower.starts_with("path=") {
            cookie.path = part[5..].to_string();
        } else if lower == "secure" {
            cookie.secure = true;
        } else if lower == "httponly" {
            cookie.http_only = true;
        }
        // Expires/Max-Age parsing could be added here
    }

    // Default domain to request domain
    if cookie.domain.is_none() {
        cookie.domain = Some(request_domain.to_string());
    }

    Some(cookie)
}
