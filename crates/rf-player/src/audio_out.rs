//! Ses çıkışı: WASAPI (paylaşımlı mod), Linux'ta cpal ve testler için sessiz, gerçek zamanlı ilerleyen `NullSink`.

use std::time::Instant;

/// Oynatıcının sesi yazdığı yer. `write` alabildiği kadarını alır (kare sayısı döner); `played_frames`
/// hoparlörden çıkmış kare sayısıdır ve oynatma saati bundan hesaplanır.
pub trait AudioSink: Send {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn write(&mut self, interleaved: &[f32]) -> usize;
    fn played_frames(&self) -> u64;
    fn pause(&mut self, on: bool);
    fn set_volume(&mut self, v: f32);
    /// Kuyruktaki çalınmamış sesi atar (atlamada); `played_frames` sayacı korunur.
    fn flush(&mut self) {}
}

/// Sessiz çıkış: yazılanı gerçek zamanda "çalar" (yazılandan ileri gitmez). Arabellek 200 ms.
pub struct NullSink {
    rate: u32,
    channels: u16,
    written: u64,
    /// Duraklatıldığında ya da kuyruk bittiğinde donmuş çalınan sayı.
    base: u64,
    since: Option<Instant>,
    manual: u64,
}

impl NullSink {
    pub fn new(rate: u32, channels: u16) -> NullSink {
        NullSink { rate, channels, written: 0, base: 0, since: None, manual: 0 }
    }

    /// Testler için: saati elle ilerletir.
    pub fn advance_frames(&mut self, n: u64) {
        self.manual += n;
    }

    fn realtime(&self) -> u64 {
        let run = self.since.map_or(0, |t| (t.elapsed().as_secs_f64() * self.rate as f64) as u64);
        (self.base + run).min(self.written)
    }
}

impl AudioSink for NullSink {
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn write(&mut self, interleaved: &[f32]) -> usize {
        let frames = (interleaved.len() / self.channels.max(1) as usize) as u64;
        let queued = self.written - self.realtime();
        let room = (self.rate as u64 / 5).saturating_sub(queued);
        let n = frames.min(room);
        if self.since.is_some() && self.realtime() == self.written {
            // Kuyruk boşalmıştı: saat yeni veriden başlar.
            self.base = self.written;
            self.since = Some(Instant::now());
        }
        self.written += n;
        n as usize
    }
    fn played_frames(&self) -> u64 {
        self.manual + self.realtime()
    }
    fn pause(&mut self, on: bool) {
        if on {
            self.base = self.realtime();
            self.since = None;
        } else if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }
    fn set_volume(&mut self, _v: f32) {}
    fn flush(&mut self) {
        self.base = self.realtime();
        self.written = self.base;
        if self.since.is_some() {
            self.since = Some(Instant::now());
        }
    }
}

#[cfg(windows)]
pub use wasapi::wasapi_sink;

/// On Linux the system's default output through cpal (ALSA; PipeWire and PulseAudio serve it
/// through their ALSA plugins). Keeps the Windows name so callers stay the same.
#[cfg(target_os = "linux")]
pub use cpal_out::cpal_sink as wasapi_sink;

/// No audio output on other platforms: callers fall back to [`NullSink`], so playback runs
/// silently on the same clock.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn wasapi_sink() -> anyhow::Result<Box<dyn AudioSink>> {
    anyhow::bail!("audio output is not implemented on this platform yet")
}

