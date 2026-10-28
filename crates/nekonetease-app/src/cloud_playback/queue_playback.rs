use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use nekonetease_core::entities::cloud::Quality;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::{AppError, AppResult};
use nekonetease_engine::player::engine::event_sink::ExternalTrackPlayback;
use nekonetease_engine::player::engine::state::{lock_state, with_state_emit};
use nekonetease_engine::player::engine::types::PlaybackStatus;

use super::CloudPlayback;
use super::playback::ReservedPlayback;
use super::queue::QueueUpdate;
use super::source::ActiveStreamTicket;

/// Routes normal queue transitions into the app's async cloud resolver. The queue has already
/// advanced before the engine calls this adapter, so it only needs the selected song identity.
struct CloudQueuePlaybackHandler {
    playback: CloudPlayback,
    runtime: tokio::runtime::Handle,
}

impl ExternalTrackPlayback for CloudQueuePlaybackHandler {
    fn play(&self, track: Arc<TrackSummary>) {
        let song_id = track.id;
        let (volume, speed, keep_paused) = {
            let state = lock_state(&self.playback.player_state);
            (state.effective_volume(), state.playback_speed, state.status == PlaybackStatus::Paused)
        };

        // Clear the drained/outgoing deck and mark the transition as waiting before any network
        // work begins. Otherwise the playback monitor can see the old deck at EOS and advance
        // the cloud queue again while the first ticket is still being resolved.
        self.playback.engine.stop();
        with_state_emit(&self.playback.player_state, &self.playback.sinks, |state| {
            state.status = PlaybackStatus::Stopped;
        });
        let generation = self.playback.engine.reserve_playback_request();
        self.playback.pending_queue_playback_generation.store(generation, Ordering::Release);
        let playback = self.playback.clone();
        self.runtime.spawn(async move {
            let result = playback
                .play_queued_track(song_id, volume, speed, generation, keep_paused, None)
                .await;
            let _ = playback.pending_queue_playback_generation.compare_exchange(
                generation,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            if let Err(error) = result {
                log::warn!(
                    "cloud queue track {song_id}: {}",
                    nekonetease_core::error::describe(&error)
                );
            }
        });
    }

    fn play_from(&self, track: Arc<TrackSummary>, position_ms: u64) {
        let song_id = track.id;
        let quality = self.playback.quality();
        let cached_ticket = self.playback.valid_stream_ticket(song_id, quality);
        let (volume, speed, position_ms) = {
            let state = lock_state(&self.playback.player_state);
            if state
                .source
                .as_ref()
                .is_none_or(|current| current.id != song_id || !current.file_path.is_empty())
            {
                return;
            }
            (
                state.effective_volume(),
                state.playback_speed,
                position_ms.min(state.duration_ms.saturating_sub(500)),
            )
        };

        // The engine released this source during a long pause. Keep the selected cloud track
        // seated but stopped while its decoder is prepared; a still-valid ticket can be reused.
        self.playback.engine.stop();
        let generation = self.playback.engine.reserve_playback_request();
        with_state_emit(&self.playback.player_state, &self.playback.sinks, |state| {
            state.status = PlaybackStatus::Stopped;
            state.position_ms = position_ms;
        });
        self.spawn_resume(song_id, quality, volume, speed, position_ms, generation, cached_ticket);
    }

    fn resume(&self, track: Arc<TrackSummary>) {
        let song_id = track.id;
        let quality = self.playback.quality();
        let (volume, speed, position_ms) = {
            let state = lock_state(&self.playback.player_state);
            if state
                .source
                .as_ref()
                .is_none_or(|current| current.id != song_id || !current.file_path.is_empty())
            {
                return;
            }
            let position_ms = if self.playback.engine.holds_source() {
                self.playback.engine.query_heard_position()
            } else {
                state.position_ms
            }
            .min(state.duration_ms.saturating_sub(500));
            (state.effective_volume(), state.playback_speed, position_ms)
        };

        let cached_ticket = self.playback.valid_stream_ticket(song_id, quality);
        if cached_ticket.is_some() && self.playback.engine.holds_source() {
            self.playback.engine.resume();
            return;
        }

        // The saved ticket has expired, does not match this source, or the decoder was released.
        // Keep any old decoder paused while a replacement is prepared.
        self.playback.engine.pause();
        let generation = self.playback.engine.reserve_playback_request();
        with_state_emit(&self.playback.player_state, &self.playback.sinks, |state| {
            state.status = PlaybackStatus::Paused;
            state.position_ms = position_ms;
        });
        self.spawn_resume(song_id, quality, volume, speed, position_ms, generation, cached_ticket);
    }
}

impl CloudQueuePlaybackHandler {
    fn spawn_resume(
        &self,
        song_id: i64,
        quality: Quality,
        volume: f64,
        speed: f64,
        position_ms: u64,
        generation: u64,
        cached_ticket: Option<ActiveStreamTicket>,
    ) {
        let playback = self.playback.clone();
        self.runtime.spawn(async move {
            let result = playback
                .play_reserved_with_ticket(
                    ReservedPlayback {
                        song_id,
                        quality,
                        volume,
                        speed,
                        start_position: Some(Duration::from_millis(position_ms)),
                        generation,
                        keep_paused: false,
                        queue_update: QueueUpdate::Preserve,
                        report_refusal: true,
                    },
                    cached_ticket,
                )
                .await;
            if let Err(error) = result {
                log::warn!(
                    "cloud track resume {song_id}: {}",
                    nekonetease_core::error::describe(&error)
                );
            }
        });
    }
}

impl CloudPlayback {
    /// True when a cloud track was restored from disk and has no decoded source on the engine.
    /// A normal pause with an active deck must continue through the ordinary transport path.
    pub fn restored_cloud_track_waiting(&self) -> bool {
        let paused_cloud_track = {
            let state = lock_state(&self.player_state);
            state.status == PlaybackStatus::Paused
                && state.source.as_ref().is_some_and(|track| track.file_path.is_empty())
        };
        paused_cloud_track && !self.engine.holds_source()
    }

