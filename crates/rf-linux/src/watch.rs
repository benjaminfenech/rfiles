//! Directory change notifications (inotify via `notify`), debounced.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps the watch alive; drop to stop.
pub struct DirWatch {
    _w: RecommendedWatcher,
}

/// Calls `on_change` (from a background thread) at most once per `debounce` after changes in `dir`.
pub fn watch_dir(dir: &Path, debounce: Duration, on_change: impl Fn() + Send + Sync + 'static) -> anyhow::Result<DirWatch> {
    let (tx, rx) = crossbeam_channel::unbounded::<()>();
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if !matches!(ev.kind, notify::EventKind::Access(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    w.watch(dir, RecursiveMode::NonRecursive)?;
    let on_change = Arc::new(on_change);
    std::thread::Builder::new().name("rf-watch".into()).spawn(move || {
        while rx.recv().is_ok() {
            // swallow the burst
            while rx.recv_timeout(debounce).is_ok() {}
            on_change();
        }
    })?;
    Ok(DirWatch { _w: w })
}
