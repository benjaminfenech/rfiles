//! Linux: FFmpeg plays what FilmCraft can't (DivX/Xvid, WMV/WMA, FLV, Theora…), as Media
//! Foundation does on Windows. `ffprobe` describes the file; one `ffmpeg` streams raw frames
//! (8-bit 4:2:0 at a constant frame rate) and another streams audio (f32 at the player's rate).
//! Both read forward through small jumps and start again at the new time (`-ss`) for seeks.
//!
//! [`with_fallback`] wraps a FilmCraft source that opens a file but has no decoder for it: the
//! first "unsupported" error switches it to FFmpeg for good.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, anyhow};
use filmcraft_color::{ColorInfo, Matrix, Primaries, Range, Transfer};
use filmcraft_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use filmcraft_media::{
    AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result,
    SharedSource, VideoStreamInfo,
};
use filmcraft_time::{FrameRate, Tick};
use serde::Deserialize;

/// Extensions tried only through FFmpeg (FilmCraft's own lists cover the rest).
pub const EXTRA_EXTENSIONS: &[&str] = &[
    "wmv", "asf", "wma", "flv", "f4v", "ogv", "ogm", "ac3", "dts", "rm", "rmvb", "vob", "divx",
    "xvid", "3g2", "amr", "ape", "wv", "mka", "dv",
];

/// A request at most this far past the stream's position is read through, not seeked to.
const READ_THROUGH_SECS: f64 = 2.0;
/// Most audio channels passed on (the player mixes down to the device's).
const MAX_CHANNELS: u32 = 8;

/// A running `ffmpeg`, killed when dropped.
struct Proc {
    child: Child,
    out: ChildStdout,
}

impl Proc {
    fn spawn(args: &[String], path: &Path, seek: f64) -> std::io::Result<Proc> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
        if seek > 0.0 {
            // before -i: a fast seek to the keyframe before, then decoded up to `seek` exactly
            cmd.arg("-ss").arg(format!("{seek:.6}"));
        }
        cmd.arg("-i").arg(path_arg(path)).args(args).arg("-");
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let out = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout"))?;
        Ok(Proc { child, out })
    }

    /// Fills `buf`; returns how much was read (less only at the end of the stream).
    fn read_full(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            match self.out.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        n
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A path FFmpeg won't mistake for an option or a protocol (`-x.avi`, `http:…`).
fn path_arg(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(".").join(path)
    }
}

#[derive(Deserialize, Default)]
struct Probe {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Deserialize, Default)]
struct ProbeFormat {
    duration: Option<String>,
    format_name: Option<String>,
    size: Option<String>,
}

#[derive(Deserialize, Default)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    sample_aspect_ratio: Option<String>,
    pix_fmt: Option<String>,
    color_space: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    duration: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    disposition: ProbeDisposition,
}

#[derive(Deserialize, Default)]
struct ProbeDisposition {
    #[serde(default)]
    attached_pic: u8,
}

/// "30000/1001" → (30000, 1001); zero or unreadable → None.
fn ratio(s: Option<&str>, sep: char) -> Option<(u32, u32)> {
    let (a, b) = s?.split_once(sep)?;
    let (a, b) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0 && b > 0).then_some((a, b))
}

/// The colors FFmpeg's yuv420p output has: BT.709 for HD, BT.601 for SD unless the file says.
fn color_of(space: Option<&str>, height: u32) -> ColorInfo {
    let sd = match space {
        Some("bt709") => false,
        Some("bt470bg" | "smpte170m" | "bt601") => true,
        _ => height < 720,
    };
    if !sd {
        return ColorInfo::REC709;
    }
    let primaries = if space == Some("smpte170m") || height == 480 {
        Primaries::Bt601_525
    } else {
        Primaries::Bt601_625
    };
    ColorInfo {
        matrix: Matrix::Bt601,
        transfer: Transfer::Bt709,
        primaries,
        range: Range::Limited,
    }
}

