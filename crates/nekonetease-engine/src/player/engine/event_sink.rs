use std::sync::Arc;

use tokio::sync::watch;

use super::state::{PlayerViewModelLight, QueueViewModel};
use super::types::{PlaybackStatus, RepeatMode};
use nekonetease_core::entities::track::TrackSummary;

/// App-owned playback for sources the engine cannot open as local files, such as cloud streams.
/// The engine invokes this after the queue state has selected a track; implementations should
/// resolve and install the source asynchronously.
pub trait ExternalTrackPlayback: Send + Sync + 'static {
    fn play(&self, track: Arc<TrackSummary>);

    /// Start an external track from a saved position after its source has been released.
    /// Implementations that do not support seeking can fall back to a normal start.
    fn play_from(&self, track: Arc<TrackSummary>, _position_ms: u64) {
        self.play(track);
    }

    /// Resume an external track. Network backed implementations should refresh expiring sources.
    fn resume(&self, track: Arc<TrackSummary>) {
        self.play(track);
    }
}

/// Events that come *from* OS media controls (MPRIS, SMTC, `MediaPlayer`) into the player.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    SeekTo(u64),
    SetVolume(u32),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    SetSpeed(f64),
}

pub trait EventSink: Send + Sync + 'static {
    fn handle(&self, ev: PlayerEvent);
}

pub struct NoopEventSink;
impl EventSink for NoopEventSink {
    fn handle(&self, _: PlayerEvent) {}
}

/// Push direction of OS media controls — `with_state_emit` calls `sync` after
/// every state mutation so MPRIS / SMTC stay in lockstep with the player.
/// Implemented by `MediaControlsHandle`; `PlayerSinks` holds `None` on platforms
/// where no handle could be created.
pub trait MediaControlsSync: Send + Sync + 'static {
    fn sync(&self, vm: &PlayerViewModelLight, status: PlaybackStatus);

    /// The playing position, on every poll. Cheap by contract: a backend that pays per push
    /// throttles itself, since only it knows what a push costs.
    fn update_position(&self, _position_ms: u64) {}

    /// The position jumped rather than advanced. MPRIS clients extrapolate between reads and
    /// learn of a jump only through this.
    fn seeked(&self, _position_ms: u64) {}
}

/// Sinks consumed by `with_state_emit`. The two watch senders carry the
/// reactive view-model updates the Slint bridge subscribes to;
/// `media_controls` is the optional push-direction hook for OS media
/// controls. The watch payloads are `Option` because `watch::channel` needs
/// an initial value and there is no meaningful default at startup.
pub struct PlayerSinks {
    pub view_model: watch::Sender<Option<PlayerViewModelLight>>,
    pub queue: watch::Sender<Option<QueueViewModel>>,
    pub media_controls: Option<Arc<dyn MediaControlsSync>>,
    pub external_track_playback: std::sync::RwLock<Option<Arc<dyn ExternalTrackPlayback>>>,
}

impl PlayerSinks {
    /// Install the app layer's asynchronous handler for cloud-backed queue entries.
    pub fn set_external_track_playback(&self, playback: Arc<dyn ExternalTrackPlayback>) {
        if let Ok(mut slot) = self.external_track_playback.write() {
            *slot = Some(playback);
        }
    }

    pub(crate) fn external_track_playback(&self) -> Option<Arc<dyn ExternalTrackPlayback>> {
        self.external_track_playback.read().ok()?.clone()
    }
}
