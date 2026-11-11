//! Video ve ses görüntüleyici (rf-player: FilmCraft çözücüleri).
//! Video: kareler pencere boyutuna ölçeklenip arka planda BGRA'ya çevrilir; başlık çubuğu ve
//! kontroller içeriğin üstünde durur, fare durunca gizlenir. Tıklama oynat/duraklat, çift tık
//! tam ekran, tekerlek ses düzeyi (0,04). Ses: 500×300 kart (kapak, başlık/sanatçı/albüm,
//! süre, eşzamanlı şarkı sözü).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as L;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rf_player::{AudioSink, Player, PlayerState};

use crate::look::viewer::{Prepared, Probe, Viewer};
use crate::look::config::LookSettings;
use crate::t;

/// Kontroller fare durunca bu kadar sonra gizlenir.
const HIDE_AFTER: Duration = Duration::from_millis(1500);
/// Oynatma döngüsünün yoklama aralığı.
const TICK: Duration = Duration::from_millis(8);
/// While the timeline is dragged the player is sent a new position at most this often (every
/// jump makes it decode from a key frame; the bar itself follows the pointer at once).
const SCRUB_EVERY: Duration = Duration::from_millis(80);
/// After a jump the bar stays on its target this long at most, until the player reports it.
const SEEK_HOLD: Duration = Duration::from_millis(800);

/// The timeline being dragged (GPUI's drag: its moves arrive wherever the pointer goes).
struct SeekDrag;

/// Nothing follows the pointer while the timeline is dragged.
struct SeekGhost;

impl Render for SeekGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Default)]
pub struct VideoViewer {
    view: Option<Entity<MediaView>>,
    /// The file opened off the UI thread (a network drive), see `Viewer::loader`.
    opened: Option<Opened>,
}

/// The route a file plays by and the window it asks for.
struct Opened(Kind, Prepared);

/// Opens `path`: reads the container's index (on a network drive this is the slow part) and
/// picks the route it plays by.
fn open_media(path: &std::path::Path) -> Opened {
    let name = super::file_name(path);
    let opened = rf_player::open(path);
    let (kind, prepared) = match opened {
        Ok(media) => match media.info().video.clone() {
            Some(v) => {
                let par = if v.par.1 > 0 { v.par.0 as f32 / v.par.1 as f32 } else { 1.0 };
                let size = ((v.width as f32 * par).max(160.0), (v.height as f32).max(90.0));
                let mut p = Prepared::new(size, name);
                p.overlay_titlebar = true;
                p.drag_by_body = false;
                (Kind::Video(media), p)
            }
            None => {
                let mut p = Prepared::new((500.0, 300.0), name);
                p.resizable = false;
                (Kind::Audio(media), p)
            }
        },
        Err(e) => (Kind::Failed(format!("{e:#}")), Prepared::new((520.0, 220.0), name)),
    };
    Opened(kind, prepared)
}

impl Viewer for VideoViewer {
    fn name(&self) -> &'static str {
        "video"
    }
    fn priority(&self) -> i32 {
        -3
    }
    fn can_handle(&self, f: &Probe) -> bool {
        // `.ts` çoğunlukla TypeScript: yalnız MPEG-TS paket eşitlemesi varsa video
        if f.ext == "ts" && !is_transport_stream(&f.head) {
            return false;
        }
        f.is_file() && rf_player::can_open(&f.path)
    }
    fn loader(&self) -> Option<fn(&Probe) -> crate::look::viewer::Loaded> {
        Some(|f| Box::new(open_media(&f.path)))
    }
    fn take_loaded(&mut self, loaded: crate::look::viewer::Loaded) {
        self.opened = loaded.downcast::<Opened>().ok().map(|o| *o);
    }
    fn prepare(&mut self, f: &Probe, cx: &mut App) -> Prepared {
        // yalnız dizin okunur (hızlı); ilk karenin çözülebildiği arka planda denenir (MediaView::new)
        let Opened(kind, prepared) = self.opened.take().unwrap_or_else(|| open_media(&f.path));
        let path = f.path.clone();
        self.view = Some(cx.new(|cx| MediaView::new(path, kind, cx)));
        prepared
    }
    fn render(&mut self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        self.view.as_ref().map_or_else(|| div().into_any_element(), |v| v.clone().into_any_element())
    }
    fn cleanup(&mut self) {
        self.view = None;
    }
}

