use std::collections::VecDeque;
use std::path::Path;

use nekonetease_core::error::{AppError, describe};
use nekonetease_core::utils::play_counts::{PlayCountEvent, try_send};

use super::backend::PlayerBackend;
use super::event_sink::PlayerSinks;
use super::state::{
    PlayerAction, PlayerState, PlayerStateHandle, play_track_inner, stop_end_of_queue,
    with_state_emit,
};
use nekonetease_playback::player::playback::replaygain::TrackReplayGain;

/// Execute a list of `PlayerActions` against the audio backend and database.
/// Called after releasing the `PlayerState` lock.
///
/// `PlayMedia` is pre-flighted with `Path::exists()` and any decode failure
/// also auto-skips: the queue's `build_next_actions` is computed inline and
/// appended to the pending action set so a single stale double-click within
/// the watcher debounce window doesn't dead-end playback. The bad track
/// stays in the queue until the watcher catches up and `tasks::queue_prune`
/// removes it, which for a file outside the watched folders is never.
pub fn execute_actions<B: PlayerBackend>(
    actions: Vec<PlayerAction>,
    engine: &B,
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
) {
    let mut pending = Pending { actions: actions.into(), skipped: 0 };
    while let Some(action) = pending.actions.pop_front() {
        // Safe per action because nothing periodic reaches here: the position
        // tick decides in `evaluate_playing_tick` and the 30 s queue save writes
        // its file directly.
        log::debug!("player: {action}");
        match action {
            PlayerAction::PlayMedia { file_path, volume, speed, start_position_ms, replaygain } => {
                if file_path.is_empty() {
                    start_external_track(player_state, sinks, start_position_ms);
                } else {
                    start_or_skip(
                        &mut pending,
                        engine,
                        player_state,
                        sinks,
                        &file_path,
                        StartMode::Fresh,
                        || {
                            engine.play_media(
                                &file_path,
                                volume,
                                speed,
                                start_position_ms,
                                replaygain,
                            )
                        },
                    );
                }
            }
            PlayerAction::BeginCrossfade { file_path, replaygain, fade_ms, volume, speed } => {
                // `build_crossfade_actions` already advanced onto this track, so
                // the `advance_skip` a failure triggers correctly lands on the one
                // after it. In `RepeatMode::One` that also steps off the repeated
                // track — rare enough (the file vanished mid-play) to accept.
                if file_path.is_empty() {
                    start_external_track(player_state, sinks, None);
                } else {
                    start_or_skip(
                        &mut pending,
                        engine,
                        player_state,
                        sinks,
                        &file_path,
                        StartMode::Crossfade,
                        || engine.begin_crossfade(&file_path, replaygain, fade_ms, volume, speed),
                    );
                }
            }
            PlayerAction::Resume => resume_external_track(engine, player_state, sinks),
            PlayerAction::Pause { fade_ms } => engine.pause_with_fade(fade_ms),
            PlayerAction::Stop { fade_ms } => engine.stop_with_fade(fade_ms),
            PlayerAction::Seek { position_ms, file_path, replaygain } => {
                engine.seek(&file_path, position_ms, replaygain);
                if let Some(mc) = sinks.media_controls.as_ref() {
                    mc.seeked(position_ms);
                }
            }
            PlayerAction::SetVolume(v) => engine.set_volume(v),
            PlayerAction::SetSpeed(s) => engine.set_speed(s),
            PlayerAction::PreloadGapless(path) => {
                // This action only ever *clears* a stale preload (`path` is
                // `None`); the real gapless preload with baked ReplayGain happens
                // directly in `spawn_playback_monitor`. A default (unity) RG here
                // is harmless — the `None` path never builds an audio source.
                engine.preload_gapless(path.as_deref(), TrackReplayGain::default());
            }
            // Both no-op until `boot::tasks` installs the flusher, which it does before playback
            // can start. Nothing here writes the row itself: a `DbPool` on this side is what pins
            // the engine to sqlx.
            PlayerAction::UpdatePlayCount(track_id) => {
                try_send(PlayCountEvent::Play(track_id));
            }
            PlayerAction::UpdateSkipCount(track_id) => {
                try_send(PlayCountEvent::Skip(track_id));
            }
        }
    }
}

