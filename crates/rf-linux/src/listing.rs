//! Directory listing with `readdir` + `statx` (through std), streamed in batches.

use std::path::Path;

use rf_core::{Entry, EntryKind, attrs};

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("Klasör bulunamadı")]
    NotFound,
    #[error("Erişim reddedildi")]
    AccessDenied,
    #[error("Geçersiz klasör adı")]
    InvalidName,
    #[error("Sürücü hazır değil")]
    NotReady,
    #[error("Ağ yolu bulunamadı")]
    NetworkPath,
    #[error("Listeleme hatası (kod {0})")]
    Os(u32),
    #[error("İptal edildi")]
    Cancelled,
}

impl From<std::io::Error> for ListError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match e.kind() {
            NotFound => ListError::NotFound,
            PermissionDenied => ListError::AccessDenied,
            NotADirectory | InvalidFilename | InvalidInput => ListError::InvalidName,
            // a stale NFS/SMB mount (ESTALE, EHOSTDOWN, ENOTCONN…)
            StaleNetworkFileHandle | HostUnreachable | NetworkUnreachable | NotConnected | TimedOut => {
                ListError::NetworkPath
            }
            _ => ListError::Os(e.raw_os_error().unwrap_or(0) as u32),
        }
    }
}

/// Enumerates `dir` and streams entries to `on_batch` in chunks so the UI can show the first
/// items immediately. `cancelled` is polled between batches.
///
/// Names that aren't valid UTF-8 are shown lossily (Phase 1 keeps the raw bytes).
pub fn list_dir(
    dir: &Path,
    batch_size: usize,
    cancelled: &dyn Fn() -> bool,
    on_batch: &mut dyn FnMut(Vec<Entry>),
) -> Result<usize, ListError> {
    let rd = std::fs::read_dir(dir)?;
    let mut total = 0usize;
    let mut batch = Vec::with_capacity(batch_size.min(4096));
    for de in rd {
        let Ok(de) = de else { continue };
        if let Some(e) = to_entry(&de) {
            batch.push(e);
            if batch.len() >= batch_size {
                total += batch.len();
                on_batch(std::mem::replace(&mut batch, Vec::with_capacity(batch_size.min(4096))));
                if cancelled() {
                    return Err(ListError::Cancelled);
                }
            }
        }
    }
    if !batch.is_empty() {
        total += batch.len();
        on_batch(batch);
    }
    Ok(total)
}

fn ms(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// One entry. A symlink shows as what it points to (a broken one as a file) and carries
/// `REPARSE_POINT`, like a Windows junction or symlink.
pub(crate) fn to_entry(de: &std::fs::DirEntry) -> Option<Entry> {
    let name = de.file_name();
    let link = de.file_type().ok()?.is_symlink();
    let meta = if link { std::fs::metadata(de.path()).or_else(|_| de.metadata()) } else { de.metadata() };
    entry_from(&name.to_string_lossy(), link, &meta.ok()?)
}

/// The entry `name` in `dir` as [`list_dir`] would list it (the same fields): `Ok(None)` when
/// there is no such entry (deleted, renamed away), `Err` when it can't be told (no access, the
/// folder itself gone…).
pub fn stat_entry(dir: &Path, name: &str) -> Result<Option<Entry>, ListError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(ListError::InvalidName);
    }
    let path = dir.join(name);
    let lmeta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // the entry is gone; the folder too?
            return if dir.is_dir() { Ok(None) } else { Err(ListError::NotFound) };
        }
        Err(e) => return Err(e.into()),
    };
    let link = lmeta.file_type().is_symlink();
    let meta = if link { std::fs::metadata(&path).unwrap_or(lmeta) } else { lmeta };
    Ok(entry_from(name, link, &meta))
}

fn entry_from(name: &str, link: bool, meta: &std::fs::Metadata) -> Option<Entry> {
    let is_dir = meta.is_dir();
    let mut a = 0;
    if is_dir {
        a |= attrs::DIRECTORY;
    }
    if name.starts_with('.') {
        a |= attrs::HIDDEN;
    }
    if meta.permissions().readonly() {
        a |= attrs::READONLY;
    }
    if link {
        a |= attrs::REPARSE_POINT;
    }
    let kind = if is_dir { EntryKind::Dir } else { EntryKind::File };
    Some(Entry::new(
        name,
        kind,
        if is_dir { 0 } else { meta.len() },
        ms(meta.modified()),
        ms(meta.created()),
        a,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_temp_dir() {
        let dir = std::env::temp_dir().join(format!("rf-list-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for i in 0..250 {
            std::fs::write(dir.join(format!("f{i}.txt")), b"hello").unwrap();
        }
        std::fs::write(dir.join(".hidden"), b"").unwrap();
        std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();
        let mut all = Vec::new();
        let mut batches = 0;
        let n = list_dir(&dir, 100, &|| false, &mut |b| {
            batches += 1;
            all.extend(b)
        })
        .unwrap();
        assert_eq!(n, 253);
        assert!(batches >= 3);
        assert!(all.iter().any(|e| e.is_dir() && &*e.name == "sub"));
        assert!(all.iter().any(|e| &*e.name == "f7.txt" && e.size == 5 && &*e.ext == "txt"));
        assert!(all.iter().any(|e| &*e.name == ".hidden" && e.attrs & attrs::HIDDEN != 0));
        assert!(all.iter().any(|e| &*e.name == "link" && e.is_dir() && e.attrs & attrs::REPARSE_POINT != 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_dir() {
        let r = list_dir(Path::new("/definitely/not/here/rf"), 100, &|| false, &mut |_| {});
        assert!(matches!(r, Err(ListError::NotFound)));
    }

    /// One entry read alone matches what the listing gives for it; a missing name is `None`.
    #[test]
    fn stat_entry_matches_the_listing() {
        let dir = std::env::temp_dir().join(format!("rf-stat-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();
        let mut all = Vec::new();
        list_dir(&dir, 100, &|| false, &mut |b| all.extend(b)).unwrap();
        for e in &all {
            let one = stat_entry(&dir, &e.name).unwrap().expect("there");
            assert_eq!((one.name.as_ref(), one.size, one.attrs, one.modified_ms), (e.name.as_ref(), e.size, e.attrs, e.modified_ms));
        }
        assert!(stat_entry(&dir, "gone.txt").unwrap().is_none());
        assert!(matches!(stat_entry(&dir, "x/y"), Err(ListError::InvalidName)));
        assert!(matches!(stat_entry(&dir.join("no-such-folder"), "a.txt"), Err(ListError::NotFound)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