/// MPEG-TS: 188 baytlık (ya da 4 bayt zaman damgalı 192 baytlık) paketlerin başında 0x47.
pub(crate) fn is_transport_stream(head: &[u8]) -> bool {
    let synced = |start: usize, step: usize| (0..3).all(|k| head.get(start + k * step) == Some(&0x47));
    synced(0, 188) || synced(4, 192)
}

enum Kind {
    Video(rf_player::Media),
    Audio(rf_player::Media),
    Failed(String),
}

fn sink() -> Box<dyn AudioSink> {
    if std::env::var_os("RFILES_MUTE").is_some() {
        return Box::new(rf_player::NullSink::new(48000, 2));
    }
    rf_player::cpal_sink().unwrap_or_else(|e| {
        log::warn!("look audio: {e:#}");
        Box::new(rf_player::NullSink::new(48000, 2))
    })
}

fn clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

fn bgra_image(w: u32, h: u32, data: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buf = image::RgbaImage::from_raw(w, h, data)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

pub struct MediaView {
    path: PathBuf,
    video: bool,
    failed: Option<String>,
    player: Option<Player>,
    state: PlayerState,
    image: Option<Arc<RenderImage>>,
    /// Bir önceki kare: GPU dokusu bir kare sonra bırakılır (o an çizimde olabilir).
    previous: Option<Arc<RenderImage>>,
    shown_pts: f64,
    converting: bool,
    /// Kare çevrilecek boyut (aygıt pikseli).
    target: (u32, u32),
    volume: f32,
    muted: bool,
    looping: bool,
    controls: bool,
    hovered_controls: bool,
    bar: Rc<Cell<Option<Bounds<Pixels>>>>,
    meta: Vec<(&'static str, String)>,
    cover: Option<Arc<RenderImage>>,
    images: Option<Entity<crate::image_cache::ImageCache>>,
    lyrics: Vec<(f64, String)>,
    first_frame_logged: bool,
    /// Görüntü boyutu (piksel en-boyu uygulanmış).
    dims: (f32, f32),
    /// The timeline is being dragged: the fraction under the pointer, which the bar shows
    /// instead of the player's position.
    scrub: Option<f64>,
    /// The fraction last sent to the player during the drag, and when.
    scrub_sent: Option<(f64, Instant)>,
    /// Playback was paused for the drag and goes on when it ends.
    scrub_resume: bool,
    /// The last jump's target and time: the bar stays there until the player reports it.
    seek_hold: Option<(f64, Instant)>,
    _hide: Option<Task<()>>,
    _tasks: Vec<Task<()>>,
}

impl MediaView {
    fn new(path: PathBuf, kind: Kind, cx: &mut Context<Self>) -> Self {
        let s = cx.global::<LookSettings>();
        let (volume, looping) = (s.look_volume.clamp(0.0, 1.0), s.look_loop);
        let mut v = Self {
            path: path.clone(),
            video: false,
            failed: None,
            player: None,
            state: PlayerState::default(),
            image: None,
            previous: None,
            shown_pts: f64::NAN,
            converting: false,
            target: (0, 0),
            volume,
            muted: false,
            looping,
            controls: true,
            hovered_controls: false,
            bar: Default::default(),
            meta: Vec::new(),
            cover: None,
            images: None,
            lyrics: Vec::new(),
            first_frame_logged: false,
            dims: (16.0, 9.0),
            scrub: None,
            scrub_sent: None,
            scrub_resume: false,
            seek_hold: None,
            _hide: None,
            _tasks: Vec::new(),
        };
        v.start_kind(kind, cx);
        if !v.video && v.failed.is_none() {
            v.load_audio_info(cx);
        }
        // pencere kapanınca: ses durur, GPU dokuları bırakılır
        cx.on_release(|v: &mut Self, cx| {
            v.player = None;
            for img in [v.image.take(), v.previous.take(), v.cover.take()].into_iter().flatten() {
                cx.drop_image(img, None);
            }
        })
        .detach();
        v.arm_hide(cx);
        v
    }

    /// Starts what `kind` says: the video, the audio card, or the reason it can't play.
    fn start_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        match kind {
            Kind::Video(media) | Kind::Audio(media) => {
                self.video = media.info().has_video();
                if let Some(i) = media.info().video.as_ref() {
                    let par = if i.par.1 > 0 { i.par.0 as f32 / i.par.1 as f32 } else { 1.0 };
                    self.dims = ((i.width as f32 * par).max(1.0), (i.height as f32).max(1.0));
                }
                if self.video {
                    self.probe_first_frame(media.clone(), cx);
                }
                let (volume, looping) = (self.volume, self.looping);
                let mut p = Player::new(media, sink());
                p.set_volume(volume);
                p.set_loop(looping);
                p.play();
                self.player = Some(p);
                self.tick(cx);
            }
            Kind::Failed(e) => self.failed = Some(e),
        }
    }

    /// FilmCraft bazı biçimleri (ör. 4:2:2 HEVC) açar ama karesini çözemez: ilk kare arka planda
    /// denenir, olmazsa açıklama gösterilir.
    fn probe_first_frame(&mut self, media: rf_player::Media, cx: &mut Context<Self>) {
        let probe = cx.background_spawn(async move { media.frame_at(0.0).err().map(|e| format!("{e:#}")) });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let Some(err) = probe.await else { return };
            let _ = this.update(cx, |v, cx| {
                v.player = None;
                v.failed = Some(err);
                cx.notify();
            });
        }));
    }

    fn load_audio_info(&mut self, cx: &mut Context<Self>) {
        let p = self.path.clone();
        let work = cx.background_spawn(async move {
            let meta = rf_linux::props::media_props(&p);
            let lyrics = crate::look::lyrics::sidecar(&p).unwrap_or_default();
            (meta, lyrics)
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let (meta, lyrics) = work.await;
            if std::env::var_os("RFILES_LOOK_TRACE").is_some() {
                eprintln!("look: ses bilgisi {meta:?}, {} söz satırı", lyrics.len());
            }
            let _ = this.update(cx, |v, cx| {
                v.meta = meta;
                v.lyrics = lyrics;
                cx.notify();
            });
        }));
        // kapak: kabuk küçük resmi, paylaşılan önbellekten (kendi iş parçacıklarında, mesaj döngülü)
        let images = crate::look::LookManager::images(cx);
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        self.images = Some(images);
    }

    /// Oynatma döngüsü: yeni kare varsa arka planda çevrilir; durum güncellenir.
    fn tick(&mut self, cx: &mut Context<Self>) {
        self._tasks.push(cx.spawn(async move |this, cx| {
            // paused or ended: a slower look (a click or a seek still shows within a few frames)
            let mut wait = TICK;
            loop {
                cx.background_executor().timer(wait).await;
                let job = this.update(cx, |v, cx| {
                    let p = v.player.as_ref()?;
                    let st = p.state();
                    wait = if st.playing { TICK } else { std::time::Duration::from_millis(33) };
                    let coarse = |s: &PlayerState| ((s.position * 10.0) as i64, s.playing, s.ended, (s.duration * 10.0) as i64);
                    if coarse(&st) != coarse(&v.state) {
                        cx.notify();
                    }
                    v.state = st;
                    if !v.video || v.converting || v.target.0 == 0 {
                        return None;
                    }
                    let f = p.current_frame()?;
                    if f.pts == v.shown_pts {
                        return None;
                    }
                    v.converting = true;
                    Some((f, v.target))
                });
                let Ok(job) = job else { break };
                let Some((frame, (w, h))) = job else { continue };
                let pts = frame.pts;
                let data = cx.background_spawn(async move { rf_player::to_bgra(&frame, w, h) }).await;
                let img = bgra_image(w, h, data);
                if this
                    .update(cx, |v, cx| {
                        v.converting = false;
                        v.shown_pts = pts;
                        if let Some(old) = v.previous.take() {
                            cx.drop_image(old, None);
                        }
                        v.previous = std::mem::replace(&mut v.image, img);
                        if !v.first_frame_logged {
                            v.first_frame_logged = true;
                            crate::look::trace::mark("ilk video karesi");
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn arm_hide(&mut self, cx: &mut Context<Self>) {
        self._hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIDE_AFTER).await;
            let _ = this.update(cx, |v, cx| {
                // (hover is not reported while the timeline is dragged: the drag keeps them up)
                if !v.hovered_controls && v.scrub.is_none() && v.video {
                    v.controls = false;
                    cx.notify();
                }
            });
        }));
    }

    fn show_controls(&mut self, cx: &mut Context<Self>) {
        if !self.controls {
            self.controls = true;
            cx.notify();
        }
        self.arm_hide(cx);
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.player {
            if p.state().playing {
                p.pause();
            } else {
                p.play();
            }
        }
        cx.notify();
    }

    /// The timeline is pressed or dragged to `f` (0..1): the bar goes there at once, the player
    /// follows (at most every `SCRUB_EVERY`, and once more when the drag ends).
    fn scrub_to(&mut self, f: f64, cx: &mut Context<Self>) {
        let f = f.clamp(0.0, 1.0);
        if self.scrub.is_none() {
            self.scrub_sent = None;
            self.scrub_resume = false;
            // our own player would play a scrap of sound after every jump: it waits for the
            // drag to end (its picture follows a jump while paused too)
            if let Some(p) = self.player.as_mut().filter(|p| p.state().playing) {
                p.pause();
                self.scrub_resume = true;
            }
        }
        self.scrub = Some(f);
        if self.scrub_sent.is_none_or(|(_, at)| at.elapsed() >= SCRUB_EVERY) {
            self.scrub_sent = Some((f, Instant::now()));
            self.seek_fraction(f);
        }
        self.show_controls(cx);
        cx.notify();
    }

    /// The button is up: the player goes to where the drag ended and plays on if it was playing.
    fn end_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(f) = self.scrub.take() else { return };
        if self.scrub_sent.take().is_none_or(|(sent, _)| sent != f) {
            self.seek_fraction(f);
        }
        if std::mem::take(&mut self.scrub_resume) {
            if let Some(p) = &mut self.player {
                p.play();
            }
        }
        self.arm_hide(cx);
        cx.notify();
    }

    fn seek_fraction(&mut self, f: f64) {
        let f = f.clamp(0.0, 1.0);
        self.seek_hold = Some((f, Instant::now()));
        if let Some(p) = &mut self.player {
            let d = p.state().duration;
            p.seek(f * d);
        }
    }

    fn set_volume(&mut self, v: f32, cx: &mut Context<Self>) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(p) = &mut self.player {
            p.set_volume(self.volume);
        }
        crate::look::config::update(cx, |s| s.look_volume = self.volume);
        cx.notify();
    }

    fn render_controls(&self, light_text: bool, cx: &mut Context<Self>) -> AnyElement {
        let (playing, pos, dur) = (self.state.playing, self.state.position, self.state.duration);
        let reported = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 };
        // the pointer's place while dragging; after a jump its target, until the player is there
        let held = self.seek_hold.filter(|(f, at)| at.elapsed() < SEEK_HOLD && (reported - f).abs() > 0.01).map(|(f, _)| f);
        let shown = self.scrub.or(held).unwrap_or(reported);
        let pos = if self.scrub.is_some() || held.is_some() { shown * dur } else { pos };
        let frac = shown as f32;
        let scrubbing = self.scrub.is_some();
        // (paused only for the drag: the button keeps saying what happens when it ends)
        let playing = playing || self.scrub_resume;
        let fg: Hsla = if light_text { white().into() } else { rgb(0x1a1a1a).into() };
        let track: Hsla = if light_text { hsla(0., 0., 1., 0.3) } else { hsla(0., 0., 0., 0.2) };
        let accent = cx.global::<gpui_kit::component::Theme>().primary;
        let bar = self.bar.clone();
        let btn = |id: &'static str, icon: L, active: bool| {
            div()
                .id(id)
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .when(active, |d| d.bg(hsla(0., 0., 1., 0.18)))
                .hover(|s| s.bg(hsla(0., 0., 1., 0.12)))
                .child(Icon::new(icon).size(px(16.)).text_color(fg))
        };
        h_flex()
            .id("look-media-controls")
            .w_full()
            .gap_2()
            .px_3()
            .py_2()
            .items_center()
            .text_color(fg)
            .text_xs()
            .on_hover(cx.listener(|v, h: &bool, _, _| v.hovered_controls = *h))
            // A press on the controls stops here. The picture behind them toggles play on every
            // press: reaching it, a click on the pause button paused on the press and played
            // again on the release, and the loop and mute buttons paused the video too.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(btn("media-play", if playing { L::Pause } else { L::Play }, false).on_click(cx.listener(|v, _, _, cx| v.toggle_play(cx))))
            .child(div().w(px(84.)).flex_shrink_0().child(format!("{} / {}", clock(pos), clock(dur))))
            .child(
                div()
                    .id("media-seek")
                    .flex_1()
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .child({
                        let bar = bar.clone();
                        canvas(move |b, _, _| bar.set(Some(b)), |_, _, _, _| {}).absolute().size_full()
                    })
                    .group("media-seek")
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(track)
                            .child(div().h_full().w(relative(frac)).rounded(px(2.)).bg(accent))
                            // the handle: there while the pointer is on the timeline or drags it
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-4.))
                                    .left(relative(frac))
                                    .ml(px(-6.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(fg)
                                    .when(!scrubbing, |d| d.opacity(0.).group_hover("media-seek", |s| s.opacity(1.))),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(move |v, ev: &MouseDownEvent, _, cx| {
                        if let Some(b) = bar.get() {
                            v.scrub_to((f32::from(ev.position.x - b.origin.x) / f32::from(b.size.width).max(1.0)) as f64, cx);
                        }
                        cx.stop_propagation();
                    }))
                    // dragging: the moves keep coming when the pointer leaves the thin bar (and
                    // the window); the button going up anywhere ends it
                    .on_drag(SeekDrag, |_, _, _, cx| cx.new(|_| SeekGhost))
                    .on_drag_move(cx.listener(|v, ev: &DragMoveEvent<SeekDrag>, _, cx| {
                        if v.scrub.is_none() {
                            return;
                        }
                        if ev.event.pressed_button != Some(MouseButton::Left) {
                            v.end_scrub(cx);
                            return;
                        }
                        let b = ev.bounds;
                        v.scrub_to((f32::from(ev.event.position.x - b.origin.x) / f32::from(b.size.width).max(1.0)) as f64, cx);
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|v, _: &MouseUpEvent, _, cx| v.end_scrub(cx)))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(|v, _: &MouseUpEvent, _, cx| v.end_scrub(cx))),
            )
            .child(btn("media-loop", L::RotateCw, self.looping).on_click(cx.listener(|v, _, _, cx| {
                    v.looping = !v.looping;
                    if let Some(p) = &mut v.player {
                        p.set_loop(v.looping);
                    }
                    let on = v.looping;
                    crate::look::config::update(cx, |s| s.look_loop = on);
                    cx.notify();
                })))
                .child(btn("media-mute", if self.muted { L::Minus } else { L::Music }, self.muted).on_click(cx.listener(|v, _, _, cx| {
                    v.muted = !v.muted;
                    let m = v.muted;
                    if let Some(p) = &mut v.player {
                        p.set_mute(m);
                    }
                    cx.notify();
                })))
            .child(div().w(px(34.)).text_right().child(format!("{:.0}%", self.volume * 100.0)))
            .into_any_element()
    }
}

