//! TLS configuration and fingerprint support
//!
//! NOTE: TLS fingerprint emulation is now handled by wreq via `EmulationPreset`.
//! The types below are kept for backward compatibility.

/// TLS configuration for the HTTP client
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Whether to verify server certificates
    pub verify_certificates: bool,
    /// Whether to prefer HTTP/2 ALPN
    pub prefer_h2: bool,
    /// Custom User-Agent (now handled by emulation presets)
    pub user_agent: String,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            verify_certificates: true,
            prefer_h2: true,
            user_agent: String::from("MiniBrowser/0.1"),
        }
    }
}

// ChromeTlsFingerprint removed — superseded by wreq EmulationPreset
