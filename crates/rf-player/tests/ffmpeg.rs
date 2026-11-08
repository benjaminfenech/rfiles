//! Linux: files FilmCraft can't decode play through FFmpeg. The test files are made with
//! `ffmpeg` itself; without it the tests are skipped.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

use rf_player::{Planes, open};

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// 2 s at 10 fps, 64 x 48: red for the first second, blue for the second; a 1 kHz tone.
fn make(dir: &Path, name: &str, codec: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=red:s=64x48:r=10:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=blue:s=64x48:r=10:d=1",
        ])
        .args(["-f", "lavfi", "-i", "sine=f=1000:d=2:sample_rate=44100"])
        .args([
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-map",
            "2:a",
        ])
        .args(codec)
        .arg(&out)
        .status()
        .unwrap();
    assert!(ok.success(), "ffmpeg made {name}");
    out
}

/// Mean (Y, U, V) of a frame.
fn mean_yuv(p: &Planes) -> (u32, u32, u32) {
    let Planes::Yuv8 { planes, .. } = p else {
        panic!("expected 8-bit YUV")
    };
    let mean = |v: &[u8]| (v.iter().map(|&x| x as u64).sum::<u64>() / v.len() as u64) as u32;
    (mean(&planes[0]), mean(&planes[1]), mean(&planes[2]))
}

fn is_red(p: &Planes) -> bool {
    let (_, u, v) = mean_yuv(p);
    v > 200 && u < 128
}

fn is_blue(p: &Planes) -> bool {
    let (_, u, v) = mean_yuv(p);
    u > 200 && v < 160
}

fn check(path: &Path) {
    let m = open(path).unwrap();
    let v = m.info().video.clone().expect("video stream");
    assert_eq!((v.width, v.height), (64, 48));
    assert!(
        (m.duration() - 2.0).abs() < 0.15,
        "duration {}",
        m.duration()
    );
    // forward, read through, then back (starts FFmpeg again)
    assert!(
        is_red(&m.frame_at(0.25).unwrap().planes),
        "red at 0.25 s: {:?}",
        mean_yuv(&m.frame_at(0.25).unwrap().planes)
    );
    assert!(is_blue(&m.frame_at(1.55).unwrap().planes), "blue at 1.55 s");
    assert!(
        is_red(&m.frame_at(0.05).unwrap().planes),
        "red again at 0.05 s"
    );
    // past the end: the last frame stays
    assert!(
        is_blue(&m.frame_at(5.0).unwrap().planes),
        "last frame after the end"
    );
    // audio: the tone, then silence past the end
    let a = m.audio(0, 4800, 48000).unwrap();
    let ch = m.info().audio().unwrap().channels as usize;
    assert_eq!(a.len(), 4800 * ch);
    assert!(
        a.iter().any(|s| s.abs() > 0.05),
        "the tone is there (ffmpeg makes it at 1/8 of full scale)"
    );
    let tail = m.audio(48000 * 2 + 4800, 4800, 48000).unwrap();
    assert!(tail.iter().all(|s| *s == 0.0), "silence past the end");
}

#[test]
fn plays_what_filmcraft_cannot() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rf-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // FilmCraft opens the AVI but has no DivX/Xvid decoder: switches to FFmpeg on the first frame
    check(&make(
        &dir,
        "xvid.avi",
        &["-c:v", "mpeg4", "-c:a", "libmp3lame"],
    ));
    // not a FilmCraft format at all: FFmpeg from the start
    check(&make(&dir, "old.wmv", &["-c:v", "wmv2", "-c:a", "wmav2"]));
    let _ = std::fs::remove_dir_all(&dir);
}