#[cfg(target_os = "linux")]
mod cpal_out {
    //! cpal pulls audio from a callback on its own thread; the player pushes. A queue of about
    //! 100 ms sits between them (as WASAPI's buffer does on Windows). The clock counts what
    //! the callback took, minus what the device still holds (cpal's predicted playback time),
    //! and moves smoothly between callbacks.

    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use anyhow::{Context, anyhow};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SizedSample};

    use super::AudioSink;

    /// Queue length between the player and the device.
    const BUFFER_SECS: f64 = 0.1;

    struct Inner {
        queue: VecDeque<f32>,
        /// Frames the callback has taken from the queue.
        consumed: u64,
        /// Frames heard at `at` (may be negative right after the start).
        base: f64,
        at: Option<Instant>,
        /// The device's latency at the last callback.
        latency: Duration,
        /// The last value `played` returned: the clock never runs backwards.
        last: u64,
        /// While paused, the clock stands here.
        frozen: Option<u64>,
    }

    /// What the player and the audio callback share.
    pub(super) struct Shared {
        rate: u32,
        channels: u16,
        capacity: usize,
        paused: AtomicBool,
        volume: AtomicU32,
        inner: Mutex<Inner>,
    }

    impl Shared {
        pub(super) fn new(rate: u32, channels: u16) -> Shared {
            Shared {
                rate,
                channels,
                capacity: (rate as f64 * BUFFER_SECS) as usize * channels.max(1) as usize,
                paused: AtomicBool::new(true),
                volume: AtomicU32::new(1.0f32.to_bits()),
                inner: Mutex::new(Inner {
                    queue: VecDeque::new(),
                    consumed: 0,
                    base: 0.0,
                    at: None,
                    latency: Duration::ZERO,
                    last: 0,
                    frozen: Some(0),
                }),
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
            self.inner.lock().unwrap_or_else(|e| e.into_inner())
        }

        /// The audio callback: fills `out` (interleaved) from the queue, silence for the rest.
        /// `latency` is how long until the first sample of `out` is heard.
        pub(super) fn fill(&self, out: &mut [f32], latency: Duration, now: Instant) {
            let ch = self.channels.max(1) as usize;
            let mut g = self.lock();
            g.latency = latency;
            if self.paused.load(Ordering::Relaxed) {
                out.fill(0.0);
                return;
            }
            let before = g.consumed;
            let n = out.len().min(g.queue.len()) / ch * ch;
            let vol = f32::from_bits(self.volume.load(Ordering::Relaxed));
            for (o, s) in out[..n].iter_mut().zip(g.queue.drain(..n)) {
                *o = s * vol;
            }
            out[n..].fill(0.0);
            g.consumed += (n / ch) as u64;
            // everything taken before this buffer, less what is still waiting in the device
            g.base = before as f64 - latency.as_secs_f64() * self.rate as f64;
            g.at = Some(now);
        }

        pub(super) fn played(&self, now: Instant) -> u64 {
            let mut g = self.lock();
            if let Some(f) = g.frozen {
                return f;
            }
            let est = match g.at {
                Some(at) => g.base + now.saturating_duration_since(at).as_secs_f64() * self.rate as f64,
                None => 0.0,
            };
            let v = (est.max(0.0) as u64).min(g.consumed).max(g.last);
            g.last = v;
            v
        }

        pub(super) fn write(&self, interleaved: &[f32]) -> usize {
            let ch = self.channels.max(1) as usize;
            let mut g = self.lock();
            let room = self.capacity.saturating_sub(g.queue.len()) / ch;
            let n = (interleaved.len() / ch).min(room);
            g.queue.extend(&interleaved[..n * ch]);
            n
        }

        pub(super) fn pause(&self, on: bool, now: Instant) {
            if on {
                let f = self.played(now);
                self.lock().frozen = Some(f);
                self.paused.store(true, Ordering::Relaxed);
            } else {
                let mut g = self.lock();
                if let Some(f) = g.frozen.take() {
                    // the device first plays out the silence it holds: the clock waits that long
                    g.base = f as f64 - g.latency.as_secs_f64() * self.rate as f64;
                    g.at = Some(now);
                    g.last = f;
                }
                self.paused.store(false, Ordering::Relaxed);
            }
        }

        pub(super) fn flush(&self) {
            self.lock().queue.clear();
        }

        fn set_volume(&self, v: f32) {
            self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    struct CpalSink {
        shared: Arc<Shared>,
        /// Dropping it ends the stream's thread (cpal streams can't move between threads).
        _stop: mpsc::Sender<()>,
    }

    impl AudioSink for CpalSink {
        fn sample_rate(&self) -> u32 {
            self.shared.rate
        }
        fn channels(&self) -> u16 {
            self.shared.channels
        }
        fn write(&mut self, interleaved: &[f32]) -> usize {
            self.shared.write(interleaved)
        }
        fn played_frames(&self) -> u64 {
            self.shared.played(Instant::now())
        }
        fn pause(&mut self, on: bool) {
            self.shared.pause(on, Instant::now());
        }
        fn set_volume(&mut self, v: f32) {
            self.shared.set_volume(v);
        }
        fn flush(&mut self) {
            self.shared.flush();
        }
    }

    fn build<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: cpal::StreamConfig, shared: Arc<Shared>) -> anyhow::Result<cpal::Stream> {
        let mut scratch: Vec<f32> = Vec::new();
        let stream = device.build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts.playback.saturating_duration_since(ts.callback);
                scratch.resize(out.len(), 0.0);
                shared.fill(&mut scratch, latency, Instant::now());
                for (o, s) in out.iter_mut().zip(&scratch) {
                    *o = T::from_sample(*s);
                }
            },
            // a lost device ends in silence; the clock then stops, as at the end of the audio
            |_e| {},
            None,
        )?;
        Ok(stream)
    }

    fn open() -> anyhow::Result<(cpal::Stream, Arc<Shared>)> {
        let device = cpal::default_host().default_output_device().context("no audio output device")?;
        let supported = device.default_output_config().context("audio output configuration")?;
        let config = supported.config();
        let shared = Arc::new(Shared::new(config.sample_rate, config.channels));
        let s = shared.clone();
        use cpal::SampleFormat as F;
        let stream = match supported.sample_format() {
            F::F32 => build::<f32>(&device, config, s),
            F::F64 => build::<f64>(&device, config, s),
            F::I16 => build::<i16>(&device, config, s),
            F::I32 => build::<i32>(&device, config, s),
            F::U16 => build::<u16>(&device, config, s),
            F::U8 => build::<u8>(&device, config, s),
            F::I8 => build::<i8>(&device, config, s),
            f => Err(anyhow!("unsupported sample format {f:?}")),
        }?;
        stream.play().context("starting audio output")?;
        Ok((stream, shared))
    }

    /// The default output device at its own rate and channel count.
    pub fn cpal_sink() -> anyhow::Result<Box<dyn AudioSink>> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        std::thread::Builder::new().name("audio-out".into()).spawn(move || match open() {
            Ok((stream, shared)) => {
                let _ = ready_tx.send(Ok(shared));
                // until the sink is dropped
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })?;
        let shared = ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("audio output didn't start"))??;
        Ok(Box::new(CpalSink { shared, _stop: stop_tx }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const MS: Duration = Duration::from_millis(1);

        #[test]
        fn clock_follows_what_was_heard() {
            // 1000 frames a second, mono: one frame per millisecond
            let s = Shared::new(1000, 1);
            let t0 = Instant::now();
            assert_eq!(s.write(&[0.5; 500]), 100, "the queue holds 100 ms");
            s.pause(false, t0);
            // the device holds 20 ms; it takes 10 frames
            let mut out = [0.0; 10];
            s.fill(&mut out, 20 * MS, t0);
            assert_eq!(out, [0.5; 10]);
            // 0 frames heard at t0 (none fed before), and none until the device's 20 ms pass
            assert_eq!(s.played(t0 + 5 * MS), 0);
            // later the estimate moves, but never past what was taken
            assert_eq!(s.played(t0 + 25 * MS), 5);
            assert_eq!(s.played(t0 + 500 * MS), 10);
            // next callback: 10 frames before it, 20 ms still in the device -> heard -10 at t1
            let t1 = t0 + 30 * MS;
            s.fill(&mut out, 20 * MS, t1);
            // the earlier value stands (no running backwards) until the estimate passes it
            assert_eq!(s.played(t1), 10);
            assert_eq!(s.played(t1 + 25 * MS), 15);
        }

        #[test]
        fn pause_freezes_and_resumes_after_the_device_latency() {
            let s = Shared::new(1000, 1);
            let t0 = Instant::now();
            s.write(&[0.1; 100]);
            s.pause(false, t0);
            let mut out = [1.0; 50];
            s.fill(&mut out, Duration::ZERO, t0);
            assert_eq!(s.played(t0 + 30 * MS), 30);
            s.pause(true, t0 + 30 * MS);
            assert_eq!(s.played(t0 + 400 * MS), 30);
            // paused: silence, nothing taken
            let mut out = [1.0; 10];
            s.fill(&mut out, 10 * MS, t0 + 400 * MS);
            assert_eq!(out, [0.0; 10]);
            // resumed: stands for the device's 10 ms of silence, then moves
            let t1 = t0 + 500 * MS;
            s.pause(false, t1);
            assert_eq!(s.played(t1 + 5 * MS), 30);
            assert_eq!(s.played(t1 + 15 * MS), 35);
        }

        #[test]
        fn volume_flush_and_stereo() {
            let s = Shared::new(1000, 2);
            s.set_volume(0.5);
            assert_eq!(s.write(&[1.0, -1.0, 1.0, -1.0, 1.0]), 2, "whole frames only");
            s.pause(false, Instant::now());
            let mut out = [9.0; 6];
            s.fill(&mut out, Duration::ZERO, Instant::now());
            assert_eq!(out, [0.5, -0.5, 0.5, -0.5, 0.0, 0.0]);
            s.write(&[1.0; 20]);
            s.flush();
            let mut out = [9.0; 4];
            s.fill(&mut out, Duration::ZERO, Instant::now());
            assert_eq!(out, [0.0; 4]);
        }
    }
}

