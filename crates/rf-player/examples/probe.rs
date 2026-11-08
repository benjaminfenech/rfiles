//! Tries files with rf-player (FilmCraft): opens each, decodes a frame at 0.5 s and some audio,
//! and says what worked: `cargo run -p rf-player --example probe -- <files…>`

fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !rf_player::can_open(path) {
            println!("{name}: extension not tried");
            continue;
        }
        let media = match rf_player::open(path) {
            Ok(m) => m,
            Err(e) => {
                println!("{name}: open failed: {e:#}");
                continue;
            }
        };
        let video = match media.info().video.is_some() {
            false => "no video".to_string(),
            true => match media.frame_at(0.5) {
                Ok(f) => format!("video {}x{}", f.width, f.height),
                Err(e) => format!("video FAILED ({e:#})"),
            },
        };
        let audio = match media.info().has_audio() {
            false => "no audio".to_string(),
            true => match media.audio(0, 1024, 48000) {
                Ok(a) => format!("audio ok ({} samples)", a.len()),
                Err(e) => format!("audio FAILED ({e:#})"),
            },
        };
        println!("{name}: {video}, {audio}");
    }
}
