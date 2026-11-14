//! Owner, group and permission bits, for the Properties window's Permissions section.
//! Blocking (disk, the user database): call off the UI thread.

use std::ffi::CStr;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

/// A file's owner, group and mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perms {
    /// The permission bits (`0o7777`: setuid, setgid, sticky, then owner / group / others rwx).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// The owner's account name, else the uid.
    pub owner: String,
    /// The group's name, else the gid.
    pub group: String,
    pub is_dir: bool,
}

/// This process's (effective) user id.
pub fn my_uid() -> u32 {
    // SAFETY: geteuid can't fail
    unsafe { libc::geteuid() }
}

impl Perms {
    /// Whether this user may change the item's permissions and group (its owner, or root).
    pub fn changeable(&self) -> bool {
        let me = my_uid();
        self.uid == me || me == 0
    }
}

/// The item's permissions (a link's target's, as `chmod` changes those).
pub fn read(path: &Path) -> io::Result<Perms> {
    let m = std::fs::metadata(path)?;
    Ok(Perms {
        mode: m.mode() & 0o7777,
        uid: m.uid(),
        gid: m.gid(),
        owner: user_name(m.uid()).unwrap_or_else(|| m.uid().to_string()),
        group: group_name(m.gid()).unwrap_or_else(|| m.gid().to_string()),
        is_dir: m.is_dir(),
    })
}

/// The account name for `uid` (through NSS, so LDAP and systemd-homed users too).
pub fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    loop {
        // SAFETY: every pointer is to a live local of the right type and size
        let r = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr().cast(), buf.len(), &mut out) };
        if r == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 4, 0);
            continue;
        }
        if r != 0 || out.is_null() {
            return None;
        }
        // SAFETY: getpwuid_r succeeded, so pw_name points at a NUL-terminated string in buf
        return Some(unsafe { CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned());
    }
}

/// The name of group `gid` (through NSS).
pub fn group_name(gid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::group = std::ptr::null_mut();
    loop {
        // SAFETY: as in user_name
        let r = unsafe { libc::getgrgid_r(gid, &mut gr, buf.as_mut_ptr().cast(), buf.len(), &mut out) };
        if r == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 4, 0);
            continue;
        }
        if r != 0 || out.is_null() {
            return None;
        }
        // SAFETY: getgrgid_r succeeded, so gr_name points at a NUL-terminated string in buf
        return Some(unsafe { CStr::from_ptr(gr.gr_name) }.to_string_lossy().into_owned());
    }
}

/// [`user_name`], remembered (for list columns: one lookup per user, not per row); the uid
/// when it has no name.
pub fn user_label(uid: u32) -> std::sync::Arc<str> {
    static CACHE: std::sync::Mutex<Vec<(u32, std::sync::Arc<str>)>> = std::sync::Mutex::new(Vec::new());
    cached(&CACHE, uid, || user_name(uid))
}

/// [`group_name`], remembered; the gid when it has no name.
pub fn group_label(gid: u32) -> std::sync::Arc<str> {
    static CACHE: std::sync::Mutex<Vec<(u32, std::sync::Arc<str>)>> = std::sync::Mutex::new(Vec::new());
    cached(&CACHE, gid, || group_name(gid))
}

fn cached(cache: &std::sync::Mutex<Vec<(u32, std::sync::Arc<str>)>>, id: u32, look: impl FnOnce() -> Option<String>) -> std::sync::Arc<str> {
    if let Some((_, n)) = cache.lock().unwrap().iter().find(|(i, _)| *i == id) {
        return n.clone();
    }
    let n: std::sync::Arc<str> = look().unwrap_or_else(|| id.to_string()).into();
    cache.lock().unwrap().push((id, n.clone()));
    n
}

/// The groups this user is in (the ones a file of theirs can be given to), sorted by name.
pub fn my_groups() -> Vec<(u32, String)> {
    // SAFETY: a zero-length call only returns the count
    let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let mut ids = vec![0 as libc::gid_t; n.max(0) as usize];
    // SAFETY: ids has room for n entries
    let n = unsafe { libc::getgroups(ids.len() as libc::c_int, ids.as_mut_ptr()) };
    ids.truncate(n.max(0) as usize);
    // SAFETY: getegid can't fail
    ids.push(unsafe { libc::getegid() });
    ids.sort_unstable();
    ids.dedup();
    let mut out: Vec<(u32, String)> = ids.into_iter().map(|g| (g, group_name(g).unwrap_or_else(|| g.to_string()))).collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Bits to turn on and off. Applied to a folder's contents, the execute bits change only on
/// folders (where they mean "may open"): a folder's tree shouldn't turn every file into a
/// program, or take that away from scripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Change {
    pub set: u32,
    pub clear: u32,
    /// A group to give the items to.
    pub gid: Option<u32>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.set == 0 && self.clear == 0 && self.gid.is_none()
    }

    /// The new mode for an item with `mode`; `inside` = reached through a folder's tree.
    pub fn mode(&self, mode: u32, is_dir: bool, inside: bool) -> u32 {
        let keep = if inside && !is_dir { 0o111 } else { 0 };
        (mode | (self.set & !keep)) & !(self.clear & !keep)
    }
}

