//! The file clipboard (copy / cut / paste of files), shared with other apps the way Linux file
//! managers do it:
//!
//! * `x-special/gnome-copied-files`: `copy` or `cut`, then one `file://` URI per line
//!   (GNOME, Nemo, Thunar, Caja, PCManFM);
//! * `text/uri-list` plus `application/x-kde-cutselection` = `1` for a cut (KDE / Dolphin,
//!   browsers, editors);
//! * plain text: the paths, one per line (terminals, text fields).
//!
//! On Wayland through the data-control protocol (wlroots compositors like Hyprland and Sway,
//! KDE; not GNOME), on X11 as the CLIPBOARD selection owner. Where neither works, copy and
//! paste stay inside rfiles (the in-process copy below).

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crate::fileops::Transfer;

const GNOME: &str = "x-special/gnome-copied-files";
const URI_LIST: &str = "text/uri-list";
const KDE_CUT: &str = "application/x-kde-cutselection";
/// What `clear` offers: nothing anyone pastes, but we own the clipboard (so `sequence`
/// notices when someone else copies).
const EMPTY: &str = "application/x-rfiles-cleared";
/// How long a reader waits for the clipboard's owner.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// What we last put on the clipboard (the fallback where the system clipboard isn't reachable).
static FILES: Mutex<Option<(Vec<PathBuf>, Transfer)>> = Mutex::new(None);
static SEQUENCE: AtomicU32 = AtomicU32::new(1);
/// Which of our publications is current; an older owner losing the clipboard doesn't count.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Called when our publication `generation` stops owning the clipboard (someone else copied).
fn lost(generation: u64) {
    if GENERATION.load(Ordering::SeqCst) == generation {
        *FILES.lock().unwrap() = None;
        SEQUENCE.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Backend {
    Wayland,
    X11,
    None,
}

fn backend() -> Backend {
    let set = |v: &str| std::env::var_os(v).is_some_and(|s| !s.is_empty());
    if set("WAYLAND_DISPLAY") {
        Backend::Wayland
    } else if set("DISPLAY") {
        Backend::X11
    } else {
        Backend::None
    }
}

// ---------------------------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------------------------

fn payloads(paths: &[PathBuf], kind: Transfer) -> Vec<(String, Vec<u8>)> {
    let uris: Vec<String> = paths.iter().map(|p| crate::xdg_thumbs::file_uri(&std::path::absolute(p).unwrap_or_else(|_| p.clone()))).collect();
    let verb = if kind == Transfer::Move { "cut" } else { "copy" };
    let text = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
    let mut out = vec![
        (GNOME.to_string(), format!("{verb}\n{}", uris.join("\n")).into_bytes()),
        // RFC 2483: CRLF line ends
        (URI_LIST.to_string(), uris.iter().map(|u| format!("{u}\r\n")).collect::<String>().into_bytes()),
        (KDE_CUT.to_string(), if kind == Transfer::Move { b"1".to_vec() } else { b"0".to_vec() }),
    ];
    for t in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"] {
        out.push((t.to_string(), text.clone().into_bytes()));
    }
    out
}

/// `file:///a%20b` → `/a b`; other schemes and remote hosts → None.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    rf_core::uri::file_uri_to_path(uri)
}

/// The local files of a `text/uri-list` (comments and other schemes skipped).
fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(data).lines().filter(|l| !l.starts_with('#')).filter_map(uri_to_path).collect()
}

/// `copy|cut` then URIs (GNOME's format).
fn parse_gnome(data: &[u8]) -> Option<(Vec<PathBuf>, Transfer)> {
    let text = String::from_utf8_lossy(data);
    let mut lines = text.lines();
    let kind = match lines.next()?.trim() {
        "cut" => Transfer::Move,
        "copy" => Transfer::Copy,
        _ => return None,
    };
    let files: Vec<PathBuf> = lines.filter_map(uri_to_path).collect();
    (!files.is_empty()).then_some((files, kind))
}