/// Start the queue's selected cloud track through the app-installed network player. The queue
/// state has already moved to this track, so the callback can load it without a second selection.
/// A missing app handler is treated as a playback failure rather than opening an empty path.
fn start_external_track(
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
    start_position_ms: Option<u64>,
) {
    let track = super::state::lock_state(player_state)
        .source
        .as_ref()
        .filter(|track| track.file_path.is_empty())
        .cloned();
    let Some(track) = track else {
        log::warn!("Skipping external playback action without a selected cloud track");
        return;
    };
    if let Some(playback) = sinks.external_track_playback() {
        if let Some(position_ms) = start_position_ms {
            playback.play_from(track, position_ms);
        } else {
            playback.play(track);
        }
    } else {
        log::warn!("No external playback handler is installed for cloud song {}", track.id);
    }
}

/// Cloud sources are short-lived, so resuming one reopens it through the app's network player.
/// Local sources continue using the native engine's in-place resume.
fn resume_external_track<B: PlayerBackend>(
    engine: &B,
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
) {
    let track = super::state::lock_state(player_state)
        .source
        .as_ref()
        .filter(|track| track.file_path.is_empty())
        .cloned();
    if let (Some(track), Some(playback)) = (track, sinks.external_track_playback()) {
        playback.resume(track);
    } else {
        engine.resume();
    }
}

/// Mutate state, publish the `ViewModel`, then execute the resulting side
/// effects — all while holding the per-`PlayerStateHandle` execution lock so
/// mutation order equals side-effect order across tokio workers.
///
/// This is the serialized replacement for the bare `with_state_emit(...)` +
/// `execute_actions(...)` pair. `with_state_emit` alone keeps each *mutation*
/// atomic, but the `execute_actions` that follows runs on whatever worker the
/// caller is on; two batches from different tasks (e.g. the playback monitor's
/// EOS-advance and a UI `Stop`) could otherwise interleave their side effects
/// and leave state and backend disagreeing (a rare TOCTOU). Holding `exec_lock`
/// across both halves closes that window.
///
/// The lock spans only synchronous work (no `.await`), so a blocking mutex is
/// correct. `enqueue_auto_skip`'s nested `with_state_emit` inside
/// `execute_actions` takes the *state* mutex, never `exec_lock`, so there is no
/// re-entrancy.
pub fn emit_and_execute<B, F>(
    engine: &B,
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
    f: F,
) where
    B: PlayerBackend,
    F: FnOnce(&mut PlayerState) -> Vec<PlayerAction>,
{
    let _exec = player_state.lock_exec();
    let actions = with_state_emit(player_state, sinks, f);
    execute_actions(actions, engine, player_state, sinks);
}

/// What is left to run of one [`execute_actions`] call.
struct Pending {
    actions: VecDeque<PlayerAction>,
    /// Tracks this call has skipped as unplayable. A repeating queue wraps on a skip, so without
    /// a bound a queue with nothing playable in it skips forever, holding the lock every
    /// transport control waits on.
    skipped: usize,
}

impl Pending {
    /// Run `actions` next, in their order, ahead of anything still queued from the batch.
    fn prepend(&mut self, actions: Vec<PlayerAction>) {
        for (i, a) in actions.into_iter().enumerate() {
            self.actions.insert(i, a);
        }
    }
}