fn apply_one(path: &Path, m: &std::fs::Metadata, ch: &Change, inside: bool) -> io::Result<bool> {
    let mut changed = false;
    if let Some(g) = ch.gid.filter(|g| *g != m.gid()) {
        std::os::unix::fs::chown(path, None, Some(g))?;
        changed = true;
    }
    let old = m.mode() & 0o7777;
    let new = ch.mode(old, m.is_dir(), inside);
    if new != old {
        // chown can clear setuid / setgid; set the whole mode again after it
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(new))?;
        changed = true;
    }
    Ok(changed)
}

/// Applies `ch` to one item (following a link to its target, as `chmod` does).
pub fn apply(path: &Path, ch: &Change) -> io::Result<()> {
    apply_one(path, &std::fs::metadata(path)?, ch, false).map(|_| ())
}

/// [`apply`] to a folder and everything under it, not following links (their targets may be
/// anywhere); the number of items changed.
pub fn apply_tree(dir: &Path, ch: &Change, cancel: &dyn Fn() -> bool) -> io::Result<usize> {
    let mut changed = usize::from(apply_one(dir, &std::fs::metadata(dir)?, ch, false)?);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            if cancel() {
                return Ok(changed);
            }
            let p = e?.path();
            let m = std::fs::symlink_metadata(&p)?;
            if m.file_type().is_symlink() {
                continue;
            }
            changed += usize::from(apply_one(&p, &m, ch, true)?);
            if m.is_dir() {
                stack.push(p);
            }
        }
    }
    Ok(changed)
}

/// `rwxr-xr-x`, with `s` / `S` / `t` / `T` for setuid, setgid and sticky as `ls` shows them.
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for (i, special, mark) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (mode >> i) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(match (bits & 1 != 0, mode & special != 0) {
            (true, true) => mark,
            (false, true) => mark.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_strings() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o640), "rw-r-----");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        assert_eq!(mode_string(0o2644), "rw-r-Sr--");
    }

    #[test]
    fn execute_bits_change_only_on_folders_inside_a_tree() {
        let ch = Change { set: 0o111, clear: 0o002, gid: None };
        assert_eq!(ch.mode(0o646, false, false), 0o755, "the item itself");
        assert_eq!(ch.mode(0o646, false, true), 0o644, "a file inside: read / write only");
        assert_eq!(ch.mode(0o646, true, true), 0o755, "a folder inside");
        assert_eq!(Change { set: 0, clear: 0o111, gid: None }.mode(0o755, false, true), 0o755, "scripts keep running");
    }

    #[test]
    fn reads_and_applies() {
        let d = std::env::temp_dir().join(format!("rf-perms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/script.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(d.join("sub/script.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(d.join("a.txt"), b"x").unwrap();
        std::fs::set_permissions(d.join("a.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();

        let p = read(&d.join("a.txt")).unwrap();
        assert_eq!(p.mode, 0o644);
        assert!(!p.is_dir);
        // SAFETY: getuid can't fail
        assert_eq!(p.uid, unsafe { libc::getuid() });
        assert!(!p.owner.is_empty() && !p.group.is_empty());

        apply(&d.join("a.txt"), &Change { set: 0o100, clear: 0o044, gid: None }).unwrap();
        assert_eq!(read(&d.join("a.txt")).unwrap().mode, 0o700);

        // others lose read everywhere; the script keeps its execute bits
        let n = apply_tree(&d, &Change { set: 0, clear: 0o005, gid: None }, &|| false).unwrap();
        assert_eq!(read(&d.join("sub/script.sh")).unwrap().mode, 0o751);
        assert_eq!(read(&d.join("sub")).unwrap().mode & 0o007, 0o000);
        assert!(n >= 3);

        // a group of our own is accepted
        let mine = my_groups();
        assert!(!mine.is_empty());
        apply(&d.join("a.txt"), &Change { gid: Some(mine[0].0), ..Default::default() }).unwrap();
        assert_eq!(read(&d.join("a.txt")).unwrap().gid, mine[0].0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_of_root() {
        assert_eq!(&*user_label(0), "root");
        assert_eq!(&*user_label(0), "root", "from the cache");
        assert_eq!(&*user_label(4_000_000_000), "4000000000", "no name: the id");
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert_eq!(read(Path::new("/")).unwrap().owner, "root");
    }
}
