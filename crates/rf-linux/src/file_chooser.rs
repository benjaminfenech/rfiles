//! rfiles as other apps' Open/Save dialog: a backend for xdg-desktop-portal's FileChooser
//! (`org.freedesktop.impl.portal.FileChooser`).
//!
//! Apps ask the portal (`org.freedesktop.portal.FileChooser`, or GTK/Qt/Electron/Firefox do it
//! for them); the portal passes the call to the backend `portals.conf` names for FileChooser.
//! The packages install `rfiles.portal` (so the portal knows the backend) and a D-Bus service
//! file that starts `rfiles --file-chooser-service`; [`set_enabled`] names rfiles in the
//! user's `portals.conf`.
//!
//! [`serve`] answers the calls: each one becomes a [`Request`] for the app's picker window,
//! and the call returns when the picker sends its [`Outcome`] (or the portal closes the
//! request).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::channel::oneshot;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// The backend's bus name, also the `DBusName` in `rfiles.portal`.
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.rfiles";
/// What D-Bus starts rfiles with for it.
pub const SERVICE_ARG: &str = "--file-chooser-service";
/// The portal's name for the backend: the `.portal` file's name.
pub const PORTAL: &str = "rfiles";
const IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
const OBJECT: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Open,
    Save,
    /// Saving several files into a folder the user picks (`SaveFiles`).
    SaveFiles,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

/// A file type the app offers ("Images": `*.png`, `image/jpeg`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    /// As `*.png;*.jpg`, MIME types through shared-mime-info. Empty = every file.
    pub fn glob_spec(&self) -> String {
        let mut globs: Vec<String> = Vec::new();
        for p in &self.patterns {
            match p {
                Pattern::Glob(g) => globs.push(g.clone()),
                Pattern::Mime(m) if m == "*" || m == "*/*" || m == "application/octet-stream" => return String::new(),
                Pattern::Mime(m) => globs.extend(crate::xdg_icons::exts_for_mime(m).into_iter().map(|e| format!("*.{e}"))),
            }
        }
        if globs.iter().any(|g| g == "*" || g == "*.*") {
            return String::new();
        }
        globs.join(";")
    }

    fn to_value(&self) -> Value<'static> {
        let pats: Vec<(u32, String)> = self
            .patterns
            .iter()
            .map(|p| match p {
                Pattern::Glob(g) => (0, g.clone()),
                Pattern::Mime(m) => (1, m.clone()),
            })
            .collect();
        Value::from((self.name.clone(), pats))
    }
}

/// An app's extra option in the dialog (`choices`). rfiles shows none and answers each with
/// its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub default: String,
}

/// One Open/Save call, as the picker needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub kind: Kind,
    pub app_id: String,
    /// `wayland:<handle>` / `x11:<xid>`, or empty.
    pub parent_window: String,
    pub title: String,
    pub accept_label: Option<String>,
    pub multiple: bool,
    /// Picking folders instead of files.
    pub directory: bool,
    pub filters: Vec<Filter>,
    /// Which of [`filters`](Self::filters) starts selected.
    pub current_filter: Option<usize>,
    /// Save: the suggested name.
    pub current_name: Option<String>,
    pub current_folder: Option<PathBuf>,
    /// Save: the file being saved again ("Save As" of an existing file).
    pub current_file: Option<PathBuf>,
    /// SaveFiles: the names that will be written into the picked folder.
    pub files: Vec<String>,
    pub choices: Vec<Choice>,
}

/// How the picker ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Files (or folders) chosen, and the filter shown at the time.
    Picked { paths: Vec<PathBuf>, filter: Option<usize> },
    Cancelled,
}

/// What the app does with a call: show a picker for the request, send its outcome on the
/// sender, and close the picker when the receiver fires (the portal took the request back).
pub type Handler = Arc<dyn Fn(Request, oneshot::Sender<Outcome>, oneshot::Receiver<()>) + Send + Sync>;