/// What FFmpeg reports about a file, as FilmCraft describes media.
fn media_info(path: &Path, p: &Probe) -> anyhow::Result<MediaInfo> {
    let video = p
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0);
    let audio = p
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));
    if video.is_none() && audio.is_none() {
        anyhow::bail!("no video or audio stream");
    }
    let seconds = |s: Option<&String>| {
        s.and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0)
    };
    let duration = seconds(p.format.duration.as_ref())
        .or_else(|| video.and_then(|v| seconds(v.duration.as_ref())))
        .or_else(|| audio.and_then(|a| seconds(a.duration.as_ref())))
        .context("unknown duration")?;
    let video = video.and_then(|v| {
        let (width, height) = (v.width?, v.height?);
        let (num, den) = ratio(v.avg_frame_rate.as_deref(), '/')
            .or_else(|| ratio(v.r_frame_rate.as_deref(), '/'))
            .unwrap_or((25, 1));
        // odd rates (e.g. 1000/1 from a broken header) are capped
        let (num, den) = if num as f64 / den as f64 > 120.0 {
            (30, 1)
        } else {
            (num, den)
        };
        Some(VideoStreamInfo {
            width,
            height,
            frame_rate: FrameRate::new(num as i64, den as i64),
            par: ratio(v.sample_aspect_ratio.as_deref(), ':').unwrap_or((1, 1)),
            codec: v.codec_name.clone().unwrap_or_default(),
            pixel_format: v.pix_fmt.clone().unwrap_or_default(),
            color: color_of(v.color_space.as_deref(), height),
            has_alpha: false,
            bitrate: v.bit_rate.as_ref().and_then(|b| b.parse().ok()),
            hdr: None,
        })
    });
    let audio_streams = audio
        .and_then(|a| {
            Some(AudioStreamInfo {
                sample_rate: a.sample_rate.as_ref()?.parse().ok()?,
                channels: a.channels?.clamp(1, MAX_CHANNELS),
                codec: a.codec_name.clone().unwrap_or_default(),
                bits_per_sample: None,
            })
        })
        .into_iter()
        .collect::<Vec<_>>();
    Ok(MediaInfo {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        kind: if video.is_some() {
            MediaKind::Movie
        } else {
            MediaKind::AudioOnly
        },
        duration: Tick::from_seconds_f64(duration),
        video,
        audio_streams,
        container: p.format.format_name.clone().unwrap_or_default(),
        start_timecode: None,
        file_size: p.format.size.as_ref().and_then(|s| s.parse().ok()),
    })
}

struct VideoState {
    proc: Option<Proc>,
    /// Frame index (at the output rate) the running process started at.
    start: u64,
    /// Frames read since `start`.
    read: u64,
    last: Option<(u64, Arc<VideoFrame>)>,
}

struct AudioState {
    proc: Option<Proc>,
    rate: u32,
    /// Sample index of the next sample the process gives.
    next: i64,
}

