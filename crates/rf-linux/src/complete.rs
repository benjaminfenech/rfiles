//! Address-bar path completion: folders whose name starts with what the user typed after the
//! last separator. Bounded (stops after `limit` matches) and cheap (one directory enumeration,
//! names only). Blocking: call off the UI thread; slow network paths just answer late.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
/// Splits `C:\Users\oz` into (`C:\Users\`, `oz`). `None` when there is no directory part.
fn split(input: &str) -> Option<(PathBuf, String)> {
    let cut = input.rfind(['\\', '/'])?;
    let (dir, rest) = input.split_at(cut + 1);
    Some((PathBuf::from(dir), rest.to_string()))
}
/// Folder completions for `input` (case-insensitive prefix), sorted naturally, hidden/system
/// folders excluded unless `include_hidden`. Returns full paths.
pub fn complete_dirs(input: &str, limit: usize, include_hidden: bool) -> Vec<PathBuf> {
    let input = input.trim().trim_matches('"');
    let owned;
    let input = if input.len() == 2 && input.ends_with(':') {
        owned = format!("{input}\\");
        owned.as_str()
    } else {
        input
    };
    let Some((dir, prefix)) = split(input) else { return Vec::new() };
    if !dir.is_absolute() && !dir.to_string_lossy().starts_with(r"\\") {
        return Vec::new();
    }
    let prefix = prefix.to_lowercase();
    let mut out = Vec::new();
    let found = std::cell::Cell::new(0usize);
    let _ = crate::list_dir(
        &dir,
        256,
        &|| found.get() >= limit * 4,
        &mut |batch| {
            for e in batch {
                if !e.is_dir() {
                    continue;
                }
                if !include_hidden && (e.is_hidden() || e.is_system()) {
                    continue;
                }
                if e.name.to_lowercase().starts_with(&prefix) {
                    out.push(dir.join(&*e.name));
                    found.set(found.get() + 1);
                }
            }
        },
    );
    out.sort_by(|a, b| {
        rf_core::sort::natural_cmp(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    out.truncate(limit);
    out
}
/// True if `p` looks like something the completion can work on (absolute or UNC).
pub fn completable(p: &str) -> bool {
    let p = p.trim();
    Path::new(p).is_absolute() || p.starts_with(r"\\")
        || (p.len() == 2 && p.ends_with(':'))
}
