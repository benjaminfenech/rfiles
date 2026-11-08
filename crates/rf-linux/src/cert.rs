//! Certificate files (.cer, .crt, .der, .pem): subject, issuer and validity.
//!
//! Linux: rf-win decodes with Windows' CryptoAPI. Here `info` returns `None` (the DirectLook
//! card shows no certificate part) until an X.509 parser is added (Phase 2).

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    /// Unix milliseconds.
    pub not_before: i64,
    pub not_after: i64,
}

pub fn info(_path: &Path) -> Option<CertInfo> {
    None
}