impl Render for MediaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = crate::theme::Palette::for_window(window);
        if let Some(e) = &self.failed {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(div().text_color(p.text).child(t!("Bu dosya oynatılamıyor")))
                .child(div().text_xs().text_color(p.text_secondary).child(e.clone()))
                .into_any_element();
        }
        if !self.video {
            return self.render_audio(window, cx);
        }
        // video: alana sığan dikdörtgen
        let area = window.viewport_size();
        let (aw, ah) = (f32::from(area.width), f32::from(area.height));
        let (vw, vh) = self.dims;
        let s = (aw / vw).min(ah / vh);
        let (dw, dh) = ((vw * s).floor(), (vh * s).floor());
        let scale = window.scale_factor();
        // converted on the CPU at most at the video's own size: a larger window is filled by the
        // GPU stretching the picture (a 1080p clip full screen on 4K was scaled to 33 MB a frame)
        let k = ((vw / (dw * scale)).min(vh / (dh * scale))).min(1.0);
        self.target = (((dw * scale * k) as u32).max(2), ((dh * scale * k) as u32).max(2));
        let controls = self.controls;
        let mut root = div()
            .id("look-media")
            .relative()
            .size_full()
            .bg(black())
            .on_mouse_move(cx.listener(|v, ev: &MouseMoveEvent, _, cx| {
                // a drag of the timeline whose button-up never arrived
                if v.scrub.is_some() && ev.pressed_button != Some(MouseButton::Left) {
                    v.end_scrub(cx);
                }
                v.show_controls(cx);
            }))
            .on_scroll(cx.listener(|v, ev: &ScrollEvent, _, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y) / 100.0,
                    ScrollDelta::Lines(l) => l.y / 3.0,
                };
                v.set_volume(v.volume + 0.04 * dy, cx);
                v.show_controls(cx);
            }))
            // A click on the picture plays / pauses at once. A double click is fullscreen: its
            // second press takes the first one's toggle back, so playback is as it was. (The
            // click count keeps growing while clicks come quickly: every even one is a second.)
            .on_mouse_down(MouseButton::Left, cx.listener(|v, ev: &MouseDownEvent, window, cx| {
                v.toggle_play(cx);
                if ev.click_count % 2 == 0 {
                    window.toggle_fullscreen();
                }
            }));
        if let Some(img) = self.image.clone() {
            root = root.child(
                gpui_kit::img(img)
                    .absolute()
                    .left(px(((aw - dw) / 2.0).floor()))
                    .top(px(((ah - dh) / 2.0).floor()))
                    .w(px(dw))
                    .h(px(dh))
                    .object_fit(ObjectFit::Fill),
            );
        }
        if controls {
            root = root.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .bg(linear_gradient(0., linear_color_stop(hsla(0., 0., 0., 0.7), 0.), linear_color_stop(hsla(0., 0., 0., 0.0), 1.)))
                    .pt_6()
                    .child(self.render_controls(true, cx)),
            );
        }
        root.into_any_element()
    }
}