/// A file played through FFmpeg.
pub struct FfmpegSource {
    path: PathBuf,
    info: MediaInfo,
    fps: f64,
    video: Mutex<VideoState>,
    audio: Mutex<AudioState>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Opens `path` through FFmpeg (only `ffprobe` runs here; decoding starts on the first request).
pub fn open(path: &Path) -> anyhow::Result<SharedSource> {
    Ok(Arc::new(FfmpegSource::open(path)?))
}

impl FfmpegSource {
    pub fn open(path: &Path) -> anyhow::Result<FfmpegSource> {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path_arg(path))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    anyhow!("To play this file, install FFmpeg (it provides ffmpeg and ffprobe).")
                }
                _ => anyhow!("ffprobe: {e}"),
            })?;
        anyhow::ensure!(out.status.success(), "FFmpeg can't read this file either");
        let probe: Probe = serde_json::from_slice(&out.stdout).context("ffprobe output")?;
        let info = media_info(path, &probe)?;
        let fps = info.video.as_ref().map_or(25.0, |v| v.frame_rate.as_f64());
        Ok(FfmpegSource {
            path: path.to_path_buf(),
            info,
            fps,
            video: Mutex::new(VideoState {
                proc: None,
                start: 0,
                read: 0,
                last: None,
            }),
            audio: Mutex::new(AudioState {
                proc: None,
                rate: 0,
                next: 0,
            }),
        })
    }

    /// Starts decoding again at frame `at`.
    fn restart(&self, st: &mut VideoState, at: u64) -> Result<()> {
        let mut args: Vec<String> = ["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.push(format!("fps={}", self.fps));
        args.extend(
            ["-pix_fmt", "yuv420p", "-f", "rawvideo"]
                .iter()
                .map(|s| s.to_string()),
        );
        st.proc = Some(
            Proc::spawn(&args, &self.path, at as f64 / self.fps)
                .map_err(|e| MediaError::Io(format!("ffmpeg: {e}")))?,
        );
        st.start = at;
        st.read = 0;
        Ok(())
    }

    fn read_frame(&self, st: &mut VideoState, index: u64) -> Option<Arc<VideoFrame>> {
        let v = self.info.video.as_ref()?;
        let (w, h) = (v.width as usize, v.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut buf = vec![0u8; w * h + 2 * cw * ch];
        if st.proc.as_mut()?.read_full(&mut buf) < buf.len() {
            st.proc = None;
            return None;
        }
        let v_plane = buf.split_off(w * h + cw * ch);
        let u_plane = buf.split_off(w * h);
        Some(Arc::new(VideoFrame {
            width: v.width,
            height: v.height,
            data: PixelData::Yuv8 {
                planes: [Arc::new(buf), Arc::new(u_plane), Arc::new(v_plane)],
                chroma: Chroma::C420,
                alpha: None,
            },
            color: v.color,
            par: v.par,
            pts: Tick::from_seconds_f64(index as f64 / self.fps),
        }))
    }
}

impl MediaSource for FfmpegSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        if self.info.video.is_none() {
            return Err(MediaError::NoStream("video"));
        }
        // past the end, the last frame stays on screen
        let last_index =
            ((self.info.duration.seconds() * self.fps).ceil() as u64).saturating_sub(1);
        let want = ((req.time.seconds().max(0.0) * self.fps + 1e-6).floor() as u64).min(last_index);
        let mut st = lock(&self.video);
        if let Some((i, f)) = &st.last
            && *i == want
        {
            return Ok(f.clone());
        }
        let next = st.start + st.read;
        let near = st.proc.is_some()
            && want >= next
            && ((want - next) as f64) < READ_THROUGH_SECS * self.fps;
        let mut from = if near { None } else { Some(want) };
        loop {
            if let Some(at) = from {
                self.restart(&mut st, at)?;
            }
            let mut got = None;
            while st.start + st.read <= want {
                let index = st.start + st.read;
                match self.read_frame(&mut st, index) {
                    Some(f) => {
                        st.read += 1;
                        got = Some(f.clone());
                        st.last = Some((index, f));
                    }
                    None => break,
                }
            }
            if let Some(f) = got.or_else(|| {
                near.then(|| st.last.as_ref().map(|(_, f)| f.clone()))
                    .flatten()
            }) {
                return Ok(f);
            }
            // the stream ended before `want` (it is a little shorter than its header says): read
            // the last second again and keep its final frame
            match from {
                Some(at) if at > 0 => from = Some(at.saturating_sub(self.fps.ceil() as u64)),
                _ => return Err(MediaError::Decode("ffmpeg gave no frame".into())),
            }
        }
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        let Some(a) = self.info.audio() else {
            return Err(MediaError::NoStream("audio"));
        };
        let ch = a.channels.clamp(1, MAX_CHANNELS) as usize;
        let rate = sample_rate.max(1);
        let mut out = AudioBuffer::silence(rate, ch, frames);
        let start = start.max(0);
        let mut st = lock(&self.audio);
        let near = st.proc.is_some()
            && st.rate == rate
            && start >= st.next
            && ((start - st.next) as f64) < READ_THROUGH_SECS * rate as f64;
        if !near {
            let args: Vec<String> = [
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-ac",
                &ch.to_string(),
                "-ar",
                &rate.to_string(),
                "-f",
                "f32le",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            st.proc = Some(
                Proc::spawn(&args, &self.path, start as f64 / rate as f64)
                    .map_err(|e| MediaError::Io(format!("ffmpeg: {e}")))?,
            );
            st.rate = rate;
            st.next = start;
        }
        let frame_bytes = ch * 4;
        // skip up to `start`
        let mut skip = ((start - st.next) as usize) * frame_bytes;
        let mut scratch = vec![0u8; 64 * 1024];
        while skip > 0 {
            let n = skip.min(scratch.len());
            let Some(p) = st.proc.as_mut() else { break };
            let got = p.read_full(&mut scratch[..n]);
            skip -= got;
            if got < n {
                st.proc = None;
            }
        }
        st.next = start;
        let mut buf = vec![0u8; frames * frame_bytes];
        let got = match st.proc.as_mut() {
            Some(p) => p.read_full(&mut buf),
            None => 0,
        };
        // past the end: silence (the rest of `out`)
        let whole = got / frame_bytes;
        for (i, frame) in buf[..whole * frame_bytes]
            .chunks_exact(frame_bytes)
            .enumerate()
        {
            for (c, s) in frame.as_chunks::<4>().0.iter().enumerate() {
                out.channels[c][i] = f32::from_le_bytes(*s);
            }
        }
        st.next += frames as i64;
        if whole < frames {
            st.proc = None;
        }
        Ok(out)
    }
}

