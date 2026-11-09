//! The in-memory file-name index of local drives (upstream 0.4.0, NTFS/ReFS). Not on Linux
//! yet: never running, so searches walk the disk as before.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing usable yet: the first walk is running.
    Building,
    /// Searchable (a saved index may be refreshing behind it).
    Ready,
    /// The root couldn't be indexed.
    Failed,
}

pub fn start(_roots: Vec<PathBuf>, _dir: PathBuf) {}

pub fn stop() {}

/// Index hits under `under`; None when the index can't answer (always, on Linux).
pub fn search(_pattern: &str, _under: &Path, _include_hidden: bool, _max: usize) -> Option<Vec<PathBuf>> {
    None
}

/// (root, state, live entries, memory) per indexed root.
pub fn status() -> Vec<(PathBuf, State, usize, usize)> {
    Vec::new()
}

/// The drives the index would cover.
pub fn local_roots() -> Vec<PathBuf> {
    Vec::new()
}

pub fn running() -> bool {
    false
}
