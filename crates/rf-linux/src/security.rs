//! Who may do what with a file (its Windows DACL), for the Properties window's Security tab.
//! Linux has owner / group / other permissions instead, and the tab isn't shown; the types
//! match rf-win's.

use std::path::Path;

use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub sid: String,
    pub allow: u32,
    pub deny: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Security {
    pub owner: Option<String>,
    pub entries: Vec<Entry>,
    pub null_dacl: bool,
    pub protected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    Full,
    Modify,
    ReadExecute,
    List,
    Read,
    Write,
    Special,
}

pub const RIGHTS: [Right; 7] = [Right::Full, Right::Modify, Right::ReadExecute, Right::List, Right::Read, Right::Write, Right::Special];

/// Linux: the file's owner (a user name, else the uid); no access-control entries.
pub fn read(path: &Path) -> Result<Security> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::symlink_metadata(path)?.uid();
    Ok(Security { owner: Some(user_name(uid).unwrap_or_else(|| uid.to_string())), ..Default::default() })
}

/// The account name for `uid` from /etc/passwd (local accounts; LDAP / systemd-homed users show as the uid).
fn user_name(uid: u32) -> Option<String> {
    let text = std::fs::read_to_string("/etc/passwd").ok()?;
    text.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        (f.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_string())
    })
}

pub fn ticks(_mask: u32, _is_dir: bool) -> Vec<Right> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn owner_of_root_is_root() {
        assert_eq!(super::read(std::path::Path::new("/")).unwrap().owner.as_deref(), Some("root"));
    }
}