/// Reads the files on the clipboard through `types` / `read` (one backend's).
fn files_from(types: &[String], read: &dyn Fn(&str) -> anyhow::Result<Vec<u8>>) -> anyhow::Result<Option<(Vec<PathBuf>, Transfer)>> {
    let has = |t: &str| types.iter().any(|x| x == t);
    if has(GNOME) {
        if let Some(r) = parse_gnome(&read(GNOME)?) {
            return Ok(Some(r));
        }
    }
    if has(URI_LIST) {
        let files = parse_uri_list(&read(URI_LIST)?);
        if files.is_empty() {
            return Ok(None);
        }
        let cut = has(KDE_CUT) && read(KDE_CUT).is_ok_and(|d| d.first() == Some(&b'1'));
        return Ok(Some((files, if cut { Transfer::Move } else { Transfer::Copy })));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------------------------
// Wayland (data-control)
// ---------------------------------------------------------------------------------------------

mod wayland {
    use super::*;
    use wl_clipboard_rs::{copy, paste};

    /// Takes the clipboard with `sources`, served from a thread until someone else copies.
    pub fn publish(sources: Vec<(String, Vec<u8>)>, generation: u64) -> anyhow::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("rf-clipboard".into()).spawn(move || {
            let sources = sources
                .into_iter()
                .map(|(m, b)| copy::MimeSource { source: copy::Source::Bytes(b.into_boxed_slice()), mime_type: copy::MimeType::Specific(m) })
                .collect();
            let mut opts = copy::Options::new();
            // this thread is the serving thread
            opts.foreground(true);
            match opts.prepare_copy_multi(sources) {
                Ok(prepared) => {
                    let _ = tx.send(Ok(()));
                    // returns once another client owns the clipboard
                    let _ = prepared.serve();
                    lost(generation);
                }
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("wayland clipboard: {e}")));
                }
            }
        })?;
        rx.recv_timeout(READ_TIMEOUT).map_err(|_| anyhow::anyhow!("wayland clipboard: no answer"))?
    }

    pub fn types() -> anyhow::Result<Vec<String>> {
        match paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified) {
            Ok(t) => Ok(t.into_iter().collect()),
            Err(paste::Error::ClipboardEmpty | paste::Error::NoSeats) => Ok(Vec::new()),
            Err(e) => Err(anyhow::anyhow!("wayland clipboard: {e}")),
        }
    }

    /// The clipboard's `mime` data; the owner writes it into a pipe, read here with a limit.
    pub fn read(mime: &str) -> anyhow::Result<Vec<u8>> {
        let (mut pipe, _) = paste::get_contents(paste::ClipboardType::Regular, paste::Seat::Unspecified, paste::MimeType::Specific(mime))
            .map_err(|e| anyhow::anyhow!("wayland clipboard: {e}"))?;
        // a hung owner never closes the pipe: read on a thread, give up after a while
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let r = (&mut pipe).take(16 << 20).read_to_end(&mut data).map(|_| data);
            let _ = tx.send(r);
        });
        Ok(rx.recv_timeout(READ_TIMEOUT).map_err(|_| anyhow::anyhow!("the clipboard's owner didn't answer"))??)
    }
}

// ---------------------------------------------------------------------------------------------
// X11 (CLIPBOARD selection)
// ---------------------------------------------------------------------------------------------

mod x11 {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, Window,
        WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

    fn atom(c: &RustConnection, name: &str) -> anyhow::Result<Atom> {
        Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
    }

