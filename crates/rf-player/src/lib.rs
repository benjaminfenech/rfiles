//! Önizleme için saf Rust oynatıcı: FilmCraft çözücüleriyle açma ve çözme, ses çıkışı, video çizimi.

mod audio_out;
mod clock;
mod convert;
/// Linux only: FFmpeg for what FilmCraft can't play.
pub mod ffmpeg;
mod player;
mod source;

pub use audio_out::{AudioSink, NullSink, cpal_sink};
pub use clock::Clock;
pub use convert::to_bgra;
pub use player::{Player, PlayerState};
pub use source::{Frame, Media, Planes, can_open, open};
