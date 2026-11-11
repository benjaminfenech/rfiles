//! Checks the file clipboard against the system's:
//! `cargo run -p rf-linux --example clipboard -- copy|cut <file>… [seconds]` puts files on it
//! (and keeps serving for `seconds`, default 10); `… -- show` prints what is there.

fn main() -> anyhow::Result<()> {
    use rf_linux::clipboard::*;
    use rf_linux::fileops::Transfer;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some(verb @ ("copy" | "cut")) => {
            let secs: u64 = args.last().and_then(|a| a.parse().ok()).unwrap_or(10);
            let files: Vec<std::path::PathBuf> = args[1..].iter().filter(|a| a.parse::<u64>().is_err()).map(Into::into).collect();
            set_files(0, &files, if verb == "cut" { Transfer::Move } else { Transfer::Copy })?;
            let seq = sequence();
            println!("published {} file(s); serving for {secs} s", files.len());
            for _ in 0..secs * 10 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if sequence() != seq {
                    println!("another app copied something: sequence changed");
                    break;
                }
            }
        }
        _ => {
            println!("has_files: {}", has_files());
            println!("get_files: {:?}", get_files(0)?);
        }
    }
    Ok(())
}

