use std::path::PathBuf;

use rf_player::open;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

#[test]
fn opens_h264_mp4() {
    let m = open(&data("h264.mp4")).unwrap();
    let v = m.info().video.clone().unwrap();
    assert_eq!((v.width, v.height), (160, 90));
    assert!(m.info().audio_streams.is_empty());
}

#[test]
fn decodes_frames_of_each_codec() {
    for f in ["h264.mp4", "hevc.mkv", "prores.mov"] {
        let m = open(&data(f)).unwrap();
        let fr = m.frame_at(0.5).unwrap();
        assert_eq!((fr.width, fr.height), (160, 90), "{f}");
    }
}

#[test]
fn audio_only_file() {
    let m = open(&data("silent.mp3")).unwrap();
    assert!(m.info().video.is_none());
    let a = m.audio(0, 4800, 48000).unwrap();
    assert!(!a.is_empty());
    assert!(a.iter().all(|s| s.abs() < 1e-3));
}

#[test]
fn can_open_by_extension() {
    assert!(rf_player::can_open(&data("h264.mp4")));
    assert!(rf_player::can_open(&data("silent.mp3")));
    assert!(!rf_player::can_open(std::path::Path::new("a.txt")));
}

/// Karenin zamanı, istenen anı kapsayan karenin başlangıcıdır (25 kare/s: 0,5 s → 0,48 s).
#[test]
fn frame_pts_is_frame_start() {
    let m = open(&data("h264.mp4")).unwrap();
    assert!((m.frame_at(0.5).unwrap().pts - 0.48).abs() < 1e-6);
    assert!((m.frame_at(0.0).unwrap().pts).abs() < 1e-9);
    assert!((m.frame_at(0.9).unwrap().pts - 0.88).abs() < 1e-6);
}
