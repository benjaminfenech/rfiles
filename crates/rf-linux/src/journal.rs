//! A crash-safe, append-only record file: one record per line, `crc32<TAB>fields…`, where the
//! fields are separated by tabs. A line whose checksum doesn't match, and everything after it,
//! is ignored on reading, so a record torn by a crash or a power cut is dropped instead of
//! trusted. Port of rf-win's `journal` (same format); fields must not contain tabs or line
//! breaks, which Linux file names can: callers escape paths (see `fastcopy`).
//!
//! Durability: [`Writer::sync`] flushes to the disk (`fdatasync`, and the first time also the
//! folder, so the new file's name survives a power cut). Callers sync what must not be lost
//! (the plan of a copy, before the first byte is copied) and batch the rest.
//!
//! A writer holds an exclusive `flock` on the file (Windows: no write sharing), so another
//! process can tell a running job (locked) from an interrupted one (the writer died, and the
//! kernel dropped its lock): [`in_use`].

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

pub struct Writer {
    out: BufWriter<File>,
    /// The folder still to be synced once (a new file's directory entry).
    dir: Option<PathBuf>,
}

fn line(fields: &[&str]) -> String {
    debug_assert!(fields.iter().all(|f| !f.contains(['\t', '\n'])), "journal fields can't hold tabs or line breaks");
    let payload = fields.join("\t");
    format!("{:08x}\t{payload}\n", crc32fast::hash(payload.as_bytes()))
}

/// Takes `f`'s lock without waiting: `exclusive` for a writer, shared to test for one.
fn lock(f: &File, exclusive: bool) -> std::io::Result<()> {
    let op = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH };
    // SAFETY: a valid fd; the lock goes with the open file (released when it closes)
    if unsafe { libc::flock(f.as_raw_fd(), op | libc::LOCK_NB) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn locked(f: File, path: &Path) -> std::io::Result<File> {
    lock(&f, true).map_err(|e| {
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            std::io::Error::new(std::io::ErrorKind::ResourceBusy, format!("{} is in use by a running job", path.display()))
        } else {
            e
        }
    })?;
    Ok(f)
}

impl Writer {
    /// A new journal file (fails if it exists).
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let f = OpenOptions::new().write(true).create_new(true).open(path)?;
        let f = locked(f, path)?;
        Ok(Self { out: BufWriter::with_capacity(64 << 10, f), dir: path.parent().map(Path::to_path_buf) })
    }

    /// An existing journal, appended to (resuming a job). Fails with `ResourceBusy` while
    /// another writer has it.
    pub fn append(path: &Path) -> std::io::Result<Self> {
        let f = locked(OpenOptions::new().append(true).open(path)?, path)?;
        Ok(Self { out: BufWriter::with_capacity(64 << 10, f), dir: None })
    }

    pub fn record(&mut self, fields: &[&str]) -> std::io::Result<()> {
        self.out.write_all(line(fields).as_bytes())
    }

    /// Everything written so far is on the disk when this returns.
    pub fn sync(&mut self) -> std::io::Result<()> {
        self.out.flush()?;
        self.out.get_ref().sync_data()?;
        if let Some(d) = self.dir.take() {
            File::open(&d)?.sync_all()?;
        }
        Ok(())
    }
}

/// The intact records of a journal, in order: reading stops at the first line that is torn or
/// fails its checksum.
pub fn read(path: &Path) -> std::io::Result<Vec<Vec<String>>> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let mut out = Vec::new();
    let mut rest = &bytes[..];
    while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
        let (ln, tail) = rest.split_at(nl);
        rest = &tail[1..];
        let Ok(s) = std::str::from_utf8(ln) else { break };
        let Some((crc, payload)) = s.split_once('\t') else { break };
        if u32::from_str_radix(crc, 16).ok() != Some(crc32fast::hash(payload.as_bytes())) {
            break;
        }
        out.push(payload.split('\t').map(str::to_string).collect());
    }
    Ok(out)
}

