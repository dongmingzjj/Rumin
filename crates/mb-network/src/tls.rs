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

/// Chrome-like TLS fingerprint configuration
///
/// This structure holds the parameters needed to mimic a Chrome TLS fingerprint.
/// Now superseded by wreq's built-in emulation via `EmulationPreset`.
#[derive(Debug, Clone)]
pub struct ChromeTlsFingerprint {
    /// TLS cipher suites (e.g. TLS_AES_128_GCM_SHA256)
    pub cipher_suites: Vec<String>,
    /// Supported elliptic curves (e.g. X25519, P-256)
    pub curves: Vec<String>,
    /// Signature algorithms (e.g. ecdsa_secp256r1_sha256)
    pub sigalgs: Vec<String>,
    /// ALPN protocols (e.g. h2, http/1.1)
    pub alpn: Vec<String>,
}

impl Default for ChromeTlsFingerprint {
    fn default() -> Self {
        Self {
            cipher_suites: vec![
                "TLS_AES_128_GCM_SHA256".into(),
                "TLS_AES_256_GCM_SHA384".into(),
                "TLS_CHACHA20_POLY1305_SHA256".into(),
            ],
            curves: vec![
                "X25519".into(),
                "P-256".into(),
                "P-384".into(),
            ],
            sigalgs: vec![
                "ecdsa_secp256r1_sha256".into(),
                "rsa_pss_rsae_sha256".into(),
                "rsa_pkcs1_sha256".into(),
                "ecdsa_secp384r1_sha384".into(),
                "rsa_pss_rsae_sha384".into(),
                "rsa_pkcs1_sha384".into(),
                "rsa_pss_rsae_sha512".into(),
                "rsa_pkcs1_sha512".into(),
            ],
            alpn: vec!["h2".into(), "http/1.1".into()],
        }
    }
}

impl ChromeTlsFingerprint {
    /// Create a new fingerprint with Chrome defaults
    pub fn chrome() -> Self {
        Self::default()
    }
}
