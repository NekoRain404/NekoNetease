//! Both directions of `queue.json`: the snapshot a save writes, and the restore that seats it back
//! at boot.

use std::sync::Arc;

use super::PlayerState;
use crate::player::engine::queue::{current_index_from_i32, current_index_to_i32};
use crate::player::engine::types::{PersistableQueue, PersistedPlayback, PlaybackStatus};
use nekonetease_core::entities::track::TrackSummary;

impl PlayerState {
    /// The snapshot `queue.json` holds, for the two save sites to write.
    ///
    pub fn to_persisted(&self) -> PersistedPlayback {
        // Cloud IDs are not local database row IDs. Persisting them into the local queue would
        // restore unrelated files when a local track happens to share the same numeric ID.
        let has_external_track = self.queue.tracks.iter().any(|track| track.file_path.is_empty());
        let queue = if has_external_track {
            PersistableQueue { track_ids: Vec::new(), current_index: -1 }
        } else {
            self.queue.to_persistable()
        };
        let mut playlist_tracks: Vec<TrackSummary> =
            self.queue.tracks_in_play_order().iter().map(|track| (**track).clone()).collect();
        let current_index = current_index_to_i32(self.queue.current_index);
        let current_position_ms = self.source.as_ref().map(|_| self.position_ms);
        if let (Some(index), Some(position_ms)) =
            (usize::try_from(current_index).ok(), current_position_ms)
            && let Some(track) = playlist_tracks.get_mut(index)
        {
            track.last_position = i64::try_from(position_ms).unwrap_or(i64::MAX);
        }
        PersistedPlayback {
            queue,
            playlist_tracks,
            playlist_current_index: current_index,
            current_position_ms,
        }
    }
}

/// Restore queue from persisted data. Called at startup via
/// `library::queue::restore_persisted_playback`.
///
/// `shuffle_enabled` and `repeat_mode` are user preferences and live in
/// `settings.json`, not `queue.json` — the caller is responsible for
/// hydrating them. Note that `original_order` is not persisted, so a
/// caller restoring a non-empty queue should also force shuffle off.
pub fn restore_queue(
    state: &mut PlayerState,
    tracks: Vec<Arc<TrackSummary>>,
    persistable: &PersistableQueue,
) {
    let len = tracks.len();
    state.queue.tracks = tracks;
    state.queue.play_order = (0..len).collect();
    state.queue.original_order = (0..len).collect();
    state.queue.current_index = current_index_from_i32(persistable.current_index);

    if let Some(track) = state.queue.get_current().cloned() {
        state.duration_ms = u64::try_from(track.duration_ms.max(0)).unwrap_or(0);
        state.position_ms = u64::try_from(track.last_position.max(0)).unwrap_or(0);
        state.source = Some(track);
        state.status = PlaybackStatus::Paused;
    }
}
