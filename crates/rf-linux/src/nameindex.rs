//! The file-name index's settings (upstream 0.4.0). Linux has no index yet (see `indexer`);
//! the user's exclusions are kept for when it does.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

static USER_SKIP: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Folders left out by default: upstream's list without the Windows ones, plus ~/.cache.
const SKIP_NAMES: &[&str] = &["node_modules", ".git", "__pycache__", ".cargo", ".rustup", ".gradle", ".cache"];

pub fn set_excluded(dirs: &[PathBuf]) {
    *USER_SKIP.write().unwrap_or_else(|e| e.into_inner()) = dirs.to_vec();
}

/// The built-in exclusions, for showing them.
pub fn default_excluded_names() -> &'static [&'static str] {
    SKIP_NAMES
}

pub fn skipped(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| SKIP_NAMES.iter().any(|s| n == *s))
        || USER_SKIP.read().unwrap_or_else(|e| e.into_inner()).iter().any(|u| dir.starts_with(u))
}