/// Replaces the journal with exactly `records` (dropping a corrupt tail before more is appended)
/// and returns a writer that appends to it. The new file is synced before it takes the old one's
/// place, so a crash here leaves either the old journal or the new one, never neither. The
/// returned writer is the new file's own (its lock held from the start, so no other process
/// sees the job as interrupted while it is swapped in).
pub fn rewrite(path: &Path, records: &[Vec<String>]) -> std::io::Result<Writer> {
    let tmp = path.with_extension("jrn-new");
    let _ = std::fs::remove_file(&tmp);
    let mut w = Writer::create(&tmp)?;
    for r in records {
        w.record(&r.iter().map(String::as_str).collect::<Vec<_>>())?;
    }
    w.sync()?;
    std::fs::rename(&tmp, path)?;
    if let Some(d) = path.parent() {
        File::open(d)?.sync_all()?;
    }
    Ok(w)
}

/// True while a writer has the journal open (its job is running, maybe in another process).
pub fn in_use(path: &Path) -> bool {
    match File::open(path) {
        Ok(f) => lock(&f, false).is_err(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the lock is free within a second. A child process another test starts at the
    /// same moment holds a copy of the writer's file until it runs its program (the file is
    /// closed then), and an flock lock stays while any copy is open.
    fn freed(p: &std::path::Path) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while in_use(p) {
            if std::time::Instant::now() > until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        true
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rf-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&d);
        d
    }

    #[test]
    fn torn_and_corrupt_tails_are_dropped() {
        let p = tmp("torn");
        {
            let mut w = Writer::create(&p).unwrap();
            w.record(&["B", "c", "/a b/ç.txt"]).unwrap();
            w.record(&["F", "0"]).unwrap();
            w.sync().unwrap();
            assert!(in_use(&p), "an open writer marks the job as running");
            assert_eq!(Writer::append(&p).err().map(|e| e.kind()), Some(std::io::ErrorKind::ResourceBusy), "one writer at a time");
        }
        assert!(freed(&p));
        // a torn record (no newline) and garbage after a crash
        let mut f = OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(line(&["F", "1"]).as_bytes()).unwrap();
        f.write_all(b"deadbeef\tF\t2\n").unwrap();
        f.write_all(line(&["F", "3"]).as_bytes()).unwrap();
        f.write_all(b"0123").unwrap();
        drop(f);
        let r = read(&p).unwrap();
        assert_eq!(r, vec![vec!["B".to_string(), "c".into(), "/a b/ç.txt".into()], vec!["F".into(), "0".into()], vec!["F".into(), "1".into()]]);
        // appending after a reopen keeps the earlier records
        let mut w = Writer::append(&p).unwrap();
        w.record(&["F", "9"]).unwrap();
        w.sync().unwrap();
        drop(w);
        // …but behind a corrupt tail it can't be read: resuming rewrites the journal first
        assert_eq!(read(&p).unwrap().len(), 3);
        let _ = std::fs::remove_file(&p);
    }

    /// The rewritten journal is locked the whole time: no window where it looks interrupted.
    #[test]
    fn rewrite_keeps_the_lock() {
        let p = tmp("rewrite");
        {
            let mut w = Writer::create(&p).unwrap();
            w.record(&["B"]).unwrap();
            w.record(&["F", "0"]).unwrap();
            w.sync().unwrap();
        }
        let mut w = rewrite(&p, &[vec!["B".into()]]).unwrap();
        assert!(in_use(&p));
        w.record(&["F", "1"]).unwrap();
        w.sync().unwrap();
        drop(w);
        assert!(freed(&p));
        assert_eq!(read(&p).unwrap(), vec![vec!["B".to_string()], vec!["F".into(), "1".into()]]);
        assert!(!p.with_extension("jrn-new").exists());
        let _ = std::fs::remove_file(&p);
    }
}
