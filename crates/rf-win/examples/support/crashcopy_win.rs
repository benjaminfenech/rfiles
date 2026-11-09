// Windows implementation of crashcopy; the entry point lives in crashcopy.rs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rf_win::fileops::{Tracker, Transfer};

pub fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, dst, jdir, ms) = (PathBuf::from(&a[0]), PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].parse::<u64>().unwrap());
    rf_win::fastcopy::set_journal_dir(jdir);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        eprintln!("crash after {ms} ms");
        std::process::exit(9);
    });
    let t = Arc::new(Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false))));
    let r = rf_win::fastcopy::transfer(Transfer::Copy, &[src], &dst, t);
    eprintln!("finished before the crash: {r:?}");
}