#[cfg(windows)]
mod wasapi {
    use super::AudioSink;
    use anyhow::Context;
    use windows::Win32::Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
        IAudioClock, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};

    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    /// Arabellek süresi (100 ns birimi): 100 ms.
    const BUFFER_HNS: i64 = 1_000_000;

    struct Wasapi {
        client: IAudioClient,
        render: IAudioRenderClient,
        clock: IAudioClock,
        clock_freq: u64,
        buffer_frames: u32,
        rate: u32,
        channels: u16,
        volume: f32,
        started: bool,
        paused: bool,
        /// Atlamada temizlenen kuyruk saat konumunu geri götürmesin diye eklenen düzeltme.
        played_offset: u64,
    }

    // WASAPI nesneleri serbest iş parçacıklıdır (MTA); çıkış tek bir iş parçacığından sürülür.
    unsafe impl Send for Wasapi {}

    /// Varsayılan hoparlöre paylaşımlı modda açılan çıkış (aygıtın örnekleme hızı ve kanal sayısıyla, 32 bit kayan nokta).
    pub fn wasapi_sink() -> anyhow::Result<Box<dyn AudioSink>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator")?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).context("default audio endpoint")?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).context("IAudioClient")?;
            let mix = client.GetMixFormat()?;
            let (rate, channels) = ((*mix).nSamplesPerSec, (*mix).nChannels);
            CoTaskMemFree(Some(mix as *const _));
            let block = channels * 4;
            let fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
                nChannels: channels,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * block as u32,
                nBlockAlign: block,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                BUFFER_HNS,
                0,
                &fmt,
                None,
            )?;
            let buffer_frames = client.GetBufferSize()?;
            let render: IAudioRenderClient = client.GetService()?;
            let clock: IAudioClock = client.GetService()?;
            let clock_freq = clock.GetFrequency()?;
            Ok(Box::new(Wasapi {
                client,
                render,
                clock,
                clock_freq,
                buffer_frames,
                rate,
                channels,
                volume: 1.0,
                started: false,
                paused: true,
                played_offset: 0,
            }))
        }
    }

    impl Wasapi {
        fn device_played(&self) -> u64 {
            let mut pos = 0u64;
            if unsafe { self.clock.GetPosition(&mut pos, None) }.is_err() || self.clock_freq == 0 {
                return 0;
            }
            (pos as u128 * self.rate as u128 / self.clock_freq as u128) as u64
        }
    }

    impl AudioSink for Wasapi {
        fn sample_rate(&self) -> u32 {
            self.rate
        }
        fn channels(&self) -> u16 {
            self.channels
        }
        fn write(&mut self, interleaved: &[f32]) -> usize {
            let ch = self.channels as usize;
            let padding = unsafe { self.client.GetCurrentPadding() }.unwrap_or(self.buffer_frames);
            let room = self.buffer_frames.saturating_sub(padding) as usize;
            let n = (interleaved.len() / ch).min(room);
            if n == 0 {
                return 0;
            }
            unsafe {
                let Ok(ptr) = self.render.GetBuffer(n as u32) else { return 0 };
                let out = std::slice::from_raw_parts_mut(ptr as *mut f32, n * ch);
                for (o, i) in out.iter_mut().zip(interleaved) {
                    *o = i * self.volume;
                }
                let _ = self.render.ReleaseBuffer(n as u32, 0);
                if !self.started && !self.paused {
                    let _ = self.client.Start();
                    self.started = true;
                }
            }
            n
        }
        fn played_frames(&self) -> u64 {
            self.device_played() + self.played_offset
        }
        fn pause(&mut self, on: bool) {
            self.paused = on;
            unsafe {
                if on {
                    let _ = self.client.Stop();
                    self.started = false;
                } else if !self.started {
                    let _ = self.client.Start();
                    self.started = true;
                }
            }
        }
        fn set_volume(&mut self, v: f32) {
            self.volume = v.clamp(0.0, 1.0);
        }
        fn flush(&mut self) {
            let before = self.played_frames();
            unsafe {
                let _ = self.client.Stop();
                let _ = self.client.Reset();
            }
            // Reset aygıt saatini sıfırlar: sayaç geri gitmesin.
            self.played_offset = before.saturating_sub(self.device_played());
            self.started = false;
            if !self.paused {
                unsafe {
                    let _ = self.client.Start();
                }
                self.started = true;
            }
        }
    }

    impl Drop for Wasapi {
        fn drop(&mut self) {
            unsafe {
                let _ = self.client.Stop();
            }
        }
    }
}
