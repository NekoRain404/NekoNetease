use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use nekonetease_core::entities::cloud::Quality;
use nekonetease_core::entities::track::{StreamInfo, TrackSummary};
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_engine::player::engine::state::{lock_state, with_state_emit};
use nekonetease_engine::player::engine::types::PlaybackStatus;
use nekonetease_playback::player::playback::replaygain::TrackReplayGain;

use super::queue::QueueUpdate;
use super::refusal::{ResolveFailure, kind_for, refine_refusal, refusal_error};
use super::source::{ActiveStreamTicket, download_cover, track_summary};
use super::{CloudPlayback, CloudPlaybackResult};
use crate::library::cloud_cache;

pub(super) struct ReservedPlayback {
    pub(super) song_id: i64,
    pub(super) quality: Quality,
    pub(super) volume: f64,
    pub(super) speed: f64,
    pub(super) start_position: Option<Duration>,
    pub(super) generation: u64,
    pub(super) keep_paused: bool,
    pub(super) queue_update: QueueUpdate,
    /// Whether this request's refusal is announced. A queue walk retries every entry at once, and
    /// only the entry the user asked for should raise the toast.
    pub(super) report_refusal: bool,
}

struct ResolvedCloudTrack {
    active_ticket: ActiveStreamTicket,
    summary: Arc<TrackSummary>,
    cover_url: Option<String>,
    used_cached_ticket: bool,
}

/// The level the server reported, or `None` when it reported none — the legacy fallback route
/// answers without one.
pub(super) fn served_quality(level: &str) -> Option<Quality> {
    (!level.is_empty()).then(|| Quality::from_level(level))
}

/// The served tier when it is below the one requested. Equal or better tiers play as announced.
pub(super) fn downgrade(requested: Quality, served: Option<Quality>) -> Option<Quality> {
    served.filter(|served| served.bitrate() < requested.bitrate())
}

