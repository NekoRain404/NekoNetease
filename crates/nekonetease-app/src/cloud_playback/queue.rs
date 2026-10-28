use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use nekonetease_core::entities::cloud::Song;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::{AppError, AppResult};
use nekonetease_engine::player::engine::queue::QueueState;
use nekonetease_engine::player::engine::state::{lock_state, with_state_emit};
use nekonetease_engine::player::engine::types::{PlaybackStatus, RepeatMode};

use super::playback::ReservedPlayback;
use super::source::track_summary;
use super::{CloudPlayback, CloudPlaybackResult};

/// A row activation keeps the transport preference; an explicit order choice replaces it.
#[derive(Clone, Copy)]
pub enum CloudQueueShuffle {
    Preserve,
    Ordered,
    Shuffle,
}

impl CloudQueueShuffle {
    pub(super) fn enabled(self, current: bool) -> bool {
        match self {
            Self::Preserve => current,
            Self::Ordered => false,
            Self::Shuffle => true,
        }
    }
}

#[derive(Clone)]
pub(super) struct CloudQueuePlan {
    pub(super) tracks: Vec<Arc<TrackSummary>>,
    pub(super) start_index: usize,
    pub(super) shuffle: CloudQueueShuffle,
}

impl CloudQueuePlan {
    pub(super) fn apply(self, queue: &mut QueueState, rng: &mut impl rand::Rng) {
        let shuffle = self.shuffle.enabled(queue.shuffle_enabled);
        queue.clear();
        queue.add_tracks(self.tracks);
        let _ = queue.skip_to_index(self.start_index);
        if shuffle {
            queue.shuffle_play_order_in_place(rng, true);
        } else {
            queue.unshuffle();
        }
    }
}

pub(super) enum QueueUpdate {
    Clear,
    Preserve,
    Replace(CloudQueuePlan),
}

impl CloudPlayback {
    /// Queue cloud songs without querying the local library or narrowing their IDs.
    pub async fn enqueue(&self, song_ids: &[i64], play_next: bool) -> AppResult<()> {
        self.enqueue_in_generation(song_ids, play_next, None).await
    }

    pub fn queue_generation(&self) -> u64 {
        self.queue_generation.load(Ordering::Acquire)
    }

    /// Reports whether the stopped player is waiting for a queued cloud track to load.
    pub fn is_queue_track_loading(&self) -> bool {
        let generation = self.pending_queue_playback_generation.load(Ordering::Acquire);
        generation != 0 && self.engine.is_playback_request_current(generation)
    }

    /// A delayed roaming refill must not append to a playlist selected in the meantime.
    pub async fn enqueue_roaming(&self, song_ids: &[i64], generation: u64) -> AppResult<()> {
        self.enqueue_in_generation(song_ids, false, Some(generation)).await
    }

    async fn enqueue_in_generation(
        &self,
        song_ids: &[i64],
        play_next: bool,
        queue_generation: Option<u64>,
    ) -> AppResult<()> {
        if song_ids.is_empty() {
            return Ok(());
        }
        let generation = self.cloud.status().generation;
        let cancellation = self.cloud.account_tasks_token();
        let songs = self.cloud.songs(song_ids).await?;
        if cancellation.is_cancelled() || self.cloud.status().generation != generation {
            return Err(AppError::network_msg("网易云排队请求已取消"));
        }
        let mut by_id: HashMap<i64, Song> = songs.into_iter().map(|song| (song.id, song)).collect();
        let tracks = song_ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .map(|song| {
                let liked = self.cloud.cached_is_liked(song.id).unwrap_or(false);
                Arc::new(track_summary(song, None, liked))
            })
            .collect();
        with_state_emit(&self.player_state, &self.sinks, |state| {
            if queue_generation.is_some_and(|expected| self.queue_generation() != expected) {
                return;
            }
            if queue_generation.is_some()
                && ((state.status == PlaybackStatus::Stopped && !self.is_queue_track_loading())
                    || state.source.as_ref().is_none_or(|track| !track.file_path.is_empty()))
            {
                return;
            }
            queue_cloud_tracks(&mut state.queue, tracks, play_next);
        });
        Ok(())
    }

    /// Build the cloud queue in the supplied list order and start the selected song. Existing
    /// repeat and shuffle preferences are retained; `shuffle` forces shuffle on for Shuffle All.
    pub async fn play_queue(
        &self,
        song_ids: &[i64],
        selected_song_id: i64,
        shuffle: bool,
        volume: f64,
        speed: f64,
    ) -> AppResult<CloudPlaybackResult> {
        if song_ids.is_empty() {
            return Err(AppError::Queue("云端播放列表没有歌曲".to_owned()));
        }
        self.queue_generation.fetch_add(1, Ordering::AcqRel);
        let generation = self.engine.reserve_playback_request();
        let songs = self.cloud.songs(song_ids).await?;
        if !self.engine.is_playback_request_current(generation) {
            return Ok(CloudPlaybackResult {
                started: false,
                trial: false,
                restriction: None,
                served_level: None,
            });
        }
        // Warm liked membership once so cloud queue/player hearts also reflect the account when
        // the Favorites page has not been visited in this process. A lookup failure must not make
        // an otherwise playable playlist fail.
        if self.cloud.cached_is_liked(selected_song_id).is_none() {
            let _ = self.cloud.liked_ids().await;
        }
        let mut by_id: HashMap<i64, Song> = songs.into_iter().map(|song| (song.id, song)).collect();
        let tracks: Vec<Arc<TrackSummary>> = song_ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .map(|song| {
                let liked = self.cloud.cached_is_liked(song.id).unwrap_or(false);
                Arc::new(track_summary(song, None, liked))
            })
            .collect();
        let shuffle =
            if shuffle { CloudQueueShuffle::Shuffle } else { CloudQueueShuffle::Preserve };
        self.start_cloud_queue(generation, tracks, selected_song_id, shuffle, volume, speed).await
    }

