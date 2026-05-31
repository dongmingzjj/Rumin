//! mb-network: Network layer for the minimal browser
//!
//! Provides HTTP client, TLS, cookies, request/response types.

pub mod client;
pub mod cookie;
pub mod request;
pub mod response;
pub mod tls;

// Re-export key types at crate level
pub use client::{ClientConfig, ClientConfigBuilder, EmulationPreset, HttpClient, ProxyConfig};
pub use cookie::{Cookie, CookieJar};
pub use request::{HttpRequest, Method};
pub use response::HttpResponse;
pub use tls::{ChromeTlsFingerprint, TlsConfig};
