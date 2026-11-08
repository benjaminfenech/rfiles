//! Linux has no apartment threads or message pumps. This keeps rf-win's "shell thread" API:
//! one long-lived worker runs the jobs in order, so a slow job still never blocks the UI.

use std::sync::OnceLock;

use crossbeam_channel::Sender;

type Job = Box<dyn FnOnce(crate::Opaque) + Send>;

static THREAD: OnceLock<Sender<Job>> = OnceLock::new();

/// Runs `f` on the shell worker thread. `f` receives a placeholder owner (0).
pub fn run(f: impl FnOnce(crate::Opaque) + Send + 'static) {
    let tx = THREAD.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        std::thread::Builder::new()
            .name("rf-shell".into())
            .spawn(move || {
                for job in rx {
                    job(0);
                }
            })
            .expect("spawn shell thread");
        tx
    });
    let _ = tx.send(Box::new(f));
}