    /// Resolve and play the restored cloud queue from its saved track and position. The queue
    /// itself was loaded synchronously from `queue.json`, so this only fetches a fresh stream.
    pub async fn resume_restored_queue(&self) -> AppResult<()> {
        let (song_id, position_ms, volume, speed) = {
            let state = lock_state(&self.player_state);
            let Some(track) = state.current_track().filter(|track| track.file_path.is_empty())
            else {
                return Ok(());
            };
            (track.id, state.position_ms, state.effective_volume(), state.playback_speed)
        };
        if !self.restored_cloud_track_waiting() {
            return Ok(());
        }

        self.engine.stop();
        with_state_emit(&self.player_state, &self.sinks, |state| {
            state.status = PlaybackStatus::Stopped;
        });
        let generation = self.engine.reserve_playback_request();
        self.pending_queue_playback_generation.store(generation, Ordering::Release);
        let result = self
            .play_queued_track(
                song_id,
                volume,
                speed,
                generation,
                false,
                Some(Duration::from_millis(position_ms)),
            )
            .await;
        let _ = self.pending_queue_playback_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        result
    }

    /// Install the app-side resolver used when normal queue behavior selects a cloud track.
    pub fn external_track_handler(
        &self,
        runtime: tokio::runtime::Handle,
    ) -> Arc<dyn ExternalTrackPlayback> {
        Arc::new(CloudQueuePlaybackHandler { playback: self.clone(), runtime })
    }

    /// Try each queue entry at most once if a selected stream is unavailable. This shares the
    /// engine's repeat and shuffle order and stops rather than wrapping forever through a broken
    /// playlist.
    pub(super) async fn play_queued_track(
        &self,
        first_song_id: i64,
        volume: f64,
        speed: f64,
        generation: u64,
        keep_paused: bool,
        start_position: Option<Duration>,
    ) -> AppResult<()> {
        let attempts = lock_state(&self.player_state).queue.len().max(1);
        let mut song_id = first_song_id;
        let mut last_error = None;
        for attempt in 0..attempts {
            match self
                .play_reserved(ReservedPlayback {
                    song_id,
                    quality: self.quality(),
                    volume,
                    speed,
                    start_position: if attempt == 0 { start_position } else { None },
                    generation,
                    keep_paused,
                    queue_update: QueueUpdate::Preserve,
                    // The walk retries every entry once; only the entry the user asked for says
                    // why it refused.
                    report_refusal: attempt == 0,
                })
                .await
            {
                Ok(result) if result.started => return Ok(()),
                Ok(_) => return Ok(()), // A newer selection superseded this request.
                Err(error) => last_error = Some(error),
            }
            if attempt + 1 == attempts {
                break;
            }
            let Some(next_id) = self.advance_cloud_queue_after_failure(song_id, generation) else {
                break;
            };
            song_id = next_id;
        }

        if !self.engine.is_playback_request_current(generation)
            || !self.current_cloud_track_is(song_id)
        {
            return Ok(());
        }
        self.engine.stop();
        with_state_emit(&self.player_state, &self.sinks, |state| {
            state.status = PlaybackStatus::Stopped;
            state.position_ms = 0;
        });
        Err(last_error.unwrap_or_else(|| AppError::network_msg("播放列表中没有可播放的歌曲")))
    }

    fn current_cloud_track_is(&self, song_id: i64) -> bool {
        lock_state(&self.player_state)
            .source
            .as_ref()
            .is_some_and(|track| track.id == song_id && track.file_path.is_empty())
    }

    /// Advance after a failure only while the failed song still owns the waiting player state.
    fn advance_cloud_queue_after_failure(
        &self,
        failed_song_id: i64,
        generation: u64,
    ) -> Option<i64> {
        if !self.engine.is_playback_request_current(generation) {
            return None;
        }
        let mut next_id = None;
        with_state_emit(&self.player_state, &self.sinks, |state| {
            if state
                .source
                .as_ref()
                .is_none_or(|track| track.id != failed_song_id || !track.file_path.is_empty())
                || state.status != PlaybackStatus::Stopped
            {
                return;
            }
            if let Some(track) = state.queue.advance_after_failure().cloned() {
                next_id = Some(track.id);
                state.duration_ms = u64::try_from(track.duration_ms.max(0)).unwrap_or(0);
                state.position_ms = 0;
                state.source = Some(track);
            }
        });
        next_id
    }
}
