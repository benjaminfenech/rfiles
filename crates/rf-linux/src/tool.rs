//! Running command-line tools (poppler, libavif, libheif) as child processes: their output
//! collected, a timeout, and paths they can't mistake for options. Blocks: worker threads only.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::anyhow;

/// The tool isn't installed (not found on `PATH`).
#[derive(Debug, PartialEq, Eq)]
pub struct NotInstalled(pub String);

impl std::fmt::Display for NotInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not installed", self.0)
    }
}

impl std::error::Error for NotInstalled {}

/// A path the tools won't mistake for an option.
pub fn path_arg(path: &Path) -> PathBuf {
    if path.as_os_str().as_encoded_bytes().starts_with(b"-") {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    }
}

pub struct Ran {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `cmd`, collecting its output; kills it after `timeout`. A missing program is a
/// [`NotInstalled`] error.
pub fn run(mut cmd: Command, timeout: Duration) -> anyhow::Result<Ran> {
    let prog = cmd.get_program().to_string_lossy().into_owned();
    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NotInstalled(prog).into()),
        Err(e) => return Err(anyhow!("{prog}: {e}")),
    };
    // read both pipes on their own threads so a full pipe never stalls the child
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = reader(child.stdout.take().map(|s| Box::new(s) as _));
    let err = reader(child.stderr.take().map(|s| Box::new(s) as _));
    let Some(status) = wait_timeout(&mut child, timeout)? else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("{prog} took longer than {timeout:?}");
    };
    Ok(Ran {
        ok: status.success(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// Waits for `child` to exit, up to `timeout`; `None` when it's still running then (the
/// caller decides whether to kill it). Sleeps in the kernel on a pidfd (Linux 5.3+) instead of
/// waking up to poll; falls back to polling where pidfds aren't available.
pub fn wait_timeout(child: &mut std::process::Child, timeout: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    use std::os::fd::{FromRawFd, OwnedFd, AsRawFd};
    let deadline = Instant::now() + timeout;
    // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as libc::pid_t, 0) };
    if fd >= 0 {
        // SAFETY: a fresh fd we own
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        loop {
            if let Some(s) = child.try_wait()? {
                return Ok(Some(s));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let mut p = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = left.as_millis().clamp(1, i32::MAX as u128) as i32;
            // SAFETY: one valid pollfd
            if unsafe { libc::poll(&mut p, 1, ms) } < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
        }
    }
    let mut nap = Duration::from_millis(1);
    loop {
        if let Some(s) = child.try_wait()? {
            return Ok(Some(s));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(nap.min(deadline.saturating_duration_since(Instant::now())));
        nap = (nap * 2).min(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;

    #[test]
    fn exits_and_times_out() {
        let mut c = Command::new("sh").args(["-c", "exit 3"]).spawn().unwrap();
        let t = Instant::now();
        let s = wait_timeout(&mut c, Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(s.code(), Some(3));
        assert!(t.elapsed() < Duration::from_secs(2));

        let mut c = Command::new("sleep").arg("5").spawn().unwrap();
        let t = Instant::now();
        assert!(wait_timeout(&mut c, Duration::from_millis(200)).unwrap().is_none());
        assert!(t.elapsed() >= Duration::from_millis(200) && t.elapsed() < Duration::from_secs(2));
        let _ = c.kill();
        let _ = c.wait();
    }
}
