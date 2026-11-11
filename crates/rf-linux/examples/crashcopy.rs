//! Starts a copy with the journal on and kills the process part-way, like a crash or a power
//! cut would: `crashcopy <source> <dest folder> <journal folder> <ms before the crash> [MB/s]`.
//! The app's next start should offer to resume it (rf-win's example, on Linux). The optional
//! speed limit keeps a big file copying long enough to be cut off mid-file.
//! `crashcopy --resume <journal folder>` lists the cut-off jobs and resumes them, as the app's
//! Resume button does.
fn main() {
    use rf_linux::fileops::{Tracker, Transfer};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() == 2 && a[0] == "--resume" {
        rf_linux::fastcopy::set_journal_dir(PathBuf::from(&a[1]));
        for job in rf_linux::fastcopy::interrupted() {
            eprintln!("{job:?}");
            let t = Arc::new(Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false))));
            let started = std::time::Instant::now();
            let r = rf_linux::fastcopy::resume(&job.journal, t);
            eprintln!("resumed in {:?}: {r:?}", started.elapsed());
        }
        return;
    }
    if !(4..=5).contains(&a.len()) {
        eprintln!("usage: crashcopy <source> <dest folder> <journal folder> <ms before the crash> [MB/s]");
        std::process::exit(2);
    }
    let (src, dst, jdir, ms) = (PathBuf::from(&a[0]), PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].parse::<u64>().unwrap());
    rf_linux::fastcopy::set_journal_dir(jdir);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        eprintln!("crash after {ms} ms");
        // SAFETY: ends the process at once, no cleanup (what a crash leaves is the point)
        unsafe { libc::_exit(9) };
    });
    let t = Arc::new(Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false))));
    if let Some(mbs) = a.get(4) {
        t.limit.store(mbs.parse::<u64>().unwrap() << 20, std::sync::atomic::Ordering::Relaxed);
    }
    let r = rf_linux::fastcopy::transfer(Transfer::Copy, &[src], &dst, t);
    eprintln!("finished before the crash: {r:?}");
}

