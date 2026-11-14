//! Single-instance plumbing: the first process binds a Unix socket in `$XDG_RUNTIME_DIR`;
//! later ones connect, send their message (one line) and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;

/// Socket path for an app of the bundle (rfiles and DirectLook are separate groups).
fn socket_for(app: &str) -> PathBuf {
    let suffix = match std::env::var_os("RFILES_CONFIG_DIR") {
        Some(dir) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            dir.to_string_lossy().hash(&mut h);
            format!("-{:08x}", h.finish() as u32)
        }
        None => String::new(),
    };
    // without XDG_RUNTIME_DIR, a folder only you can enter (never the shared /tmp, where
    // another user could take the name first and receive the paths sent to rfiles)
    let dir = dirs::runtime_dir()
        .or_else(|| crate::private_tmp::root_dir().ok())
        .unwrap_or_else(crate::private_tmp::root);
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    dir.join(format!("{app}-{user}{suffix}.sock"))
}

pub enum Role {
    /// We are the first instance. Keep the guard alive for the process lifetime.
    Primary(InstanceGuard),
    /// Another instance is running and received our message.
    Forwarded,
}

/// Removes the socket when the primary exits.
pub struct InstanceGuard(Option<PathBuf>);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Listeners bound by `acquire_app`, waiting for `serve_app` to take them.
static BOUND: Mutex<Vec<(String, UnixListener)>> = Mutex::new(Vec::new());

/// Decides the role. `message` is sent to the primary when one exists ("" = just focus a new window/tab).
pub fn acquire(message: &str) -> Role {
    acquire_app("rfiles", message)
}

/// [`acquire`] for another application of the bundle (e.g. "directlook").
pub fn acquire_app(app: &str, message: &str) -> Role {
    let path = socket_for(app);
    if send(&path, message) {
        return Role::Forwarded;
    }
    // nobody answered: a socket file left by a crash is stale
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(l) => {
            BOUND.lock().unwrap().push((app.to_string(), l));
            Role::Primary(InstanceGuard(Some(path)))
        }
        Err(e) => {
            log::warn!("single instance socket {}: {e}", path.display());
            Role::Primary(InstanceGuard(None))
        }
    }
}

/// Sends `message` to the running instance of `app` (false if none answers).
pub fn send_to(app: &str, message: &str) -> bool {
    send(&socket_for(app), message)
}

fn send(path: &std::path::Path, message: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(path) else { return false };
    let _ = s.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    let mut bytes = message.as_bytes().to_vec();
    bytes.push(b'\n');
    s.write_all(&bytes).is_ok()
}

/// Serves the socket on a background thread; each received line is passed to `on_message`.
pub fn serve(on_message: impl Fn(String) + Send + Sync + 'static) {
    serve_app("rfiles", on_message)
}

/// [`serve`] for another application of the bundle.
pub fn serve_app(app: &str, on_message: impl Fn(String) + Send + Sync + 'static) {
    let listener = {
        let mut bound = BOUND.lock().unwrap();
        let i = bound.iter().position(|(a, _)| a == app);
        i.map(|i| bound.remove(i).1)
    };
    let Some(listener) = listener else {
        log::warn!("single instance: serve_app({app}) without a bound socket");
        return;
    };
    let on_message = std::sync::Arc::new(on_message);
    std::thread::Builder::new()
        .name("rf-instance-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let cb = on_message.clone();
                // each client on its own thread so a stalled one can't block the next
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                    for line in BufReader::new(std::io::Read::take(stream, 64 * 1024)).lines().map_while(Result::ok) {
                        cb(line);
                    }
                });
            }
        })
        .ok();
}
