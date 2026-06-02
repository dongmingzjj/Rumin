//! HTTP client implementation
//!
//! Provides an async HTTP client built on wreq with:
//! - HTTP/1.1 and HTTP/2 support
//! - Connection pooling
//! - Timeout configuration
//! - TLS fingerprint emulation (Chrome via BoringSSL)
//! - Cookie jar integration

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use wreq_util::Emulation;

use crate::cookie::CookieJar;
use crate::request::HttpRequest;
use crate::response::HttpResponse;
use crate::tls::TlsConfig;

/// Proxy configuration
#[derive(Debug, Clone)]
pub enum ProxyConfig {
    /// No proxy
    None,
    /// HTTP proxy at the given URL (e.g. "http://127.0.0.1:7890")
    Http(String),
    // SOCKS5 would require additional dependencies (tokio-socks)
}

/// TLS fingerprint emulation preset
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmulationPreset {
    Chrome136,
    Chrome120,
    Firefox136,
    Safari18,
}

impl Default for EmulationPreset {
    fn default() -> Self {
        Self::Chrome136
    }
}

impl EmulationPreset {
    fn to_emulation(self) -> Emulation {
        match self {
            Self::Chrome136 => Emulation::Chrome136,
            Self::Chrome120 => Emulation::Chrome120,
            Self::Firefox136 => Emulation::Firefox136,
            Self::Safari18 => Emulation::Safari18,
        }
    }
}

/// Configuration for the HTTP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Total request timeout
    pub request_timeout: Duration,
    /// Maximum idle connections per host
    pub pool_max_idle_per_host: usize,
    /// Idle connection timeout
    pub pool_idle_timeout: Duration,
    /// TLS configuration (kept for compatibility, emulation handles TLS)
    pub tls: TlsConfig,
    /// Proxy configuration
    pub proxy: ProxyConfig,
    /// Whether to support HTTP/2
    pub http2: bool,
    /// TLS fingerprint emulation preset
    pub emulation: EmulationPreset,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            pool_max_idle_per_host: 32,
            pool_idle_timeout: Duration::from_secs(90),
            tls: TlsConfig::default(),
            proxy: ProxyConfig::None,
            http2: true,
            emulation: EmulationPreset::default(),
        }
    }
}

/// Builder for [`ClientConfig`]
#[derive(Debug, Default)]
pub struct ClientConfigBuilder {
    config: ClientConfig,
}

impl ClientConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config.request_timeout = timeout;
        self
    }

    pub fn pool_max_idle_per_host(mut self, max: usize) -> Self {
        self.config.pool_max_idle_per_host = max;
        self
    }

    pub fn pool_idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.pool_idle_timeout = timeout;
        self
    }

    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.config.tls = tls;
        self
    }

    pub fn proxy(mut self, proxy: ProxyConfig) -> Self {
        self.config.proxy = proxy;
        self
    }

    pub fn http2(mut self, enable: bool) -> Self {
        self.config.http2 = enable;
        self
    }

    pub fn emulation(mut self, preset: EmulationPreset) -> Self {
        self.config.emulation = preset;
        self
    }

    pub fn build(self) -> ClientConfig {
        self.config
    }
}

/// HTTP client with connection pooling, TLS fingerprint emulation, and cookie support
pub struct HttpClient {
    client: wreq::Client,
    config: ClientConfig,
    cookie_jar: Option<Arc<Mutex<CookieJar>>>,
}

impl HttpClient {
    /// Create a new client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(ClientConfig::default())
    }

    /// Create a new client with the given configuration
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        let mut builder = wreq::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .pool_idle_timeout(config.pool_idle_timeout)
            // Follow up to 10 redirects (wreq defaults to none)
            .redirect(wreq::redirect::Policy::limited(10))
            // Apply TLS fingerprint emulation (sets TLS, HTTP/2, and headers)
            .emulation(config.emulation.to_emulation());

        // Configure proxy
        match &config.proxy {
            ProxyConfig::None => {}
            ProxyConfig::Http(url) => {
                builder = builder.proxy(wreq::Proxy::all(url)?);
            }
        }

        let client = builder.build()?;

        Ok(Self {
            client,
            config,
            cookie_jar: None,
        })
    }

    /// Attach a shared cookie jar to this client
    pub fn with_cookies(mut self, jar: Arc<Mutex<CookieJar>>) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    /// Get a clone of the cookie jar contents
    pub fn cookies(&self) -> Option<CookieJar> {
        self.cookie_jar
            .as_ref()
            .and_then(|m| m.lock().ok())
            .map(|j| j.clone())
    }

    /// Execute an HTTP GET request
    pub async fn get(&self, url: &str) -> Result<HttpResponse> {
        let request = HttpRequest::get(url);
        self.execute(request).await
    }

    /// Execute an HTTP POST request with a body
    pub async fn post(&self, url: &str, body: Vec<u8>) -> Result<HttpResponse> {
        let request = HttpRequest::post(url).body(body);
        self.execute(request).await
    }

    /// Execute a full HttpRequest
    pub async fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        let url: url::Url = request.url.parse()?;

        // Build the wreq request (uses From<Method> impl in request.rs)
        let mut builder = self
            .client
            .request(http::Method::from(request.method), url.as_str());

        // Add custom headers
        for (key, value) in &request.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        // Add cookie header if we have a jar
        if let Some(ref jar_mutex) = self.cookie_jar {
            if let Ok(jar) = jar_mutex.lock() {
                let domain = url.host_str().unwrap_or("");
                let path = url.path();
                if let Some(cookie_str) = jar.cookie_header(domain, path) {
                    builder = builder.header("cookie", cookie_str.as_str());
                }
            }
        }

        // Set body
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }

        // Execute the request
        let response = builder.send().await?;

        // Extract response parts
        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response.bytes().await?;

        // Process Set-Cookie headers
        if let Some(ref jar_mutex) = self.cookie_jar {
            if let Ok(mut jar) = jar_mutex.lock() {
                let domain = url.host_str().unwrap_or("");
                for value in headers.get_all("set-cookie") {
                    if let Ok(s) = value.to_str() {
                        jar.parse_set_cookie(s, domain);
                    }
                }
            }
        }

        Ok(HttpResponse {
            status,
            headers,
            body: body_bytes,
            url: request.url,
        })
    }

    /// Get the client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new().expect("failed to create default HttpClient")
    }
}
