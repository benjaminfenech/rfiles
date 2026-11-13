//! Searching inside files ("content:" queries) through the desktop's own file index: Baloo
//! (KDE Plasma) or Tracker / LocalSearch (GNOME). Both read PDFs, office documents and
//! plain text, and answer in milliseconds.
//!
//! An index is asked only when it is running and covers the whole folder being searched;
//! otherwise [`indexed`] gives None and the caller reads the files itself
//! ([`crate::search::search_contents`]). Both are asked through their command-line tools
//! (`baloosearch6`, `tinysparql`), which the Flatpak runs on the host like other tools.
//!
//! Blocking: call off the UI thread.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::sandbox;
use crate::tool;

/// Which index answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Baloo,
    Tracker,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Baloo => "Baloo",
            Source::Tracker => "Tracker",
        }
    }
}

/// The text a search box query looks for inside files: "content:budget" → "budget".
pub fn content_query(q: &str) -> Option<&str> {
    let q = q.trim_start();
    // up to the first colon, compared folded ("İÇERİK" is longer in bytes than "içerik")
    let (head, rest) = q.split_once(':')?;
    let head = crate::search::fold_str(head);
    ["content", "contents", "icerik"].contains(&head.replace('ç', "c").as_str()).then(|| rest.trim())
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// Files under `root` whose contents (or names) contain `text`, from a desktop index that
/// covers all of `root`. None: no index is running there, so read the files instead.
pub fn indexed(root: &Path, text: &str, max: usize) -> Option<(Source, Vec<PathBuf>)> {
    if text.trim().is_empty() {
        return None;
    }
    if bus_name_owned("org.kde.baloo") && baloo_covers(root, &baloo_config()) {
        match baloo(root, text, max) {
            Ok(v) => return Some((Source::Baloo, v)),
            Err(e) => log::info!("content search: Baloo: {e:#}"),
        }
    }
    let tracker = ["org.freedesktop.LocalSearch3", "org.freedesktop.Tracker3.Miner.Files"].into_iter().find(|n| bus_name_owned(n));
    if let Some(service) = tracker
        && tracker_covers(root, &tracker_dirs())
    {
        match tracker_search(service, root, text, max) {
            Ok(v) => return Some((Source::Tracker, v)),
            Err(e) => log::info!("content search: Tracker: {e:#}"),
        }
    }
    None
}

/// Whether a program owns `name` on the session bus. Only asks: never starts the service
/// (asking Tracker directly would start its indexer on desktops that keep it off).
fn bus_name_owned(name: &str) -> bool {
    if sandbox::is_flatpak() {
        // the sandbox's bus proxy hides names rfiles may not talk to: ask the host's bus
        let mut c = sandbox::tool("gdbus", &[]);
        c.args(["call", "--session", "--dest", "org.freedesktop.DBus", "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.NameHasOwner", name]);
        return tool::run(c, Duration::from_secs(3)).is_ok_and(|r| r.ok && String::from_utf8_lossy(&r.stdout).contains("true"));
    }
    let Ok(conn) = zbus::blocking::Connection::session() else { return false };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    let Ok(name) = zbus::names::BusName::try_from(name) else { return false };
    dbus.name_has_owner(name).unwrap_or(false)
}

/// The words to look for, without the characters the indexes' query languages treat
/// specially (quotes, operators, brackets).
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\'')).collect::<String>())
        .map(|w| w.trim_matches(|c| matches!(c, '-' | '.' | '\'')).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn under(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir)
}

// --- Baloo ----------------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct BalooConfig {
    enabled: bool,
    contents: bool,
    folders: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
}

fn baloo_config() -> BalooConfig {
    let text = dirs::config_dir().and_then(|d| std::fs::read_to_string(d.join("baloofilerc")).ok()).unwrap_or_default();
    parse_baloo_config(&text, &dirs::home_dir().unwrap_or_default())
}

/// `baloofilerc`; a missing key keeps Baloo's default (on, contents too, the home folder).
fn parse_baloo_config(text: &str, home: &Path) -> BalooConfig {
    let mut c = BalooConfig { enabled: true, contents: true, folders: vec![home.to_path_buf()], excluded: Vec::new() };
    let mut section = String::new();
    let list = |v: &str| -> Vec<PathBuf> {
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                let s = s.replace("$HOME", &home.to_string_lossy());
                PathBuf::from(s.trim_end_matches('/'))
            })
            .map(|p| if p.as_os_str().is_empty() { PathBuf::from("/") } else { p })
            .collect()
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        // "folders[$e]": the [$e] marks a value with $VARIABLES in it
        let k = k.trim().trim_end_matches("[$e]");
        let v = v.trim();
        match (section.as_str(), k) {
            ("Basic Settings", "Indexing-Enabled") => c.enabled = v != "false",
            ("General", "only basic indexing") => c.contents = v != "true",
            ("General", "folders") => c.folders = list(v),
            ("General", "exclude folders") => c.excluded = list(v),
            _ => {}
        }
    }
    c
}

