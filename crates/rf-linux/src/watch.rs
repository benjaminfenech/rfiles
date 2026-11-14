//! Directory change notifications (inotify via `notify`), debounced.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps the watch alive; drop to stop.
pub struct DirWatch {
    _w: RecommendedWatcher,
}

/// What changed in a watched folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Names (directly in the folder) that were added, removed, renamed from or to, or changed.
    pub names: Vec<String>,
    /// The names can't be told (the system dropped events, too many changed, the folder itself
    /// changed): list the folder again.
    pub rescan: bool,
}

/// More changed names than this in one burst: listing the folder again is cheaper.
const MAX_NAMES: usize = 256;

/// Calls `on_change` (from a background thread) once changes in `dir` pause for `debounce`, and
/// during a burst that never pauses (files renamed in the background without end) at least every
/// `debounce` × 5, so the view never falls behind for long.
pub fn watch_dir(dir: &Path, debounce: Duration, on_change: impl Fn() + Send + Sync + 'static) -> anyhow::Result<DirWatch> {
    watch_dir_changes(dir, debounce, move |_| on_change())
}

/// One inotify event, as what it means for the folder's listing.
enum Seen {
    Name(String),
    Rescan,
}

/// [`watch_dir`] telling what changed: the names, or that the folder must be listed again.
pub fn watch_dir_changes(dir: &Path, debounce: Duration, on_change: impl Fn(Changes) + Send + Sync + 'static) -> anyhow::Result<DirWatch> {
    let (tx, rx) = crossbeam_channel::unbounded::<Seen>();
    let base = dir.to_path_buf();
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let ev = match res {
            Ok(ev) => ev,
            Err(_) => {
                let _ = tx.send(Seen::Rescan);
                return;
            }
        };
        if matches!(ev.kind, notify::EventKind::Access(_)) {
            return;
        }
        // the kernel's queue overflowed: events were lost
        if ev.need_rescan() || ev.paths.is_empty() {
            let _ = tx.send(Seen::Rescan);
            return;
        }
        for p in &ev.paths {
            // the folder itself (deleted, moved, its own attributes): the whole listing
            let seen = match (p.parent(), p.file_name()) {
                (Some(parent), Some(name)) if parent == base => Seen::Name(name.to_string_lossy().into_owned()),
                _ => Seen::Rescan,
            };
            let _ = tx.send(seen);
        }
    })?;
    w.watch(dir, RecursiveMode::NonRecursive)?;
    let on_change = Arc::new(on_change);
    let max_wait = debounce * 5;
    std::thread::Builder::new().name("rf-watch".into()).spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut ch = Changes::default();
            let add = |s: Seen, ch: &mut Changes| match s {
                Seen::Rescan => ch.rescan = true,
                Seen::Name(n) => {
                    if !ch.rescan && !ch.names.contains(&n) {
                        ch.names.push(n);
                        if ch.names.len() > MAX_NAMES {
                            ch.rescan = true;
                        }
                    }
                }
            };
            add(first, &mut ch);
            // swallow the burst, but not for longer than max_wait
            let burst = std::time::Instant::now();
            loop {
                let left = max_wait.saturating_sub(burst.elapsed());
                if left.is_zero() {
                    break;
                }
                match rx.recv_timeout(debounce.min(left)) {
                    Ok(s) => add(s, &mut ch),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            }
            if ch.rescan {
                ch.names.clear();
            }
            on_change(ch);
        }
    })?;
    Ok(DirWatch { _w: w })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn a_burst_without_pause_still_reports() {
        let dir = std::env::temp_dir().join(format!("rf-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        let _w = watch_dir(&dir, Duration::from_millis(150), move || {
            c2.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        // a change every 30 ms for 2 s: never 150 ms of quiet
        let start = Instant::now();
        let mut i = 0;
        while start.elapsed() < Duration::from_secs(2) {
            std::fs::write(dir.join(format!("f{}.txt", i % 50)), format!("{i}")).unwrap();
            i += 1;
            std::thread::sleep(Duration::from_millis(30));
        }
        let during = calls.load(Ordering::Relaxed);
        assert!(during >= 2, "reported {during} times during a 2 s burst");
        // and once more after it settles: the last report can come up to max_wait (750 ms)
        // after the burst, more on a busy machine, so wait for it rather than a fixed time
        let settle = Instant::now();
        while calls.load(Ordering::Relaxed) <= during && settle.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(calls.load(Ordering::Relaxed) > during, "no report after the burst settled");
        drop(_w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Added, renamed and removed files come back by name; the folder itself going away asks for
    /// a new listing.
    #[test]
    fn changes_name_what_changed() {
        let dir = std::env::temp_dir().join(format!("rf-watch-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::write(dir.join("in/old.txt"), b"x").unwrap();
        let got = Arc::new(std::sync::Mutex::new(Changes::default()));
        let g2 = got.clone();
        let _w = watch_dir_changes(&dir.join("in"), Duration::from_millis(100), move |c| {
            let mut g = g2.lock().unwrap();
            g.rescan |= c.rescan;
            for n in c.names {
                if !g.names.contains(&n) {
                    g.names.push(n);
                }
            }
        })
        .unwrap();
        std::fs::write(dir.join("in/new.txt"), b"x").unwrap();
        std::fs::rename(dir.join("in/old.txt"), dir.join("in/renamed.txt")).unwrap();
        let t = Instant::now();
        loop {
            {
                let g = got.lock().unwrap();
                if ["new.txt", "old.txt", "renamed.txt"].iter().all(|n| g.names.iter().any(|x| x == n)) {
                    assert!(!g.rescan);
                    break;
                }
            }
            assert!(t.elapsed() < Duration::from_secs(5), "{:?}", got.lock().unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::remove_dir_all(dir.join("in")).unwrap();
        let t = Instant::now();
        while !got.lock().unwrap().rescan {
            assert!(t.elapsed() < Duration::from_secs(5), "the folder's removal asks for a rescan");
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