/// How a track is being started — the only thing that differs between the
/// [`PlayerAction::PlayMedia`] and [`PlayerAction::BeginCrossfade`] arms of
/// [`execute_actions`].
#[derive(Copy, Clone)]
enum StartMode {
    /// Takes over the decks outright. A failure leaves them half-set, so stop
    /// before skipping on.
    Fresh,
    /// Overlaps the track still playing on the other deck. Deliberately does
    /// **not** stop on failure: the outgoing track is still audible, and the
    /// `play_media` that the auto-skip produces takes over from it cleanly either
    /// way — hard-cutting (which clears both decks) or, with `crossfade_manual`
    /// on, fading out of it. Stopping here would only insert a gap of silence
    /// ahead of that.
    Crossfade,
}

impl StartMode {
    fn stops_on_failure(self) -> bool {
        matches!(self, Self::Fresh)
    }

    /// The verb for the decode-failure log: "Failed to *play* …" / "Failed to
    /// *crossfade into* …".
    fn verb(self) -> &'static str {
        match self {
            Self::Fresh => "play",
            Self::Crossfade => "crossfade into",
        }
    }

    /// The transition the skip happened at, for the vanished-file log.
    fn at(self) -> &'static str {
        match self {
            Self::Fresh => "playback",
            Self::Crossfade => "crossfade",
        }
    }
}

/// Start a track on the backend, auto-skipping past it if it can't be played.
///
/// Shared by the two start actions. A file that has vanished is skipped
/// *silently* while the skip lands on something playable — the usual cause is a
/// stale double-click inside the watcher's debounce window — and toasts only
/// when nothing in the queue plays. A decode failure is louder: the music
/// silently stopping is otherwise invisible, so it always toasts.
fn start_or_skip<B: PlayerBackend>(
    pending: &mut Pending,
    engine: &B,
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
    file_path: &str,
    mode: StartMode,
    start: impl FnOnce() -> Result<(), AppError>,
) {
    if !Path::new(file_path).exists() {
        log::warn!("Skipping vanished file at {}: {file_path}", mode.at());
        if mode.stops_on_failure() {
            engine.stop();
        }
        if enqueue_auto_skip(pending, player_state, sinks) {
            toast_playback_failed(file_path);
        }
        return;
    }
    if let Err(e) = start() {
        log::error!("Failed to {} {file_path}: {}", mode.verb(), describe(&e));
        toast_playback_failed(file_path);
        if mode.stops_on_failure() {
            engine.stop();
        }
        enqueue_auto_skip(pending, player_state, sinks);
    }
}

fn toast_playback_failed(file_path: &str) {
    nekonetease_core::utils::toast::notify(
        nekonetease_core::utils::toast::ToastKind::PlaybackFailed,
        toast_track_name(file_path),
    );
}

/// The file name alone, for the failure toast — the full path is too long to
/// read in a toast. Falls back to the whole path when there is no file name.
fn toast_track_name(file_path: &str) -> String {
    Path::new(file_path)
        .file_name()
        .map_or_else(|| file_path.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Advance past a track that turned out to be unplayable (missing or
/// undecodable) and run whatever that produces next. Returns whether it gave up
/// instead.
///
/// It gives up once the batch has skipped a whole lap of the queue, since
/// nothing left in it will play. On an empty queue this emits `Stop` only.
/// Either way the `with_state_emit` call took the lock to do it, so the state
/// machine and `ViewModel` both reflect the end-of-queue state.
fn enqueue_auto_skip(
    pending: &mut Pending,
    player_state: &PlayerStateHandle,
    sinks: &PlayerSinks,
) -> bool {
    pending.skipped += 1;
    let skipped = pending.skipped;
    let mut gave_up = false;
    let actions = with_state_emit(player_state, sinks, |s| {
        if !s.queue.is_empty() && skipped >= s.queue.len() {
            gave_up = true;
            return stop_end_of_queue(s);
        }
        match s.queue.advance_after_failure().cloned() {
            Some(track) => play_track_inner(s, track, None),
            None => stop_end_of_queue(s),
        }
    });
    if gave_up {
        log::warn!("Stopping: none of the {skipped} track(s) tried could be played");
    }
    pending.prepend(actions);
    gave_up
}

#[cfg(test)]
#[path = "tests/actions_tests.rs"]
mod tests;