fn baloo_covers(root: &Path, c: &BalooConfig) -> bool {
    c.enabled && c.contents && c.folders.iter().any(|f| under(root, f)) && !c.excluded.iter().any(|e| under(root, e))
}

fn baloo(root: &Path, text: &str, max: usize) -> anyhow::Result<Vec<PathBuf>> {
    let q = words(text).join(" ");
    anyhow::ensure!(!q.is_empty(), "nothing to look for");
    // Plasma 6's tool; Plasma 5 named it without the 6
    let prog = if sandbox::in_path("baloosearch6") { "baloosearch6" } else { "baloosearch" };
    let mut c: Command = sandbox::tool(prog, &[]);
    c.arg("-d").arg(tool::path_arg(root)).arg("-l").arg(max.to_string()).arg("--").arg(&q);
    let r = tool::run(c, TIMEOUT)?;
    anyhow::ensure!(r.ok, "{}", String::from_utf8_lossy(&r.stderr).trim());
    Ok(parse_baloo_output(&r.stdout, root))
}

/// One path a line, then "Elapsed: … msecs".
fn parse_baloo_output(out: &[u8], root: &Path) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    out.split(|&b| b == b'\n')
        .filter(|l| l.starts_with(b"/"))
        .map(|l| PathBuf::from(std::ffi::OsStr::from_bytes(l)))
        .filter(|p| under(p, root))
        .collect()
}

// --- Tracker / LocalSearch ------------------------------------------------------------------

/// The folders Tracker indexes with their subfolders (its GSettings), None when they can't be
/// read.
fn tracker_dirs() -> Option<Vec<PathBuf>> {
    let mut c = sandbox::tool("gsettings", &[]);
    c.args(["get", "org.freedesktop.Tracker3.Miner.Files", "index-recursive-directories"]);
    let r = tool::run(c, Duration::from_secs(3)).ok().filter(|r| r.ok)?;
    Some(parse_tracker_dirs(&String::from_utf8_lossy(&r.stdout), &dirs::home_dir().unwrap_or_default()))
}

/// `['&DOCUMENTS', '$HOME', '/data']`: the special names are the XDG user folders.
fn parse_tracker_dirs(v: &str, home: &Path) -> Vec<PathBuf> {
    v.split('\'')
        .skip(1)
        .step_by(2)
        .filter_map(|s| match s {
            "$HOME" => Some(home.to_path_buf()),
            "&DESKTOP" => dirs::desktop_dir(),
            "&DOCUMENTS" => dirs::document_dir(),
            "&DOWNLOAD" => dirs::download_dir(),
            "&MUSIC" => dirs::audio_dir(),
            "&PICTURES" => dirs::picture_dir(),
            "&VIDEOS" => dirs::video_dir(),
            "&PUBLIC_SHARE" => dirs::public_dir(),
            "&TEMPLATES" => dirs::template_dir(),
            s if s.starts_with('/') => Some(PathBuf::from(s)),
            s if s.starts_with("~/") => Some(home.join(&s[2..])),
            _ => None,
        })
        .collect()
}

fn tracker_covers(root: &Path, dirs: &Option<Vec<PathBuf>>) -> bool {
    dirs.as_ref().is_some_and(|d| d.iter().any(|d| under(root, d)))
}

/// A `file://` URL prefix for `root` made only of characters Tracker never escapes, so the
/// query's filter is right whatever the rest of the path holds (the exact check is done on
/// the decoded paths).
fn safe_url_prefix(root: &Path) -> String {
    let s = root.to_string_lossy();
    let safe: String = s.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '~')).collect();
    format!("file://{safe}")
}

fn tracker_sparql(root: &Path, text: &str, max: usize) -> Option<String> {
    let w = words(text);
    if w.is_empty() {
        return None;
    }
    // FTS: every word, the last one also as the start of a longer word
    let fts: String = w.iter().map(|w| format!("{}*", w.replace('\'', ""))).collect::<Vec<_>>().join(" ");
    Some(format!(
        "SELECT DISTINCT ?url WHERE {{ ?ie fts:match \"{fts}\" ; nie:isStoredAs ?f . ?f nie:url ?url . FILTER (STRSTARTS(?url, \"{}\")) }} LIMIT {max}",
        safe_url_prefix(root)
    ))
}

