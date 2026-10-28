//! The values that cross the crate boundary, plus the arithmetic the worker and the bridge
//! must agree on.

use std::fmt;

/// One frame in RGBA8, row-major and tightly packed.
///
/// The renderer hands back premultiplied-looking bytes with the alpha channel set to `0xff`;
/// the bridge copies them straight into a Slint `Rgba8Pixel` buffer.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl VideoFrame {
    pub const BYTES_PER_PIXEL: usize = 4;

    /// Bytes per row. Rows are tightly packed, so this is `width * 4`.
    #[must_use]
    pub fn stride(&self) -> usize {
        usize::try_from(self.width).unwrap_or(usize::MAX).saturating_mul(Self::BYTES_PER_PIXEL)
    }

    /// Total bytes the frame holds.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.stride().saturating_mul(usize::try_from(self.height).unwrap_or(usize::MAX))
    }
}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
            .finish()
    }
}

/// What the overlay paints from. `Failed` carries user-facing text from mpv.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoState {
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Failed(String),
}

impl VideoState {
    /// Whether the overlay should be mounted: a file was asked for and has not finished.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Loading | Self::Playing | Self::Paused)
    }
}

/// The synchronous half of a command that never reaches mpv.
#[derive(Debug)]
pub enum VideoError {
    /// Built without libmpv, or its client API could not be initialised.
    Unavailable,
    /// The owning thread is gone, usually after `shutdown`.
    Closed,
    /// A command the worker refused outright.
    Command(String),
    /// The render context refused an operation, with libmpv's own words for it.
    Render(String),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("video playback is unavailable"),
            Self::Closed => f.write_str("video playback has shut down"),
            Self::Command(message) | Self::Render(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for VideoError {}

impl From<String> for VideoError {
    fn from(message: String) -> Self {
        Self::Render(message)
    }
}

/// Ceiling on the software-rendered frame, in physical pixels.
///
/// The overlay draws at most a little over 720 px wide at the usual window sizes, and the
/// software renderer is one CPU thread: every pixel past this is a cost nothing displays.
/// 960x540 is also the smallest common 16:9 tier above the overlay's real draw size.
pub const MAX_RENDER_WIDTH: u32 = 960;
pub const MAX_RENDER_HEIGHT: u32 = 540;

/// Scales a video down to fit the cap, preserving its aspect ratio. Never upscales.
pub(crate) fn render_size(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    if width <= MAX_RENDER_WIDTH && height <= MAX_RENDER_HEIGHT {
        return Some((width, height));
    }

    let width_limited = u64::from(width) * u64::from(MAX_RENDER_HEIGHT)
        >= u64::from(height) * u64::from(MAX_RENDER_WIDTH);
    let (scaled_width, scaled_height) = if width_limited {
        let height =
            u32::try_from(u64::from(height) * u64::from(MAX_RENDER_WIDTH) / u64::from(width))
                .unwrap_or(MAX_RENDER_HEIGHT);
        (MAX_RENDER_WIDTH, height)
    } else {
        let width =
            u32::try_from(u64::from(width) * u64::from(MAX_RENDER_HEIGHT) / u64::from(height))
                .unwrap_or(MAX_RENDER_WIDTH);
        (width, MAX_RENDER_HEIGHT)
    };
    Some((scaled_width.max(2), scaled_height.max(2)))
}

/// Maps the app's 0..=1 volume onto mpv's 0..=100 property, treating nonsense as silence.
pub(crate) fn volume_to_mpv(volume: f64) -> f64 {
    if volume.is_finite() { volume.clamp(0.0, 1.0) * 100.0 } else { 0.0 }
}

/// Hardware decoding is the default: mpv's `auto-safe` whitelist only picks APIs the host is
/// known to drive correctly, and falls back to software by itself where it cannot.
pub(crate) const HWDEC_ENABLED: &str = "auto-safe";
pub(crate) const HWDEC_DISABLED: &str = "no";

pub(crate) fn hwdec_option(enabled: bool) -> &'static str {
    if enabled { HWDEC_ENABLED } else { HWDEC_DISABLED }
}

/// Turns mpv's seconds into the milliseconds both the API and the progress watch carry.
pub(crate) fn secs_to_ms(secs: f64) -> u64 {
    std::time::Duration::try_from_secs_f64(secs)
        .map_or(0, |duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// A display title for a URL, used when the caller did not supply one.
///
/// The fragment and query never reach the title: a stream URL like `…/mv.m3u8?token=…` would
/// otherwise paint a credential in the overlay.
pub(crate) fn title_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
    if name.is_empty() { url.to_owned() } else { name.to_owned() }
}

/// Names a libmpv error with its actual message; its `Display` is just the raw variant.
pub(crate) fn mpv_error_text(error: &libmpv2::Error) -> String {
    match error {
        libmpv2::Error::Raw(code) => libmpv2_sys::mpv_error_str(*code).to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/frame_tests.rs"]
mod tests;