fn get<'a>(o: &'a HashMap<String, OwnedValue>, k: &str) -> Option<&'a Value<'static>> {
    o.get(k).map(|v| &**v)
}
fn string(o: &HashMap<String, OwnedValue>, k: &str) -> Option<String> {
    get(o, k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).filter(|s| !s.is_empty())
}
fn boolean(o: &HashMap<String, OwnedValue>, k: &str) -> bool {
    get(o, k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}
/// An `ay` path: bytes with a trailing NUL.
fn bytes_path(v: &Value<'_>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut b: Vec<u8> = Vec::try_from(v.try_clone().ok()?).ok()?;
    while b.last() == Some(&0) {
        b.pop();
    }
    (!b.is_empty()).then(|| PathBuf::from(std::ffi::OsString::from_vec(b)))
}
fn filter_of(v: Value<'_>) -> Option<Filter> {
    let (name, pats): (String, Vec<(u32, String)>) = v.try_into().ok()?;
    let patterns = pats.into_iter().map(|(t, p)| if t == 1 { Pattern::Mime(p) } else { Pattern::Glob(p) }).collect();
    Some(Filter { name, patterns })
}

/// The call's options as a [`Request`].
pub fn parse(kind: Kind, app_id: String, parent_window: String, title: String, o: &HashMap<String, OwnedValue>) -> Request {
    let filters: Vec<Filter> = get(o, "filters")
        .and_then(|v| match v {
            Value::Array(a) => Some(a.iter().filter_map(|f| filter_of(f.try_clone().ok()?)).collect()),
            _ => None,
        })
        .unwrap_or_default();
    let mut filters = filters;
    let current_filter = get(o, "current_filter").and_then(|v| filter_of(v.try_clone().ok()?)).map(|cur| match filters.iter().position(|f| *f == cur) {
        Some(i) => i,
        None => {
            filters.push(cur);
            filters.len() - 1
        }
    });
    let choices = get(o, "choices")
        .and_then(|v| {
            // (id, label, options, default)
            type Choices = Vec<(String, String, Vec<(String, String)>, String)>;
            let list: Choices = v.try_clone().ok()?.try_into().ok()?;
            Some(list.into_iter().map(|(id, _label, _options, default)| Choice { id, default }).collect())
        })
        .unwrap_or_default();
    let files = get(o, "files")
        .and_then(|v| match v {
            Value::Array(a) => Some(a.iter().filter_map(bytes_path).map(|p| p.to_string_lossy().into_owned()).collect()),
            _ => None,
        })
        .unwrap_or_default();
    Request {
        kind,
        app_id,
        parent_window,
        title,
        accept_label: string(o, "accept_label"),
        multiple: boolean(o, "multiple"),
        directory: boolean(o, "directory"),
        filters,
        current_filter,
        current_name: string(o, "current_name"),
        current_folder: get(o, "current_folder").and_then(bytes_path),
        current_file: get(o, "current_file").and_then(bytes_path),
        files,
        choices,
    }
}

/// A path as a `file://` URI (each byte outside the unreserved set escaped).
pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The call's answer: (response, results). 0 = done, 1 = cancelled.
pub fn results(req: &Request, outcome: &Outcome) -> (u32, HashMap<String, OwnedValue>) {
    let mut r: HashMap<String, OwnedValue> = HashMap::new();
    let Outcome::Picked { paths, filter } = outcome else { return (1, r) };
    let mut put = |k: &str, v: Value<'static>| {
        if let Ok(v) = OwnedValue::try_from(v) {
            r.insert(k.to_string(), v);
        }
    };
    // SaveFiles: one URI per name, in the picked folder
    let uris: Vec<String> = match (req.kind, paths.first()) {
        (Kind::SaveFiles, Some(dir)) => req.files.iter().map(|f| file_uri(&dir.join(f))).collect(),
        _ => paths.iter().map(|p| file_uri(p)).collect(),
    };
    put("uris", Value::from(uris));
    if let Some(f) = filter.and_then(|i| req.filters.get(i)) {
        put("current_filter", f.to_value());
    }
    if !req.choices.is_empty() {
        let c: Vec<(String, String)> = req.choices.iter().map(|c| (c.id.clone(), c.default.clone())).collect();
        put("choices", Value::from(c));
    }
    if req.kind == Kind::Open {
        put("writable", Value::from(true));
    }
    (0, r)
}