impl CloudPlayback {
    /// Apply a quality choice immediately and prepare the current cloud track for a source swap.
    /// The current deck is paused synchronously before the returned future starts resolving the
    /// replacement stream, so the old quality cannot continue playing during the request.
    pub fn change_quality(
        &self,
        quality: Quality,
    ) -> impl Future<Output = AppResult<CloudPlaybackResult>> + Send + 'static {
        let request = if self.quality() == quality {
            None
        } else {
            self.set_quality(quality);
            self.pause_for_quality_change()
        };
        let playback = self.clone();
        async move {
            match request {
                Some(request) => playback.play_reserved(request).await,
                None => Ok(CloudPlaybackResult {
                    started: false,
                    trial: false,
                    restriction: None,
                    served_level: None,
                }),
            }
        }
    }

    /// Pause the active cloud source and capture its exact heard position before any await. A
    /// paused player remains paused after the replacement loads; a playing player resumes on it.
    fn pause_for_quality_change(&self) -> Option<ReservedPlayback> {
        let _exec = self.player_state.lock_exec();
        let state = lock_state(&self.player_state);
        let track = state.current_track()?;
        if !track.file_path.is_empty()
            || !matches!(state.status, PlaybackStatus::Playing | PlaybackStatus::Paused)
        {
            return None;
        }

        let song_id = track.id;
        let volume = state.effective_volume();
        let speed = state.playback_speed;
        let was_paused = state.status == PlaybackStatus::Paused;
        let position_ms = if self.engine.holds_source() {
            self.engine.query_heard_position()
        } else {
            state.position_ms
        }
        .min(state.duration_ms.saturating_sub(500));
        drop(state);

        self.engine.pause();
        let generation = self.engine.reserve_playback_request();
        with_state_emit(&self.player_state, &self.sinks, |state| {
            state.status = PlaybackStatus::Paused;
            state.position_ms = position_ms;
        });

        Some(ReservedPlayback {
            song_id,
            quality: self.quality(),
            volume,
            speed,
            start_position: Some(Duration::from_millis(position_ms)),
            generation,
            keep_paused: was_paused,
            queue_update: QueueUpdate::Preserve,
            report_refusal: true,
        })
    }

    /// Resolve a short-lived ticket, open and seek the media source off the async executor, then
    /// hand the buffered decoder to the engine if the request is still current.
    pub async fn play(
        &self,
        song_id: i64,
        quality: Quality,
        volume: f64,
        speed: f64,
        start_position: Option<Duration>,
    ) -> AppResult<CloudPlaybackResult> {
        self.queue_generation.fetch_add(1, Ordering::AcqRel);
        let generation = self.engine.reserve_playback_request();
        self.play_reserved(ReservedPlayback {
            song_id,
            quality,
            volume,
            speed,
            start_position,
            generation,
            keep_paused: false,
            queue_update: QueueUpdate::Clear,
            report_refusal: true,
        })
        .await
    }

    /// Re-open a cloud stream at a new position. Cloud sources cannot use the local-file seek
    /// action, so seeking is a new range-backed decoder generation. The current-track check and
    /// generation reservation happen before the first await, preventing a delayed slider task
    /// from taking over a song selected after the user dragged it.
    pub async fn seek(&self, song_id: i64, position: Duration) -> AppResult<CloudPlaybackResult> {
        let (generation, volume, speed, was_paused, position) = {
            let _exec = self.player_state.lock_exec();
            let state = lock_state(&self.player_state);
            let Some(track) = state.source.as_ref() else {
                return Ok(CloudPlaybackResult {
                    started: false,
                    trial: false,
                    restriction: None,
                    served_level: None,
                });
            };
            if track.id != song_id || !track.file_path.is_empty() {
                return Ok(CloudPlaybackResult {
                    started: false,
                    trial: false,
                    restriction: None,
                    served_level: None,
                });
            }
            if !matches!(state.status, PlaybackStatus::Playing | PlaybackStatus::Paused) {
                return Ok(CloudPlaybackResult {
                    started: false,
                    trial: false,
                    restriction: None,
                    served_level: None,
                });
            }
            let volume = state.effective_volume();
            let speed = state.playback_speed;
            let was_paused = state.status == PlaybackStatus::Paused;
            let requested_ms = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
            let position =
                Duration::from_millis(requested_ms.min(state.duration_ms.saturating_sub(500)));
            drop(state);
            let generation = self.engine.reserve_playback_request();
            (generation, volume, speed, was_paused, position)
        };
        self.play_reserved(ReservedPlayback {
            song_id,
            quality: self.quality(),
            volume,
            speed,
            start_position: Some(position),
            generation,
            keep_paused: was_paused,
            queue_update: QueueUpdate::Preserve,
            report_refusal: true,
        })
        .await
    }

    pub(super) async fn play_reserved(
        &self,
        request: ReservedPlayback,
    ) -> AppResult<CloudPlaybackResult> {
        self.play_reserved_with_ticket(request, None).await
    }

    async fn resolve_playback_track(
        &self,
        song_id: i64,
        quality: Quality,
        cached_ticket: Option<ActiveStreamTicket>,
    ) -> Result<ResolvedCloudTrack, ResolveFailure> {
        let cached_ticket = cached_ticket.filter(|ticket| ticket.is_valid_for(song_id, quality));
        let cached_summary = cached_ticket.as_ref().and_then(|_| {
            lock_state(&self.player_state)
                .source
                .as_ref()
                .filter(|current| current.id == song_id && current.file_path.is_empty())
                .cloned()
        });
        if let (Some(active_ticket), Some(summary)) = (cached_ticket, cached_summary) {
            return Ok(ResolvedCloudTrack {
                active_ticket,
                summary,
                cover_url: None,
                used_cached_ticket: true,
            });
        }

        let song_ids = [song_id];
        // Joined rather than short-circuited: a refusal needs the song detail to say why, and both
        // requests are already on the wire once the ticket one starts.
        let (ticket, songs) =
            tokio::join!(self.cloud.stream_ticket(song_id, quality), self.cloud.songs(&song_ids));
        let ticket = ticket?;
        if let Some(restriction) = ticket.denial() {
            let song =
                songs.as_ref().ok().and_then(|songs| songs.iter().find(|song| song.id == song_id));
            return Err(ResolveFailure::Refused {
                restriction: refine_refusal(restriction, song),
                detail: song.map(|song| song.name.clone()).unwrap_or_default(),
            });
        }
        let issued_at = Instant::now();
        let expires_at = issued_at
            .checked_add(Duration::from_secs(ticket.expires_after_seconds))
            .unwrap_or(issued_at);
        let active_ticket = ActiveStreamTicket { ticket, quality, expires_at };
        let songs = songs?;
        let cached_artwork = lock_state(&self.player_state)
            .source
            .as_ref()
            .filter(|current| current.id == song_id && current.file_path.is_empty())
            .and_then(|current| current.artwork_path.clone());
        let song = songs
            .into_iter()
            .find(|song| song.id == song_id)
            .ok_or_else(|| AppError::network_msg("网易云未返回歌曲信息"))?;
        let cover_url = song.album.cover_url.clone();
        let is_favorite = self.cloud.cached_is_liked(song_id).unwrap_or(false);
        Ok(ResolvedCloudTrack {
            active_ticket,
            summary: Arc::new(track_summary(song, cached_artwork, is_favorite)),
            cover_url,
            used_cached_ticket: false,
        })
    }

    pub(super) async fn play_reserved_with_ticket(
        &self,
        request: ReservedPlayback,
        cached_ticket: Option<ActiveStreamTicket>,
    ) -> AppResult<CloudPlaybackResult> {
        let ReservedPlayback {
            song_id,
            quality,
            volume,
            speed,
            start_position,
            generation,
            keep_paused,
            queue_update,
            report_refusal,
        } = request;
        let cancellation = self.cloud.account_tasks_token();
        let resolved = match self.resolve_playback_track(song_id, quality, cached_ticket).await {
            Ok(resolved) => resolved,
            Err(ResolveFailure::Refused { restriction, detail }) => {
                if report_refusal {
                    toast::notify(kind_for(restriction), detail);
                }
                return Err(refusal_error(restriction));
            }
            Err(ResolveFailure::Failed(error)) => return Err(error),
        };
        let ResolvedCloudTrack { active_ticket, summary, cover_url, used_cached_ticket } = resolved;
        if cancellation.is_cancelled() {
            return Err(AppError::network_msg("网易云播放请求已取消"));
        }
        let should_fetch_cover = summary.artwork_path.is_none() && cover_url.is_some();

        let track_title = summary.title.clone();
        let start = start_position.unwrap_or(Duration::ZERO);
        let (active_ticket, source, from_cache) = self
            .open_source_with_expired_ticket_retry(
                song_id,
                quality,
                start,
                &cancellation,
                active_ticket,
                used_cached_ticket,
            )
            .await?;
        let served_level = served_quality(&active_ticket.ticket.level);
        let trial = active_ticket.ticket.trial;
        let restriction = active_ticket.ticket.restriction;
        if cancellation.is_cancelled() {
            return Err(AppError::network_msg("网易云播放请求已取消"));
        }

        // Serialize the deck handoff and the player-state publication together with local
        // transport actions. Otherwise the audio can start while the player bar still has no
        // current track, or an older network request can publish after a newer local selection.
        let _exec = self.player_state.lock_exec();
        let started = self.engine.play_prepared_source(
            generation,
            source,
            volume,
            speed,
            start_position,
            TrackReplayGain::default(),
        );
        if started {
            if keep_paused {
                self.engine.pause();
            }
            // What the endpoint served, carried on the summary so the Now Playing chip row can
            // state the stream's own format. The ticket has to be read before it is stored.
            let stream = served_level.map(|level| StreamInfo {
                format: active_ticket.ticket.format.clone(),
                level,
                size_bytes: active_ticket.ticket.size,
            });
            // The ticket moves into the active slot on the next line; a stream that reached the
            // deck on a miss is worth keeping, a trial or a hit is not.
            self.cache_missed_stream(&active_ticket, from_cache);
            *self.active_stream_ticket.write() = Some(active_ticket);
            let duration_ms = u64::try_from(summary.duration_ms.max(0)).unwrap_or(0);
            let position_ms = u64::try_from(start.as_millis()).unwrap_or(u64::MAX);
            let summary = match stream {
                Some(stream) => {
                    Arc::new(TrackSummary { stream: Some(stream), ..(*summary).clone() })
                }
                None => summary,
            };
            with_state_emit(&self.player_state, &self.sinks, |state| {
                self.decoder_generation.fetch_add(1, Ordering::AcqRel);
                if start_position.is_none() {
                    self.listening_generation.fetch_add(1, Ordering::AcqRel);
                }
                // A cloud queue replaces the prior queue only once its selected stream is ready,
                // so an unavailable song cannot destroy a working queue.
                match queue_update {
                    QueueUpdate::Clear => state.queue.clear(),
                    QueueUpdate::Preserve => {}
                    QueueUpdate::Replace(plan) => {
                        plan.apply(&mut state.queue, &mut rand::rng());
                    }
                }
                state.source = Some(summary);
                state.status =
                    if keep_paused { PlaybackStatus::Paused } else { PlaybackStatus::Playing };
                state.duration_ms = duration_ms;
                state.position_ms = position_ms.min(duration_ms.saturating_sub(500));
            });
        }
        if started
            && should_fetch_cover
            && let Some(url) = cover_url
        {
            self.cache_cover_after_start(song_id, url);
        }
        if started && let Some(restriction) = restriction {
            toast::notify(kind_for(restriction), track_title);
        }
        if started
            && start_position.is_none()
            && restriction.is_none()
            && let Some(served) = downgrade(quality, served_level)
        {
            toast::notify(
                ToastKind::QualityDowngraded,
                format!("{} kbps", served.bitrate() / 1000),
            );
        }
        Ok(CloudPlaybackResult {
            started,
            trial,
            restriction,
            served_level: if started { served_level } else { None },
        })
    }

    /// Cover traffic is detached from the audio handoff, so a slow image CDN cannot delay the
    /// first PCM samples. The updated summary is emitted after caching and decoded by the normal
    /// Now Playing artwork pipeline.
    fn cache_cover_after_start(&self, song_id: i64, url: String) {
        let http = self.http.clone();
        let artwork_dir = self.artwork_dir.clone();
        let player_state = self.player_state.clone();
        let sinks = self.sinks.clone();
        tokio::spawn(async move {
            let Some(path) = download_cover(&http, &url, &artwork_dir).await else { return };
            let _exec = player_state.lock_exec();
            with_state_emit(&player_state, &sinks, |state| {
                let Some(current) = state.source.as_ref() else { return };
                if current.id != song_id
                    || !current.file_path.is_empty()
                    || current.artwork_path.is_some()
                {
                    return;
                }
                let mut updated = (**current).clone();
                updated.artwork_path = Some(path);
                state.source = Some(Arc::new(updated));
            });
        });
    }

    /// Copies a missed stream into the offline cache once the deck has it, detached like the
    /// cover fetch so a second download never delays the deck. A hit or a trial has nothing to
    /// copy, and the cache declining an oversized body is not a playback failure.
    fn cache_missed_stream(&self, active_ticket: &ActiveStreamTicket, from_cache: bool) {
        if from_cache || active_ticket.ticket.trial {
            return;
        }
        let Some(data_dir) = self.artwork_dir.parent().map(Path::to_path_buf) else {
            return;
        };
        let quality = served_quality(&active_ticket.ticket.level).unwrap_or(active_ticket.quality);
        let ticket = active_ticket.ticket.clone();
        tokio::spawn(async move {
            match cloud_cache::cache_stream_at(&data_dir, &ticket, quality).await {
                Ok(None) => {}
                Ok(Some(path)) => log::debug!("Cloud stream cached at {}", path.display()),
                Err(error) => log::warn!("Cloud stream cache failed: {}", describe(&error)),
            }
        });
    }
}
