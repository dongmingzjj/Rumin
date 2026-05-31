//! HTTP request types

use std::collections::HashMap;

/// Supported HTTP methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
            Method::Patch => "PATCH",
        }
    }
}

impl From<Method> for http::Method {
    fn from(m: Method) -> Self {
        match m {
            Method::Get => http::Method::GET,
            Method::Post => http::Method::POST,
            Method::Put => http::Method::PUT,
            Method::Delete => http::Method::DELETE,
            Method::Head => http::Method::HEAD,
            Method::Options => http::Method::OPTIONS,
            Method::Patch => http::Method::PATCH,
        }
    }
}

/// HTTP request builder and container
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Create a new GET request
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Create a new POST request
    pub fn post(url: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Create a new request with the specified method
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Set a custom header
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set the request body (for POST/PUT/PATCH)
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Set the body as a JSON string, adding Content-Type header
    pub fn json_body(mut self, json: impl Into<String>) -> Self {
        self.body = json.into().into_bytes();
        self.headers
            .entry("content-type".to_string())
            .or_insert_with(|| "application/json".to_string());
        self
    }

    /// Set the body as form-encoded data, adding Content-Type header
    pub fn form_body(mut self, data: impl Into<String>) -> Self {
        self.body = data.into().into_bytes();
        self.headers
            .entry("content-type".to_string())
            .or_insert_with(|| "application/x-www-form-urlencoded".to_string());
        self
    }
}
