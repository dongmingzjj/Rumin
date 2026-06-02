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
    /// Expiration as Unix timestamp in seconds.
    /// `None` = session cookie (no Expires/Max-Age).
    pub expires: Option<i64>,
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
    /// Filters out expired cookies.
    pub fn get_matching(&self, domain: &str, path: &str) -> Vec<&Cookie> {
        let mut result = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        for (cookie_domain, cookies) in &self.cookies {
            // Domain must match exactly or be a parent domain
            if domain == cookie_domain || domain.ends_with(&format!(".{}", cookie_domain)) {
                for cookie in cookies {
                    // Path must start with cookie path
                    if path.starts_with(&cookie.path) {
                        // Filter expired cookies
                        if let Some(exp) = cookie.expires {
                            if now > exp {
                                continue;
                            }
                        }
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
    let mut max_age: Option<i64> = None;
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
        } else if lower.starts_with("max-age=") {
            // Max-Age takes priority over Expires
            if let Ok(seconds) = part[8..].trim().parse::<i64>() {
                if seconds <= 0 {
                    // Max-Age <= 0 means expire immediately
                    max_age = Some(0);
                } else {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    max_age = Some(now + seconds);
                }
            }
        } else if lower.starts_with("expires=") {
            // Only use Expires if Max-Age hasn't been set
            if max_age.is_none() {
                let date_str = part[8..].trim();
                if let Some(ts) = parse_http_date(date_str) {
                    max_age = Some(ts);
                }
            }
        }
    }

    // Apply expiration (Max-Age has priority over Expires)
    cookie.expires = max_age;

    // Default domain to request domain
    if cookie.domain.is_none() {
        cookie.domain = Some(request_domain.to_string());
    }

    Some(cookie)
}

/// Parse an HTTP date string into a Unix timestamp (seconds).
/// Supports common formats: "Wed, 21 Oct 2025 07:28:00 GMT"
fn parse_http_date(date_str: &str) -> Option<i64> {
    let months = [
        ("jan", 1), ("feb", 2), ("mar", 3), ("apr", 4),
        ("may", 5), ("jun", 6), ("jul", 7), ("aug", 8),
        ("sep", 9), ("oct", 10), ("nov", 11), ("dec", 12),
    ];

    // Strip day-of-week prefix if present (e.g. "Wed,")
    let s = date_str.trim();
    let s = if let Some(pos) = s.find(',') {
        s[pos + 1..].trim()
    } else {
        s
    };

    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }

    let day: i64 = parts[0].parse().ok()?;
    let mon_str = parts[1].to_lowercase();
    let year: i64 = parts[2].parse().ok()?;
    let time_parts: Vec<&str> = parts[3].split(':').collect();
    if time_parts.len() < 3 {
        return None;
    }
    let hour: i64 = time_parts[0].parse().ok()?;
    let minute: i64 = time_parts[1].parse().ok()?;
    let second: i64 = time_parts[2].parse().ok()?;

    let month: i64 = months
        .iter()
        .find(|(m, _)| *m == mon_str.as_str())
        .map(|(_, v)| *v)?;

    // Howard Hinnant's days_from_civil algorithm (correct for all dates)
    let mut y = year;
    let m = month;
    if m <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let adjusted_m = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * adjusted_m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146097 + doe - 719468;

    Some(days_since_epoch * 86400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_set_cookie_max_age() {
        let mut jar = CookieJar::new();
        jar.parse_set_cookie("session=abc123; Max-Age=3600; Path=/", "example.com");
        let cookies = jar.get_matching("example.com", "/");
        assert_eq!(cookies.len(), 1);
        assert!(cookies[0].expires.is_some());
        // The cookie should expire roughly 3600 seconds from now
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let diff = cookies[0].expires.unwrap() - now;
        assert!(diff > 3500 && diff <= 3600);
    }

    #[test]
    fn test_parse_set_cookie_expired_max_age() {
        let mut jar = CookieJar::new();
        // Max-Age=0 means expire immediately
        jar.parse_set_cookie("session=abc123; Max-Age=0; Path=/", "example.com");
        let cookies = jar.get_matching("example.com", "/");
        assert_eq!(cookies.len(), 0, "cookie with Max-Age=0 should be filtered out");
    }

    #[test]
    fn test_parse_set_cookie_session_cookie() {
        let mut jar = CookieJar::new();
        jar.parse_set_cookie("session=abc123; Path=/", "example.com");
        let cookies = jar.get_matching("example.com", "/");
        assert_eq!(cookies.len(), 1);
        assert!(cookies[0].expires.is_none(), "session cookie should have None expires");
    }

    #[test]
    fn test_parse_set_cookie_expires() {
        let mut jar = CookieJar::new();
        jar.parse_set_cookie("id=xyz; Expires=Wed, 21 Oct 2099 07:28:00 GMT; Path=/", "example.com");
        let cookies = jar.get_matching("example.com", "/");
        assert_eq!(cookies.len(), 1);
        assert!(cookies[0].expires.is_some());
        assert!(cookies[0].expires.unwrap() > 0);
    }

    #[test]
    fn test_max_age_overrides_expires() {
        let mut jar = CookieJar::new();
        // Both Expires (far future) and Max-Age=0; Max-Age should win
        jar.parse_set_cookie(
            "id=xyz; Expires=Wed, 21 Oct 2099 07:28:00 GMT; Max-Age=0; Path=/",
            "example.com",
        );
        let cookies = jar.get_matching("example.com", "/");
        assert_eq!(cookies.len(), 0, "Max-Age=0 should override Expires and expire immediately");
    }

    #[test]
    fn test_parse_http_date() {
        let ts = parse_http_date("Wed, 21 Oct 2025 07:28:00 GMT");
        assert!(ts.is_some());
        // 2025-10-21T07:28:00Z = 1761031680
        let expected = 1761031680;
        assert_eq!(ts.unwrap(), expected);
    }
}