/// A call in progress, as the portal sees it: closing it ends the picker.
struct RequestObject {
    close: Mutex<Option<oneshot::Sender<()>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    fn close(&self) {
        if let Some(tx) = self.close.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
    }
}

struct Chooser {
    handler: Handler,
}

impl Chooser {
    async fn run(&self, conn: &zbus::Connection, handle: OwnedObjectPath, req: Request) -> (u32, HashMap<String, OwnedValue>) {
        let (close_tx, close_rx) = oneshot::channel();
        let exported = conn.object_server().at(&handle, RequestObject { close: Mutex::new(Some(close_tx)) }).await.unwrap_or(false);
        let (tx, rx) = oneshot::channel();
        (self.handler)(req.clone(), tx, close_rx);
        let outcome = rx.await.unwrap_or(Outcome::Cancelled);
        if exported {
            let _ = conn.object_server().remove::<RequestObject, _>(&handle).await;
        }
        log::info!("file chooser: {} for {}: {:?}", req.title, req.app_id, matches!(outcome, Outcome::Picked { .. }));
        results(&req, &outcome)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl Chooser {
    #[allow(clippy::too_many_arguments)]
    async fn open_file(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(conn, handle, parse(Kind::Open, app_id, parent_window, title, &options)).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_file(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.run(conn, handle, parse(Kind::Save, app_id, parent_window, title, &options)).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_files(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let mut req = parse(Kind::SaveFiles, app_id, parent_window, title, &options);
        // a folder is picked
        req.directory = true;
        self.run(conn, handle, req).await
    }
}

/// Takes the backend's bus name and answers calls with `handler`, on a thread of its own.
/// Fails when another rfiles already serves it.
pub fn serve(handler: Handler) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    std::thread::Builder::new().name("rf-file-chooser".into()).spawn(move || {
        let built = futures::executor::block_on(async {
            zbus::connection::Builder::session()?.name(BUS_NAME)?.serve_at(OBJECT, Chooser { handler })?.build().await
        });
        match built {
            Ok(conn) => {
                log::info!("serving {BUS_NAME} ({IFACE})");
                let _ = tx.send(Ok(()));
                // the connection answers on zbus' own executor thread; keep it alive
                let _conn = conn;
                loop {
                    std::thread::park();
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e.into()));
            }
        }
    })?;
    rx.recv().unwrap_or_else(|_| Err(anyhow::anyhow!("the D-Bus thread ended")))
}

// --- choosing rfiles in portals.conf ----------------------------------------------------------

/// Where the portal finds backends (`rfiles.portal` is installed by the packages).
fn portal_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("XDG_DESKTOP_PORTAL_DIR").map(PathBuf::from).into_iter().collect();
    v.extend(["/usr/share/xdg-desktop-portal/portals", "/usr/local/share/xdg-desktop-portal/portals"].map(PathBuf::from));
    v
}

/// Whether the portal can use rfiles at all: `rfiles.portal` is installed (the deb, rpm and
/// Arch packages; not the Flatpak, which can't add portal backends).
pub fn installed() -> bool {
    portal_dirs().iter().any(|d| d.join(format!("{PORTAL}.portal")).is_file())
}

/// The `portals.conf` the portal reads for this desktop: `<desktop>-portals.conf` when the
/// user has one, else `portals.conf` (both in `~/.config/xdg-desktop-portal`).
fn config_file() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("xdg-desktop-portal");
    let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    for d in desktops.split(':').filter(|d| !d.is_empty()) {
        let f = dir.join(format!("{}-portals.conf", d.to_lowercase()));
        if f.is_file() {
            return Some(f);
        }
    }
    Some(dir.join("portals.conf"))
}

