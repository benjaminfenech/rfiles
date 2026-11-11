//! Subfolder search, the name index against the disk walk (Linux's `search_bench`, after
//! rf-win's): `cargo run --release -p rf-linux --example search_bench -- <folder> <query> [runs]`.
//! Prints the index's build time and size, its query times, and per walk run the time to the
//! first hit and to the last, and the hit count. `HEAVIEST=1` lists the biggest subtrees.
//! The process's resident memory after the build (and its peak during it) comes from
//! `/proc/self/status`.

/// (VmRSS, VmHWM) of this process in MB.
fn rss_mb() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kb = |key: &str| s.lines().find_map(|l| l.strip_prefix(key)?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()).unwrap_or(0);
    (kb("VmRSS:") >> 10, kb("VmHWM:") >> 10)
}

fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    let mut a = std::env::args().skip(1);
    let root = PathBuf::from(a.next().expect("folder"));
    let query = a.next().expect("query");
    let runs: usize = a.next().map_or(3, |s| s.parse().unwrap());
    println!("{} \"{query}\"", root.display());
    let t = Instant::now();
    let ix = rf_linux::nameindex::Index::build(&root, &std::sync::atomic::AtomicBool::new(false));
    let (rss, peak) = rss_mb();
    println!("  index build: {:.1?}, {} entries, {:.1} MB, {} names; process {rss} MB, peak {peak} MB", t.elapsed(), ix.live(), ix.memory() as f64 / 1048576.0, ix.distinct_names());
    if std::env::var_os("HEAVIEST").is_some() {
        for (p, c) in ix.heaviest(25, 3) {
            println!("    {c:>9}  {}", p.display());
        }
    }
    let mut index_hits = 0;
    for _ in 0..3 {
        let t = Instant::now();
        let hits = ix.search(&query, None, false, 50_000).unwrap_or_default();
        index_hits = hits.len();
        println!("  index query: {:.2?}  {} hits", t.elapsed(), hits.len());
    }
    for run in 0..runs {
        let t = Instant::now();
        let mut first: Option<Duration> = None;
        let mut walk_hits = std::collections::HashSet::new();
        let q = rf_linux::search::SearchQuery { root: root.clone(), pattern: query.clone(), include_hidden: false, max_results: 50_000 };
        let s = rf_linux::search::search(&q, &|| false, &mut |h| {
            if !h.is_empty() && first.is_none() {
                first = Some(t.elapsed());
            }
            walk_hits.extend(h.into_iter().map(|h| h.path));
        })?;
        println!(
            "  run {run} walk  : first {:>8.1?}  all {:>8.1?}  {} hits, {} folders, {} entries",
            first.unwrap_or_default(),
            t.elapsed(),
            s.hits,
            s.dirs_scanned,
            s.entries_scanned
        );
        if s.hits != index_hits {
            // the index leaves caches out (node_modules, .git, CACHEDIR.TAG…): the walk doesn't
            let ixh: std::collections::HashSet<PathBuf> = ix.search(&query, None, false, 50_000).unwrap_or_default().into_iter().collect();
            println!("  walk found {} hits, the index {} ({} only walked, {} only indexed)", s.hits, index_hits, walk_hits.difference(&ixh).count(), ixh.difference(&walk_hits).count());
        }
    }
    Ok(())
}

