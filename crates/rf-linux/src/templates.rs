//! The user's file templates: the files in the XDG templates folder (`~/Templates`, from
//! `user-dirs.dirs`), offered in the New menu as Dolphin, Nautilus and Thunar do.

use std::io;
use std::path::{Path, PathBuf};

/// One template: a file to copy, or a folder of them (a submenu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Template {
    File { path: PathBuf, label: String },
    Folder { label: String, items: Vec<Template> },
}

/// The templates folder, when the user has one.
pub fn dir() -> Option<PathBuf> {
    dirs::template_dir().filter(|d| d.is_dir())
}

/// The templates in `dir` (hidden files left out, links followed), sorted by label; folders
/// go one level deep, and empty ones are left out.
pub fn list(dir: &Path) -> Vec<Template> {
    list_in(dir, 1)
}

fn list_in(dir: &Path, depth: u32) -> Vec<Template> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Template> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        let path = e.path();
        // follows links: a template may be a link to a file elsewhere
        let Ok(m) = std::fs::metadata(&path) else { continue };
        if m.is_dir() {
            if depth > 0 {
                let items = list_in(&path, depth - 1);
                if !items.is_empty() {
                    out.push(Template::Folder { label: name, items });
                }
            }
        } else if m.is_file() {
            out.push(Template::File { label: label(&name), path });
        }
    }
    out.sort_by_cached_key(|t| match t {
        Template::File { label, .. } | Template::Folder { label, .. } => label.to_lowercase(),
    });
    out
}

/// A template's menu label: its name without the extension ("Letter.odt" → "Letter").
fn label(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// A new file in `dest` copied from `template`, named after it ("Letter (2).odt" if taken).
pub fn create_from(template: &Path, dest: &Path) -> io::Result<PathBuf> {
    let name = template.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    let target = crate::fileops::unique_name(dest, stem, ext);
    // create_new: never write over a file that appeared meanwhile
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
    let mut src = std::fs::File::open(template)?;
    if let Err(e) = io::copy(&mut src, &mut out) {
        let _ = std::fs::remove_file(&target);
        return Err(e);
    }
    // a script template stays runnable
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = src.metadata()?.permissions().mode() & 0o777;
        out.set_permissions(std::fs::Permissions::from_mode(mode | 0o200))?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_creates() {
        let d = std::env::temp_dir().join(format!("rf-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let t = d.join("Templates");
        std::fs::create_dir_all(t.join("Office/deeper")).unwrap();
        std::fs::create_dir_all(t.join("Empty")).unwrap();
        std::fs::write(t.join("Script.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::write(t.join("Office/Letter.odt"), b"odt").unwrap();
        std::fs::write(t.join("Office/deeper/too-deep.txt"), b"").unwrap();
        std::fs::write(t.join(".hidden"), b"").unwrap();
        std::fs::write(t.join("Makefile"), b"all:\n").unwrap();

        let got = list(&t);
        let labels: Vec<&str> = got
            .iter()
            .map(|x| match x {
                Template::File { label, .. } | Template::Folder { label, .. } => label.as_str(),
            })
            .collect();
        assert_eq!(labels, ["Makefile", "Office", "Script"], "sorted, hidden and empty folders left out");
        let Template::Folder { items, .. } = &got[1] else { panic!() };
        assert_eq!(items.len(), 1, "one level of folders: deeper/ is left out");

        let out = d.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let a = create_from(&t.join("Office/Letter.odt"), &out).unwrap();
        let b = create_from(&t.join("Office/Letter.odt"), &out).unwrap();
        assert_eq!(a, out.join("Letter.odt"));
        assert_eq!(b, out.join("Letter (2).odt"));
        assert_eq!(std::fs::read(&b).unwrap(), b"odt");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(t.join("Script.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let s = create_from(&t.join("Script.sh"), &out).unwrap();
        assert_eq!(std::fs::metadata(&s).unwrap().permissions().mode() & 0o777, 0o755, "keeps the execute bits");
        let _ = std::fs::remove_dir_all(&d);
    }
}