fn tracker_search(service: &str, root: &Path, text: &str, max: usize) -> anyhow::Result<Vec<PathBuf>> {
    let q = tracker_sparql(root, text, max).ok_or_else(|| anyhow::anyhow!("nothing to look for"))?;
    // TinySPARQL 3.8+; before it, the tracker3 tool
    let c = if sandbox::in_path("tinysparql") {
        let mut c = sandbox::tool("tinysparql", &[]);
        c.args(["query", "--dbus-service", service]).arg(&q);
        c
    } else {
        let mut c = sandbox::tool("tracker3", &[]);
        c.args(["sparql", "--dbus-service", service, "-q"]).arg(&q);
        c
    };
    let r = tool::run(c, TIMEOUT)?;
    anyhow::ensure!(r.ok, "{}", String::from_utf8_lossy(&r.stderr).trim());
    Ok(parse_tracker_output(&String::from_utf8_lossy(&r.stdout), root))
}

/// "Results:" then one indented `file://` URL a line.
fn parse_tracker_output(out: &str, root: &Path) -> Vec<PathBuf> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("file://"))
        .filter_map(crate::fm_service::uri_to_path)
        .filter(|p| under(p, root))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_prefixes() {
        assert_eq!(content_query("content:quarterly budget"), Some("quarterly budget"));
        assert_eq!(content_query("Content: x"), Some("x"));
        assert_eq!(content_query("İÇERİK:rapor"), Some("rapor"));
        assert_eq!(content_query("budget"), None);
        assert_eq!(content_query("tag:red"), None);
    }

    #[test]
    fn query_words_lose_operators() {
        assert_eq!(words(r#"quarterly "budget" (2026) a*b -x"#), vec!["quarterly", "budget", "2026", "ab", "x"]);
        assert!(words("\"*\"").is_empty());
    }

    #[test]
    fn baloo_config_and_coverage() {
        let home = Path::new("/home/u");
        let d = parse_baloo_config("", home);
        assert!(baloo_covers(Path::new("/home/u/Documents"), &d));
        assert!(!baloo_covers(Path::new("/mnt/data"), &d));
        let c = parse_baloo_config(
            "[General]\nfolders[$e]=$HOME/,/mnt/data/\nexclude folders[$e]=$HOME/build/\nonly basic indexing=false\n[Basic Settings]\nIndexing-Enabled=true\n",
            home,
        );
        assert!(baloo_covers(Path::new("/mnt/data/x"), &c));
        assert!(!baloo_covers(Path::new("/home/u/build/obj"), &c));
        assert!(!baloo_covers(Path::new("/home/u"), &parse_baloo_config("[General]\nonly basic indexing=true\n", home)));
        assert!(!baloo_covers(Path::new("/home/u"), &parse_baloo_config("[Basic Settings]\nIndexing-Enabled=false\n", home)));
    }

    #[test]
    fn baloo_output() {
        let out = b"/home/u/a b/plan.txt\n/home/u/Zan&zibar.md\n/other/x\nElapsed: 0.05 msecs\n";
        assert_eq!(parse_baloo_output(out, Path::new("/home/u")), vec![PathBuf::from("/home/u/a b/plan.txt"), PathBuf::from("/home/u/Zan&zibar.md")]);
    }

    #[test]
    fn tracker_dirs_and_output() {
        let home = Path::new("/home/u");
        assert_eq!(parse_tracker_dirs("['$HOME', '/data']\n", home), vec![PathBuf::from("/home/u"), PathBuf::from("/data")]);
        assert!(parse_tracker_dirs("@as []\n", home).is_empty());
        assert!(!tracker_covers(Path::new("/home/u"), &None));
        let out = "Results:\n  file:///home/u/Documents/Zanzibar.md\n  file:///home/u/My%20%C4%9E%20dir/a&b.txt\n  file:///home/uu/x\n\n";
        assert_eq!(
            parse_tracker_output(out, Path::new("/home/u")),
            vec![PathBuf::from("/home/u/Documents/Zanzibar.md"), PathBuf::from("/home/u/My Ğ dir/a&b.txt")]
        );
    }

    #[test]
    fn tracker_query_filters_on_a_safe_prefix() {
        let q = tracker_sparql(Path::new("/home/u/My Ğ dir"), "quarterly budg", 100).unwrap();
        assert!(q.contains(r#"fts:match "quarterly* budg*""#), "{q}");
        assert!(q.contains(r#"STRSTARTS(?url, "file:///home/u/My")"#), "{q}");
        assert!(q.ends_with("LIMIT 100"));
    }
}