    /// A connection and a hidden window to own or receive the selection with.
    fn setup() -> anyhow::Result<(RustConnection, Window)> {
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        let w = c.generate_id()?;
        c.create_window(COPY_DEPTH_FROM_PARENT, w, root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        c.flush()?;
        Ok((c, w))
    }

    pub fn publish(sources: Vec<(String, Vec<u8>)>, generation: u64) -> anyhow::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("rf-clipboard".into()).spawn(move || {
            let run = || -> anyhow::Result<(RustConnection, Window, Atom, Atom, Vec<(Atom, Vec<u8>)>)> {
                let (c, w) = setup()?;
                let clipboard = atom(&c, "CLIPBOARD")?;
                let targets = atom(&c, "TARGETS")?;
                let data = sources.into_iter().map(|(m, b)| Ok((atom(&c, &m)?, b))).collect::<anyhow::Result<Vec<_>>>()?;
                c.set_selection_owner(w, clipboard, CURRENT_TIME)?;
                anyhow::ensure!(c.get_selection_owner(clipboard)?.reply()?.owner == w, "couldn't take the X11 clipboard");
                Ok((c, w, clipboard, targets, data))
            };
            let (c, w, clipboard, targets, data) = match run() {
                Ok(v) => {
                    let _ = tx.send(Ok(()));
                    v
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            while let Ok(ev) = c.wait_for_event() {
                match ev {
                    Event::SelectionRequest(e) if e.selection == clipboard => {
                        let prop = if e.property == NONE { e.target } else { e.property };
                        let answered = if e.target == targets {
                            let mut list: Vec<Atom> = data.iter().map(|(a, _)| *a).collect();
                            list.push(targets);
                            c.change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &list).is_ok()
                        } else if let Some((_, bytes)) = data.iter().find(|(a, _)| *a == e.target) {
                            c.change_property8(PropMode::REPLACE, e.requestor, prop, e.target, bytes).is_ok()
                        } else {
                            false
                        };
                        let notify = SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: e.time,
                            requestor: e.requestor,
                            selection: e.selection,
                            target: e.target,
                            property: if answered { prop } else { NONE },
                        };
                        let _ = c.send_event(false, e.requestor, EventMask::NO_EVENT, notify);
                        let _ = c.flush();
                    }
                    Event::SelectionClear(e) if e.selection == clipboard => {
                        lost(generation);
                        break;
                    }
                    _ => {}
                }
            }
            let _ = c.destroy_window(w);
        })?;
        rx.recv_timeout(READ_TIMEOUT).map_err(|_| anyhow::anyhow!("x11 clipboard: no answer"))?
    }