impl MediaView {
    fn render_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let lyric = crate::look::lyrics::line_at(&self.lyrics, self.state.position).map(str::to_string);
        if self.cover.is_none() {
            // (no disk access here: this runs every frame, and the file may be on a network drive)
            if let (Some(images), Some(dir)) = (self.images.clone(), self.path.parent()) {
                let name = super::file_name(&self.path);
                let e = rf_core::Entry::new(&name, rf_core::EntryKind::File, 0, 0, 0, 0);
                let dir = dir.to_path_buf();
                // önbellek kendi dokusunu yönetir: burada bırakılmaz
                let thumb = images.update(cx, |c, cx| c.thumbnail(&dir, &e, 256, cx));
                if let Some(t) = thumb {
                    return self.render_audio_with(t, window, cx, lyric);
                }
            }
        }
        let cover = self.cover.clone();
        self.render_audio_inner(cover, window, cx, lyric)
    }

    fn render_audio_with(&mut self, cover: Arc<RenderImage>, window: &mut Window, cx: &mut Context<Self>, lyric: Option<String>) -> AnyElement {
        self.render_audio_inner(Some(cover), window, cx, lyric)
    }

    fn render_audio_inner(&mut self, cover: Option<Arc<RenderImage>>, window: &mut Window, cx: &mut Context<Self>, lyric: Option<String>) -> AnyElement {
        let p = crate::theme::Palette::for_window(window);
        let field = |k: &str| self.meta.iter().find(|(l, _)| *l == k).map(|(_, v)| v.clone());
        let title = field("Başlık").unwrap_or_else(|| super::file_name(&self.path));
        let artist = field("Sanatçı");
        let album = field("Albüm");
        h_flex()
            .size_full()
            .p_4()
            .gap_4()
            .items_center()
            .child(
                div()
                    .size(px(150.))
                    .flex_shrink_0()
                    .rounded(px(8.))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(p.card_bg)
                    .map(|d| match cover {
                        Some(c) => d.child(gpui_kit::img(c).size_full().object_fit(ObjectFit::Cover)).into_any_element(),
                        None => d.child(Icon::new(L::Music).size(px(56.)).text_color(p.text_secondary)).into_any_element(),
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(p.text).truncate().child(title))
                    .when_some(artist, |d, a| d.child(div().text_sm().text_color(p.text_secondary).truncate().child(a)))
                    .when_some(album, |d, a| d.child(div().text_xs().text_color(p.text_tertiary).truncate().child(a)))
                    .when_some(lyric, |d, l| d.child(div().pt_2().text_sm().text_color(p.text).child(l)))
                    .child(div().pt_2().child(self.render_controls(p.dark, cx))),
            )
            .into_any_element()
    }
}