    /// [`Self::play_queue`] for a caller that already holds the track list. The playlist detail
    /// does — its rows are the same songs, title through artwork — so a click there costs a stream
    /// ticket instead of a whole `song/detail` round trip for a list the screen is showing.
    /// `shuffle` distinguishes a row activation from an explicit order selection.
    pub async fn play_queue_tracks(
        &self,
        tracks: Vec<TrackSummary>,
        selected_song_id: i64,
        shuffle: CloudQueueShuffle,
        volume: f64,
        speed: f64,
    ) -> AppResult<CloudPlaybackResult> {
        if tracks.is_empty() {
            return Err(AppError::Queue("云端播放列表没有歌曲".to_owned()));
        }
        self.queue_generation.fetch_add(1, Ordering::AcqRel);
        let generation = self.engine.reserve_playback_request();
        let tracks: Vec<Arc<TrackSummary>> = tracks.into_iter().map(Arc::new).collect();
        self.start_cloud_queue(generation, tracks, selected_song_id, shuffle, volume, speed).await
    }

    /// Seat the queue and start `selected_song_id`, walking the candidates a repeat or shuffle
    /// preference allows until one of them actually starts.
    async fn start_cloud_queue(
        &self,
        generation: u64,
        tracks: Vec<Arc<TrackSummary>>,
        selected_song_id: i64,
        shuffle: CloudQueueShuffle,
        volume: f64,
        speed: f64,
    ) -> AppResult<CloudPlaybackResult> {
        let start_index = tracks
            .iter()
            .position(|track| track.id == selected_song_id)
            .ok_or_else(|| AppError::Queue("所选歌曲不在当前播放列表中".to_owned()))?;
        let (repeat_mode, shuffle_enabled) = {
            let state = lock_state(&self.player_state);
            (state.queue.repeat_mode, state.queue.shuffle_enabled)
        };
        let candidates = playback_candidates(
            tracks.len(),
            start_index,
            repeat_mode,
            shuffle.enabled(shuffle_enabled),
        );
        let quality = self.quality();
        let mut last_error = None;
        for candidate in candidates {
            let song_id = tracks[candidate].id;
            match self
                .play_reserved(ReservedPlayback {
                    song_id,
                    quality,
                    volume,
                    speed,
                    start_position: None,
                    generation,
                    keep_paused: false,
                    queue_update: QueueUpdate::Replace(CloudQueuePlan {
                        tracks: tracks.clone(),
                        start_index: candidate,
                        shuffle,
                    }),
                    // A fallback candidate the user did not pick; its refusal is logged, not
                    // announced, or one region-locked playlist would stack a toast per track.
                    report_refusal: candidate == start_index,
                })
                .await
            {
                Ok(result) if result.started => return Ok(result),
                Ok(result) => return Ok(result), // A newer selection superseded this request.
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error
            .unwrap_or_else(|| AppError::Queue("云端播放列表没有可播放的歌曲".to_owned())))
    }
}

/// Candidate order for a newly selected cloud queue item. Repeat-off respects the tail the user
/// selected from; looping modes wrap once. Shuffle anchors the requested item and randomizes the
/// remaining candidates before any network resolution begins.
pub(super) fn playback_candidates(
    track_count: usize,
    start_index: usize,
    repeat_mode: RepeatMode,
    shuffle: bool,
) -> Vec<usize> {
    if start_index >= track_count {
        return Vec::new();
    }
    let mut candidates = vec![start_index];
    let mut remaining: Vec<usize> = if shuffle {
        (0..track_count).filter(|&index| index != start_index).collect()
    } else {
        let mut after: Vec<_> = (start_index + 1..track_count).collect();
        if repeat_mode.wraps() {
            after.extend(0..start_index);
        }
        after
    };
    if shuffle {
        use rand::seq::SliceRandom;
        remaining.shuffle(&mut rand::rng());
    }
    candidates.extend(remaining);
    candidates
}

pub(super) fn queue_cloud_tracks(
    queue: &mut nekonetease_engine::player::engine::queue::QueueState,
    tracks: Vec<Arc<TrackSummary>>,
    play_next: bool,
) {
    if play_next {
        for track in tracks.into_iter().rev() {
            queue.insert_next(track);
        }
    } else {
        queue.add_tracks(tracks);
    }
}