    /// Asks the owner to convert the clipboard to `target` and returns the bytes it put in our
    /// property (`None`: it can't). Waits at most `READ_TIMEOUT`.
    fn convert(target: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let (c, w) = setup()?;
        let clipboard = atom(&c, "CLIPBOARD")?;
        if c.get_selection_owner(clipboard)?.reply()?.owner == NONE {
            return Ok(None);
        }
        let t = atom(&c, target)?;
        let prop = atom(&c, "RF_CLIPBOARD")?;
        c.convert_selection(w, clipboard, t, prop, CURRENT_TIME)?;
        c.flush()?;
        let deadline = std::time::Instant::now() + READ_TIMEOUT;
        loop {
            match c.poll_for_event()? {
                Some(Event::SelectionNotify(e)) if e.requestor == w => {
                    if e.property == NONE {
                        return Ok(None);
                    }
                    let r = c.get_property(true, w, prop, AtomEnum::ANY, 0, (16 << 20) / 4)?.reply()?;
                    anyhow::ensure!(r.type_ != atom(&c, "INCR")?, "clipboard data too large");
                    return Ok(Some(r.value));
                }
                Some(_) => {}
                None if std::time::Instant::now() > deadline => anyhow::bail!("the clipboard's owner didn't answer"),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    pub fn types() -> anyhow::Result<Vec<String>> {
        let Some(raw) = convert("TARGETS")? else { return Ok(Vec::new()) };
        let (c, _) = setup()?;
        let atoms = raw.chunks_exact(4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
        Ok(atoms.filter_map(|a| c.get_atom_name(a).ok()?.reply().ok()).map(|r| String::from_utf8_lossy(&r.name).into_owned()).collect())
    }

    pub fn read(mime: &str) -> anyhow::Result<Vec<u8>> {
        convert(mime)?.ok_or_else(|| anyhow::anyhow!("the clipboard has no {mime}"))
    }
}

// ---------------------------------------------------------------------------------------------
// The rf-win API
// ---------------------------------------------------------------------------------------------

fn publish(sources: Vec<(String, Vec<u8>)>) -> u64 {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let r = match backend() {
        Backend::Wayland => wayland::publish(sources, generation),
        Backend::X11 => x11::publish(sources, generation),
        Backend::None => Ok(()),
    };
    if let Err(e) = r {
        // GNOME has no data-control: copy / paste then stay inside rfiles
        log::info!("clipboard: {e:#}; files stay on rfiles' own clipboard");
    }
    generation
}

fn types() -> anyhow::Result<Vec<String>> {
    match backend() {
        Backend::Wayland => wayland::types(),
        Backend::X11 => x11::types(),
        Backend::None => anyhow::bail!("no display"),
    }
}

fn read(mime: &str) -> anyhow::Result<Vec<u8>> {
    match backend() {
        Backend::Wayland => wayland::read(mime),
        Backend::X11 => x11::read(mime),
        Backend::None => anyhow::bail!("no display"),
    }
}

/// Empties the clipboard (so a paste can't pick up what was there before).
pub fn clear(owner: crate::Opaque) -> anyhow::Result<()> {
    let _ = owner;
    *FILES.lock().unwrap() = None;
    SEQUENCE.fetch_add(1, Ordering::SeqCst);
    publish(vec![(EMPTY.to_string(), Vec::new())]);
    Ok(())
}

/// Changes every time the clipboard's content changes (ours, or another app's copy).
pub fn sequence() -> u32 {
    SEQUENCE.load(Ordering::SeqCst)
}

/// Puts files on the clipboard for copy (`Copy`) or cut (`Move`).
pub fn set_files(owner: crate::Opaque, paths: &[PathBuf], kind: Transfer) -> anyhow::Result<()> {
    if paths.is_empty() {
        return clear(owner);
    }
    *FILES.lock().unwrap() = Some((paths.to_vec(), kind));
    SEQUENCE.fetch_add(1, Ordering::SeqCst);
    publish(payloads(paths, kind));
    Ok(())
}

/// Whether there are files to paste. Only looks at the offered types (never waits for the
/// owner's data on Wayland): cheap enough for building a menu.
pub fn has_files() -> bool {
    match types() {
        Ok(t) => t.iter().any(|t| t == GNOME || t == URI_LIST),
        Err(_) => FILES.lock().unwrap().is_some(),
    }
}

/// Files + intended operation (cut → Move).
pub fn get_files(owner: crate::Opaque) -> anyhow::Result<Option<(Vec<PathBuf>, Transfer)>> {
    let _ = owner;
    let system = types().and_then(|t| files_from(&t, &read));
    match system {
        Ok(r) => Ok(r),
        Err(e) => {
            log::debug!("clipboard: {e:#}; using rfiles' own");
            Ok(FILES.lock().unwrap().clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip() {
        let paths = vec![PathBuf::from("/home/me/a b.txt"), PathBuf::from("/tmp/ü")];
        let p = payloads(&paths, Transfer::Move);
        let get = |m: &str| p.iter().find(|(t, _)| t == m).unwrap().1.clone();
        assert_eq!(String::from_utf8(get(GNOME)).unwrap(), "cut\nfile:///home/me/a%20b.txt\nfile:///tmp/%C3%BC");
        assert_eq!(parse_gnome(&get(GNOME)), Some((paths.clone(), Transfer::Move)));
        assert_eq!(parse_uri_list(&get(URI_LIST)), paths);
        assert_eq!(get(KDE_CUT), b"1");
        assert_eq!(String::from_utf8(get("text/plain")).unwrap(), "/home/me/a b.txt\n/tmp/ü");
        // what other apps write
        assert_eq!(parse_uri_list(b"# comment\r\nfile://localhost/x\r\nhttps://example.com/\r\nfile:///y\r\n"), vec![PathBuf::from("/x"), PathBuf::from("/y")]);
        assert_eq!(parse_gnome(b"copy\nfile:///z"), Some((vec![PathBuf::from("/z")], Transfer::Copy)));
        assert_eq!(parse_gnome(b"hello"), None);
    }

    #[test]
    fn reading_picks_the_richest_format() {
        let types: Vec<String> = [URI_LIST, KDE_CUT].map(String::from).to_vec();
        let read = |m: &str| -> anyhow::Result<Vec<u8>> {
            Ok(match m {
                URI_LIST => b"file:///a\r\n".to_vec(),
                KDE_CUT => b"1".to_vec(),
                _ => anyhow::bail!("no"),
            })
        };
        assert_eq!(files_from(&types, &read).unwrap(), Some((vec![PathBuf::from("/a")], Transfer::Move)));
        // a browser's link is no file
        let url = |_: &str| -> anyhow::Result<Vec<u8>> { Ok(b"https://example.com/\r\n".to_vec()) };
        assert_eq!(files_from(&[URI_LIST.to_string()], &url).unwrap(), None);
    }
}
