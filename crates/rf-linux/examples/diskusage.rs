//! Walks a folder with the disk usage engine (the "Show disk usage" view's) and prints the time,
//! the totals and the biggest entries: `cargo run --release -p rf-linux --example diskusage -- ~`
fn main() {
    use rf_linux::diskusage::{DiskTree, Metric, Progress};
    use std::sync::atomic::AtomicBool;

    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let t0 = std::time::Instant::now();
    let t = DiskTree::build(&root, &AtomicBool::new(false), &Progress::default());
    let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
    println!(
        "{}: {} entries, {} files, {:.2} GB ({:.2} GB on disk), {} unreadable folders, {:.2} s, {} MB in memory",
        root.display(),
        t.len(),
        t.files(0),
        gb(t.size(0)),
        gb(t.allocated(0)),
        t.skipped(),
        t0.elapsed().as_secs_f32(),
        t.memory() >> 20
    );
    for c in t.sorted_children(0, Metric::Logical).into_iter().take(10) {
        println!("  {:>9.2} GB  {}{}", gb(t.size(c)), t.name(c), if t.is_dir(c) { "/" } else { "" });
    }
}

