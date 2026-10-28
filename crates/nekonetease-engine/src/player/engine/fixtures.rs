//! Fixtures for the transport suites, built out of this crate's own published types.
//!
//! `#[doc(hidden)] pub` rather than `#[cfg(test)]`, and not by preference: three crates read
//! these — the engine's own suites, `library`'s playback tests and the now-playing ladder under
//! `ui/` — and a `cfg(test)` item cannot cross a crate boundary. `DbPool::test_pool` is the same
//! shape for the same reason. The cheaper fixtures each tier can spell for itself stayed
//! `cfg(test)` beside their tests; these two are shared across crate boundaries.

use std::sync::Arc;

use crate::player::engine::state::PlayerViewModelLight;
use crate::player::engine::types::RepeatMode;
use nekonetease_core::entities::track::TrackSummary;

/// A track for the suites that need one on the deck rather than a decodable file, with only the
/// tagged fields varying. `TrackSummary` has seventeen columns and the suites reading this one
/// care about four.
pub fn test_track(title: &str, artist: Option<&str>, album: Option<&str>) -> Arc<TrackSummary> {
    Arc::new(TrackSummary {
        id: 7,
        file_path: String::new(),
        file_name: String::new(),
        title: title.to_owned(),
        artist: artist.map(str::to_owned),
        album: album.map(str::to_owned),
        duration_ms: 200_000,
        artwork_path: Some("cover.jpg".to_owned()),
        track_number: None,
        disc_number: None,
        last_position: 0,
        is_favorite: false,
        rating: 0,
        replaygain_track_gain: None,
        replaygain_track_peak: None,
        replaygain_album_gain: None,
        replaygain_album_peak: None,
        stream: None,
        mv_id: None,
    })
}

/// A published view model with the track and duration fields most consumers read. Everything else
/// is left at a resting value: a test that wants one sets it on the returned struct, so a field
/// added to `PlayerViewModelLight` lands here rather than in every suite that builds one.
pub fn test_view_model(
    current_track: Option<Arc<TrackSummary>>,
    duration_ms: u64,
) -> PlayerViewModelLight {
    PlayerViewModelLight {
        status: "playing",
        current_track,
        position_ms: 0,
        duration_ms,
        progress_percent: 0.0,
        volume: 100,
        is_muted: false,
        playback_speed: 1.0,
        gapless_enabled: false,
        sleep_at_track_end: false,
        has_next: false,
        has_previous: false,
        shuffle_enabled: false,
        repeat_mode: RepeatMode::Off,
    }
}
