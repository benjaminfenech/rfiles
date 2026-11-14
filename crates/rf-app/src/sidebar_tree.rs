//! The sidebar's folder tree (Settings → Sidebar sections → Folder tree): pinned folders and
//! drives open like Dolphin's Folders panel. A folder's subfolders are read when it is opened
//! (off the UI thread) and again whenever the current folder moves inside it, so a folder made
//! or deleted there shows up the next time you go in.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Tree {
    /// Folders opened in the tree.
    pub open: HashSet<PathBuf>,
    /// The subfolders of each opened folder, once read.
    pub kids: HashMap<PathBuf, Vec<PathBuf>>,
    /// The location the open folders were last re-read for.
    pub seen: Option<PathBuf>,
}

impl Tree {
    pub fn is_open(&self, p: &Path) -> bool {
        self.open.contains(p)
    }

    /// The opened folders holding `cur` (or `cur` itself): the ones to read again when
    /// the current folder becomes `cur`.
    pub fn holding(&self, cur: &Path) -> Vec<PathBuf> {
        self.open.iter().filter(|o| cur.starts_with(o)).cloned().collect()
    }
}

/// The folders in `dir` (links to folders too), in natural name order. Hidden ones only with
/// `hidden`. A folder that can't be read has none.
pub fn subfolders(dir: &Path, hidden: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let ft = e.file_type().ok()?;
            let is_dir = ft.is_dir() || (ft.is_symlink() && e.path().is_dir());
            is_dir.then(|| (name, e.path()))
        })
        .collect();
    out.sort_by(|a, b| rf_core::sort::natural_cmp(&a.0, &b.0));
    out.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_only() {
        let root = std::env::temp_dir().join(format!("rf-sbtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        struct Gone<'a>(&'a Path);
        impl Drop for Gone<'_> {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(self.0);
            }
        }
        let _gone = Gone(&root);
        let d = &*root;
        for n in ["b10", "b2", "a", ".hid"] {
            std::fs::create_dir(d.join(n)).unwrap();
        }
        std::fs::write(d.join("file.txt"), "x").unwrap();
        std::os::unix::fs::symlink(d.join("a"), d.join("link")).unwrap();
        std::os::unix::fs::symlink(d.join("file.txt"), d.join("flink")).unwrap();
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(subfolders(d, false)), ["a", "b2", "b10", "link"]);
        assert_eq!(names(subfolders(d, true)), [".hid", "a", "b2", "b10", "link"]);
        assert!(subfolders(&d.join("missing"), false).is_empty());
    }

    #[test]
    fn holding() {
        let mut t = Tree::default();
        t.open.insert("/home/me".into());
        t.open.insert("/home/me/src".into());
        t.open.insert("/mnt".into());
        let mut h = t.holding(Path::new("/home/me/src/x"));
        h.sort();
        assert_eq!(h, [PathBuf::from("/home/me"), PathBuf::from("/home/me/src")]);
        assert!(t.holding(Path::new("/home/meow")).is_empty());
    }
}