/// A FilmCraft source that switches to FFmpeg when it turns out to have no decoder for the
/// file (it opened the container, e.g. an AVI, but can't decode DivX inside).
struct WithFallback {
    primary: SharedSource,
    path: PathBuf,
    backup: OnceLock<Option<SharedSource>>,
    video_switched: AtomicBool,
    audio_switched: AtomicBool,
}

/// Wraps `primary` (opened from `path`) so "unsupported" errors fall back to FFmpeg.
pub fn with_fallback(primary: SharedSource, path: &Path) -> SharedSource {
    Arc::new(WithFallback {
        primary,
        path: path.to_path_buf(),
        backup: OnceLock::new(),
        video_switched: AtomicBool::new(false),
        audio_switched: AtomicBool::new(false),
    })
}

impl WithFallback {
    fn backup(&self) -> Option<&SharedSource> {
        self.backup
            .get_or_init(|| match open(&self.path) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!(
                        "rf-player: FFmpeg fallback for {}: {e:#}",
                        self.path.display()
                    );
                    None
                }
            })
            .as_ref()
    }

    /// Runs `f` on the primary source until it says "unsupported", then on FFmpeg.
    fn run<T>(&self, switched: &AtomicBool, f: impl Fn(&SharedSource) -> Result<T>) -> Result<T> {
        if switched.load(Ordering::Relaxed)
            && let Some(b) = self.backup()
        {
            return f(b);
        }
        match f(&self.primary) {
            Err(MediaError::Unsupported(e)) => match self.backup() {
                Some(b) => {
                    switched.store(true, Ordering::Relaxed);
                    f(b)
                }
                None => Err(MediaError::Unsupported(e)),
            },
            r => r,
        }
    }
}

impl MediaSource for WithFallback {
    fn info(&self) -> &MediaInfo {
        self.primary.info()
    }
    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        self.run(&self.video_switched, |s| s.video_frame(req))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        self.run(&self.audio_switched, |s| {
            s.audio(start, frames, sample_rate)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ffprobe_output() {
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","width":300,"height":300,"disposition":{"attached_pic":1}},
            {"codec_type":"video","codec_name":"mpeg4","width":640,"height":480,"avg_frame_rate":"30000/1001",
             "sample_aspect_ratio":"4:3","pix_fmt":"yuv420p","disposition":{"attached_pic":0}},
            {"codec_type":"audio","codec_name":"mp3","sample_rate":"44100","channels":2}],
            "format":{"duration":"12.5","format_name":"avi","size":"1000"}}"#;
        let p: Probe = serde_json::from_str(json).unwrap();
        let info = media_info(Path::new("/x/a.avi"), &p).unwrap();
        let v = info.video.as_ref().unwrap();
        assert_eq!(
            (v.width, v.height, v.par, v.codec.as_str()),
            (640, 480, (4, 3), "mpeg4"),
            "cover art skipped"
        );
        assert!((v.frame_rate.as_f64() - 29.97).abs() < 0.01);
        assert_eq!(v.color.matrix, Matrix::Bt601);
        assert_eq!(v.color.primaries, Primaries::Bt601_525);
        assert_eq!(
            info.audio().map(|a| (a.sample_rate, a.channels)),
            Some((44100, 2))
        );
        assert!((info.duration.seconds() - 12.5).abs() < 1e-6);
        assert_eq!(info.kind, MediaKind::Movie);
        assert!(media_info(Path::new("a"), &Probe::default()).is_err());
        assert_eq!(color_of(None, 1080), ColorInfo::REC709);
    }
}
