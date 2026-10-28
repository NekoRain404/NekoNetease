//! Pure accounting over playback observations; no device, database or network dependencies.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ListenKey {
    pub song_id: i64,
    pub playback_generation: u64,
    pub account_generation: u64,
}

#[derive(Clone, Copy)]
pub(super) struct ListenSample {
    pub key: Option<ListenKey>,
    pub playing: bool,
    pub position_ms: u64,
    pub speed: f64,
    pub decoder_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ListenReport {
    pub key: ListenKey,
    pub seconds: u64,
}

#[derive(Default)]
pub(super) struct ListenTracker {
    previous: Option<ListenSample>,
    listened: Duration,
}

impl ListenTracker {
    pub fn observe(&mut self, sample: ListenSample, elapsed: Duration) -> Option<ListenReport> {
        let report = if self.previous.is_some_and(|previous| previous.key != sample.key) {
            self.finish()
        } else {
            None
        };
        if let Some(previous) = self.previous
            && previous.key.is_some()
            && previous.playing
            && previous.decoder_generation == sample.decoder_generation
            && elapsed <= Duration::from_secs(2)
            && previous.speed.is_finite()
            && (0.25..=2.0).contains(&previous.speed)
        {
            let progress =
                Duration::from_millis(sample.position_ms.saturating_sub(previous.position_ms));
            // The existing monitor samples every 500 ms. Allow its quantization, but discard
            // jumps (seek), backwards movement and time spent suspended or stalled.
            let plausible = elapsed.mul_f64(previous.speed) + Duration::from_millis(750);
            if progress <= plausible {
                self.listened +=
                    progress.div_f64(previous.speed).min(elapsed + Duration::from_millis(500));
            }
        }
        self.previous = Some(sample);
        report
    }

    pub fn finish(&mut self) -> Option<ListenReport> {
        let previous = self.previous.take();
        let seconds = std::mem::take(&mut self.listened).as_secs();
        let key = previous?.key?;
        (seconds > 0).then_some(ListenReport { key, seconds })
    }
}

#[cfg(test)]
#[path = "../tests/netease_listening_tests.rs"]
mod tests;
