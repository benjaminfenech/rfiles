//! Authenticode signatures of Windows programs, for the Properties window's Digital signatures
//! tab. Not read on Linux (the tab shows only when there are some); the types match rf-win's.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub signer: String,
    pub issuer: String,
    pub digest: String,
    pub timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signatures {
    pub list: Vec<Signature>,
    pub verdict: Result<(), String>,
}

pub fn read(_path: &Path) -> Option<Signatures> {
    None
}
