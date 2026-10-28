//! In-app MV playback on libmpv, with no Slint in sight.
//!
//! One process owns one [`VideoPlayer`]. It starts two threads: an event thread that owns the
//! mpv handle and blocks in `mpv_wait_event`, and a render thread that owns the software render
//! context and publishes the latest frame through a `watch`. Commands travel the other way over
//! an unbounded channel and wake the event thread with `mpv_wakeup`, so neither thread polls on
//! a timer.
//!
//! The crate deliberately names no UI type. The Slint bridge lives in `nekonetease-views` and
//! turns [`VideoFrame`] into an `Image`, which is the only copy of a frame that is unavoidable.

mod frame;
mod player;
mod render_sw;
mod worker;

pub use frame::{VideoError, VideoFrame, VideoState};
pub use player::VideoPlayer;