const KEY: &str = "org.freedesktop.impl.portal.FileChooser";

/// Whether `portals.conf` text names rfiles for FileChooser.
fn names_rfiles(text: &str) -> bool {
    let mut preferred = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            preferred = line == "[preferred]";
        } else if preferred
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == KEY
        {
            return v.split(';').next().map(str::trim) == Some(PORTAL);
        }
    }
    false
}

/// `text` with the FileChooser line set to rfiles (`on`) or removed. The rest stays as it was.
fn edit_config(text: &str, on: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_preferred = false;
    let mut had_preferred = false;
    let mut done = false;
    let line_for = || format!("{KEY}={PORTAL}");
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_preferred && on && !done {
                // end of [preferred] without the key: add it here
                while out.last().is_some_and(|l| l.trim().is_empty()) {
                    out.pop();
                }
                out.push(line_for());
                out.push(String::new());
                done = true;
            }
            in_preferred = t == "[preferred]";
            had_preferred |= in_preferred;
            out.push(line.to_string());
            continue;
        }
        if in_preferred && t.split_once('=').is_some_and(|(k, _)| k.trim() == KEY) {
            if on && !done {
                out.push(line_for());
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if on && !done {
        if in_preferred {
            while out.last().is_some_and(|l| l.trim().is_empty()) {
                out.pop();
            }
        } else {
            if !out.is_empty() && !out.last().is_some_and(|l| l.trim().is_empty()) {
                out.push(String::new());
            }
            if !had_preferred || !in_preferred {
                out.push("[preferred]".into());
            }
            // a [preferred] without `default` would leave every other portal unset
            if !had_preferred {
                out.push("default=*".into());
            }
        }
        out.push(line_for());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Whether rfiles is the Open/Save dialog now.
pub fn enabled() -> bool {
    config_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|t| names_rfiles(&t))
}

/// Makes rfiles the Open/Save dialog (or gives the dialog back), then restarts the portal so
/// apps get it from their next dialog on.
pub fn set_enabled(on: bool) -> anyhow::Result<()> {
    let f = config_file().ok_or_else(|| anyhow::anyhow!("no config folder"))?;
    let old = std::fs::read_to_string(&f).unwrap_or_default();
    // a desktop-specific file the user doesn't have yet: start from the system's
    let base = if old.is_empty() && on { system_config().unwrap_or_default() } else { old.clone() };
    let new = edit_config(&base, on);
    if new != old {
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&f, new)?;
    }
    restart_portal();
    Ok(())
}

/// The system's `portals.conf` for this desktop (`/usr/share/xdg-desktop-portal/hyprland-portals.conf`).
fn system_config() -> Option<String> {
    let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    for dir in ["/usr/share/xdg-desktop-portal", "/etc/xdg-desktop-portal", "/usr/local/share/xdg-desktop-portal"] {
        for d in desktops.split(':').filter(|d| !d.is_empty()) {
            if let Ok(t) = std::fs::read_to_string(Path::new(dir).join(format!("{}-portals.conf", d.to_lowercase()))) {
                return Some(t);
            }
        }
    }
    None
}

/// The portal reads `portals.conf` when it starts.
fn restart_portal() {
    let mut c = crate::sandbox::tool("systemctl", &[]);
    c.args(["--user", "try-restart", "xdg-desktop-portal.service"]);
    if let Err(e) = crate::tool::run(c, std::time::Duration::from_secs(10)) {
        log::info!("restarting xdg-desktop-portal: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v).unwrap())).collect()
    }

    fn ay(s: &str) -> Value<'static> {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        Value::from(b)
    }

    #[test]
    fn options_are_read() {
        let images = (String::from("Images"), vec![(0u32, String::from("*.png")), (1u32, String::from("image/jpeg"))]);
        let text = (String::from("Text"), vec![(0u32, String::from("*.txt"))]);
        let o = opts(vec![
            ("accept_label", Value::from("_Upload")),
            ("multiple", Value::from(true)),
            ("filters", Value::from(vec![images.clone(), text.clone()])),
            ("current_filter", Value::from(text)),
            ("current_folder", ay("/home/u/My Ğ")),
            ("choices", Value::from(vec![(String::from("enc"), String::from("Encoding"), vec![(String::from("utf8"), String::from("UTF-8"))], String::from("utf8"))])),
        ]);
        let r = parse(Kind::Open, "org.app".into(), "wayland:x".into(), "Open".into(), &o);
        assert_eq!(r.accept_label.as_deref(), Some("_Upload"));
        assert!(r.multiple && !r.directory);
        assert_eq!(r.filters.len(), 2);
        assert_eq!(r.filters[0].patterns, vec![Pattern::Glob("*.png".into()), Pattern::Mime("image/jpeg".into())]);
        assert_eq!(r.current_filter, Some(1));
        assert_eq!(r.current_folder, Some(PathBuf::from("/home/u/My Ğ")));
        assert_eq!(r.choices, vec![Choice { id: "enc".into(), default: "utf8".into() }]);
        // an unknown current filter is added to the list
        let other = (String::from("All"), vec![(0u32, String::from("*"))]);
        let r = parse(Kind::Open, String::new(), String::new(), String::new(), &opts(vec![("current_filter", Value::from(other))]));
        assert_eq!((r.filters.len(), r.current_filter), (1, Some(0)));
        assert_eq!(r.filters[0].glob_spec(), "");
    }

    #[test]
    fn answers() {
        let mut req = parse(Kind::Save, String::new(), String::new(), String::new(), &opts(vec![("current_name", Value::from("a b.txt"))]));
        assert_eq!(req.current_name.as_deref(), Some("a b.txt"));
        let (code, r) = results(&req, &Outcome::Picked { paths: vec![PathBuf::from("/tmp/a b#.txt")], filter: None });
        assert_eq!(code, 0);
        let uris: Vec<String> = r["uris"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(uris, vec!["file:///tmp/a%20b%23.txt"]);
        assert_eq!(results(&req, &Outcome::Cancelled).0, 1);
        req.kind = Kind::SaveFiles;
        req.files = vec!["x.png".into(), "y.png".into()];
        let (_, r) = results(&req, &Outcome::Picked { paths: vec![PathBuf::from("/d")], filter: None });
        let uris: Vec<String> = r["uris"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(uris, vec!["file:///d/x.png", "file:///d/y.png"]);
    }

    #[test]
    fn portals_conf_edits() {
        let hypr = "[preferred]\ndefault=hyprland;gtk\n";
        let on = edit_config(hypr, true);
        assert_eq!(on, "[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=rfiles\n");
        assert!(names_rfiles(&on));
        assert_eq!(edit_config(&on, false), hypr);
        // replaced, not added twice
        let kde = "[preferred]\ndefault=kde\norg.freedesktop.impl.portal.FileChooser=kde\n\n[other]\nx=1\n";
        let on = edit_config(kde, true);
        assert_eq!(on.matches("FileChooser").count(), 1);
        assert!(names_rfiles(&on) && on.contains("[other]\nx=1"));
        // no file yet
        let on = edit_config("", true);
        assert_eq!(on, "[preferred]\ndefault=*\norg.freedesktop.impl.portal.FileChooser=rfiles\n");
        assert!(!names_rfiles("[preferred]\norg.freedesktop.impl.portal.FileChooser=gtk;rfiles\n"));
    }

    #[test]
    fn mime_filters_become_globs() {
        let f = Filter { name: "x".into(), patterns: vec![Pattern::Glob("*.md".into()), Pattern::Mime("image/png".into())] };
        let spec = f.glob_spec();
        assert!(spec.starts_with("*.md;") && spec.contains("*.png"), "{spec}");
    }
}
