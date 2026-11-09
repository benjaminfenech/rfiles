//! Memory and speed of the name index on a real file (Linux's `index_mem`, after rf-win's):
//! load (converting an older format), search, save in the current format and load that again.
//! `cargo run --release -p rf-linux --example index_mem -- <file.rfidx> <pattern>`
//! The file is only read; the saved copy goes to the temp folder and is deleted.

/// This process's resident memory in MB (`/proc/self/status`).
#[cfg(unix)]
fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines().find_map(|l| l.strip_prefix("VmRSS:")?.trim().strip_suffix("kB")?.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;
    use std::time::Instant;

    use rf_linux::nameindex::Index;

    let mut a = std::env::args().skip(1);
    let file = PathBuf::from(a.next().expect("index file"));
    let pattern = a.next().unwrap_or_else(|| "rapor".into());
    let before = rss_mb();
    let t = Instant::now();
    let ix = Index::load(&file)?;
    println!(
        "loaded {} entries, {} different names in {:.2} s; index says {:.1} MB, process +{:.0} MB",
        ix.live(),
        ix.distinct_names(),
        t.elapsed().as_secs_f32(),
        ix.memory() as f64 / 1048576.0,
        rss_mb() - before
    );
    for _ in 0..3 {
        let t = Instant::now();
        let hits = ix.search(&pattern, None, false, 10_000).unwrap_or_default();
        println!("search {pattern:?}: {} hits in {:.1} ms", hits.len(), t.elapsed().as_secs_f64() * 1000.0);
    }
    let out = std::env::temp_dir().join(format!("rf-index-mem-{}.rfidx", std::process::id()));
    let t = Instant::now();
    ix.save(&out)?;
    let saved = t.elapsed();
    let size = std::fs::metadata(&out)?.len();
    drop(ix);
    let t = Instant::now();
    let back = Index::load(&out)?;
    println!("saved in {:.2} s ({:.1} MB on disk, was {:.1} MB), loaded back in {:.2} s, {} entries", saved.as_secs_f32(), size as f64 / 1048576.0, std::fs::metadata(&file)?.len() as f64 / 1048576.0, t.elapsed().as_secs_f32(), back.live());
    let _ = std::fs::remove_file(&out);
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("this example is for Linux");
}
