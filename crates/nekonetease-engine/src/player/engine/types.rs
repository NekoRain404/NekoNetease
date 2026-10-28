use nekonetease_core::entities::track::TrackSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

impl PlaybackStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlaybackStatus::Stopped => "stopped",
            PlaybackStatus::Playing => "playing",
            PlaybackStatus::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
    }

    /// True when manual next/previous wraps and Up Next renders a wrapping slice.
    pub fn wraps(self) -> bool {
        matches!(self, Self::All | Self::One)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistableQueue {
    pub track_ids: Vec<i64>,
    pub current_index: i32,
}

/// What `queue.json` holds: the ordered last-played playlist, plus the legacy local-id queue
/// fields. Flattening preserves the existing on-disk shape for older versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedPlayback {
    #[serde(flatten)]
    pub queue: PersistableQueue,
    /// Full track rows in play order. This keeps cloud playlists and the visible queue available
    /// at startup even when the browse view refreshes from the network.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub playlist_tracks: Vec<TrackSummary>,
    /// Index into `playlist_tracks`; `-1` means the saved list has no current track.
    #[serde(default = "no_current_track", skip_serializing_if = "is_no_current_track")]
    pub playlist_current_index: i32,
    /// Exact position at the last playback snapshot, including cloud tracks that have no DB row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_position_ms: Option<u64>,
}

fn no_current_track() -> i32 {
    -1
}

// Serde's `skip_serializing_if` contract takes `&T`, even when the value is a small integer.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_no_current_track(index: &i32) -> bool {
    *index == -1
}

#[cfg(test)]
#[path = "tests/types_tests.rs"]
mod tests;
