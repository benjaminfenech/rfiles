// Windows implementation of tasklist; the entry point lives in tasklist.rs.

pub fn main() {
    let start = std::time::Instant::now();
    let tasks = rf_win::tasklist::tasks();
    let took = start.elapsed();
    let n = tasks.len();
    for (key, windows) in rf_win::tasklist::grouped(tasks) {
        println!("{key}");
        for w in windows {
            println!("    {}{}", if w.minimized { "(küçük) " } else { "" }, w.title);
        }
    }
    println!("\n{n} pencere, {took:?}");
}
