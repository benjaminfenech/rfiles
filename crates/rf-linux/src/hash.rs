//! File hashes for the properties dialog ("Hashes" tab): MD5, SHA-1, SHA-256, SHA-512, CRC32.
//! One streaming pass over the file computes all five. Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::fs::File;
use std::io::Read;
use std::path::Path;
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
const BUF: usize = 1 << 20;
/// Lowercase hex digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashes {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub crc32: String,
    pub size: u64,
}
/// Returned (inside `anyhow::Error`) when `cancelled` fired; use `err.downcast_ref::<Cancelled>()`.
#[derive(Debug, thiserror::Error)]
#[error("İptal edildi")]
pub struct Cancelled;
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
/// Hashes `path`. `progress(bytes_done, bytes_total)` is called after every 1 MiB chunk;
/// `cancelled` is polled at the same points.
pub fn compute(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<Hashes> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    compute_reader(&mut f, total, progress, cancelled)
}
/// Same as [`compute`] for any reader (`total` is only used for progress reporting).
pub fn compute_reader(
    r: &mut dyn Read,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<Hashes> {
    let (mut md5, mut sha1, mut sha256, mut sha512) = (
        Md5::new(),
        Sha1::new(),
        Sha256::new(),
        Sha512::new(),
    );
    let mut crc = crc32fast::Hasher::new();
    let mut buf = vec![0u8; BUF];
    let mut done = 0u64;
    loop {
        if cancelled() {
            return Err(Cancelled.into());
        }
        let mut n = 0;
        while n < BUF {
            let k = r.read(&mut buf[n..])?;
            if k == 0 {
                break;
            }
            n += k;
        }
        if n == 0 {
            break;
        }
        let c = &buf[..n];
        md5.update(c);
        sha1.update(c);
        sha256.update(c);
        sha512.update(c);
        crc.update(c);
        done += n as u64;
        progress(done, total.max(done));
        if n < BUF {
            break;
        }
    }
    Ok(Hashes {
        md5: hex(&md5.finalize()),
        sha1: hex(&sha1.finalize()),
        sha256: hex(&sha256.finalize()),
        sha512: hex(&sha512.finalize()),
        crc32: format!("{:08x}", crc.finalize()),
        size: done,
    })
}
