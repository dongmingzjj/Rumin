//! HTTP response types

use http::StatusCode;

/// HTTP response container
#[derive(Debug)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: http::HeaderMap,
    pub body: bytes::Bytes,
    pub url: String,
}

impl HttpResponse {
    /// Check if the response status is successful (2xx)
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Get the status code as a u16
    pub fn status_code(&self) -> u16 {
        self.status.as_u16()
    }

    /// Get the response body as a UTF-8 string
    pub fn text(&self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.body.to_vec())
    }

    /// Try to parse the response body as JSON
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Get the raw body bytes
    pub fn bytes(&self) -> &bytes::Bytes {
        &self.body
    }

    /// Consume self and return the body bytes
    pub fn into_bytes(self) -> bytes::Bytes {
        self.body
    }

    /// Get a specific header value
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}
