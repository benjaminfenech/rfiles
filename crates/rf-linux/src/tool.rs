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
    let started = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("{prog} took longer than {timeout:?}");
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    Ok(Ran {
        ok: status.success(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}
