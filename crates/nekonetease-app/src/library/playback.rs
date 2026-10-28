use std::sync::Arc;
use std::time::Duration;

use crate::state::PlaybackContext;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::AppError;
use nekonetease_engine::player::engine::backend::{OutputChoice, PlaybackEngine};
use nekonetease_engine::player::engine::state::{
    MAX_VOLUME, PlayerAction, PlayerState, lock_state, play_track_inner, with_state_emit,
};
use nekonetease_engine::player::engine::types::PlaybackStatus;
use nekonetease_playback::player::playback::output::{self, OutputDevice};
use nekonetease_store::database::queries;

/// Which slot of `summaries` playback should start on.
///
/// `start_index` names a row in `track_ids` — the list the caller was looking
/// at — but `get_track_summaries_by_ids` drops ids that no longer exist, so the
/// two index spaces diverge the moment one row is gone and every slot past the
/// gap shifts. Resolving through the id keeps the picked track picked. `None`
/// means there is no slot to start on: the caller picked no row at all, the
/// index it passed is past the end of its own list, or the row it picked didn't
/// survive. The caller falls back to the head and warns on the last two — they
/// are different faults, and the messages say which.
fn resolve_start_slot(
    track_ids: &[i64],
    summaries: &[Arc<TrackSummary>],
    start_index: Option<usize>,
) -> Option<usize> {
    let wanted = start_index.and_then(|i| track_ids.get(i).copied())?;
    summaries.iter().position(|t| t.id == wanted)
}

/// Replace the queue with `track_ids` and start at `start_index` (`None` =
/// head). Every way of starting playback from a browsing surface routes here —
/// activating a row passes the clicked slot, the header Shuffle pill passes a
/// random one — so the queue always ends up being the list the user was
/// looking at.
///
/// With shuffle already on, the rest of the list is shuffled behind the chosen
/// track rather than played in display order: a freshly seeded `play_order` is
/// the identity permutation, so without this the transport's shuffle button
/// would stay lit while playback walked the album straight through.
/// `original_order` is left as seeded, so turning shuffle back off restores
/// display order.
pub async fn player_play_tracks(
    ctx: &PlaybackContext,
    track_ids: Vec<i64>,
    start_index: Option<usize>,
) -> Result<(), AppError> {
    let summaries: Vec<Arc<TrackSummary>> =
        queries::track::get_track_summaries_by_ids(&ctx.db, &track_ids)
            .await?
            .into_iter()
            .map(Arc::new)
            .collect();

    if summaries.is_empty() {
        return Err(AppError::Queue("No valid tracks provided".to_owned()));
    }

    let start = resolve_start_slot(&track_ids, &summaries, start_index).unwrap_or_else(|| {
        match start_index {
            None => {}
            Some(i) if i >= track_ids.len() => log::warn!(
                "play_tracks: start_index {i} is past the {} ids handed in; starting at the head",
                track_ids.len()
            ),
            Some(_) => log::warn!(
                "play_tracks: the picked track didn't survive the fetch; starting at the head"
            ),
        }
        0
    });
    ctx.emit_and_execute(|s| {
        s.queue.clear();
        s.queue.add_tracks(summaries);
        s.queue.current_index = Some(start);
        if s.queue.shuffle_enabled {
            s.queue
                .shuffle_play_order_in_place(&mut rand::rng(), /* anchor_to_current */ true);
        }

        if let Some(track) = s.queue.get_current().cloned() {
            play_track_inner(s, track, None)
        } else {
            vec![]
        }
    });
    Ok(())
}

pub fn player_play(ctx: &PlaybackContext) -> Result<(), AppError> {
    ctx.emit_and_execute(|s| play_actions(s, &ctx.engine));
    Ok(())
}

/// Resume the paused track, or start it again where its deck no longer holds it, which is what a
/// pause long enough to give an exclusive device back leaves.
///
/// The deck is asked with the state lock held, the lock order's own direction, so the answer and
/// the actions it picks can't be split by anything that goes through the emit.
fn play_actions(s: &mut PlayerState, engine: &PlaybackEngine) -> Vec<PlayerAction> {
    if s.status == PlaybackStatus::Paused && !engine.holds_source() {
        return s.build_replay_actions();
    }
    s.build_play_actions()
}

/// Fade length for a transport pause or stop, or `0` when the setting is off.
///
/// The three transport commands route through here — `player_pause`,
/// `player_toggle_play_pause` and `player_stop` — so everything
/// that reaches them fades: the UI buttons, the keyboard shortcuts, the OS media keys,
/// the tray, and the sleep timer's expiry (which ends on
/// `player_pause`, and where a fade-out is exactly what you want).
///
/// What must *not* fade is what the machine does for its own reasons, and those
/// paths pass `0` directly rather than calling this: `stop_end_of_queue` (nothing
/// left to fade), and the `Pause` that next/previous append to restore a paused
/// deck (fading there would make the incoming track audible on arrival).
fn transport_fade_ms(ctx: &PlaybackContext) -> u64 {
    if ctx.engine.crossfade_settings().fade_on_pause {
        nekonetease_playback::player::playback::crossfade::PAUSE_FADE_MS
    } else {
        0
    }
}

pub fn player_pause(ctx: &PlaybackContext) -> Result<(), AppError> {
    let fade_ms = transport_fade_ms(ctx);
    ctx.emit_and_execute(move |s| s.build_pause_actions(fade_ms));
    Ok(())
}

/// Toggle play/pause based on current playback status. Branching happens inside
/// the state lock so two near-simultaneous toggles (e.g. UI + media-key) can't
/// race past each other.
pub fn player_toggle_play_pause(ctx: &PlaybackContext) -> Result<(), AppError> {
    let fade_ms = transport_fade_ms(ctx);
    let engine = &ctx.engine;
    ctx.emit_and_execute(move |s| match s.status {
        PlaybackStatus::Playing => s.build_pause_actions(fade_ms),
        PlaybackStatus::Paused | PlaybackStatus::Stopped => play_actions(s, engine),
    });
    Ok(())
}

/// User-initiated stop, from the play button's hold and the OS media controls alike: back to the
/// top with the track still seated, so `player_play` starts it over, as MPRIS specifies.
pub fn player_stop(ctx: &PlaybackContext) -> Result<(), AppError> {
    let fade_ms = transport_fade_ms(ctx);
    ctx.emit_and_execute(move |s| s.build_stop_actions(fade_ms));
    Ok(())
}

pub fn player_seek(ctx: &PlaybackContext, position_ms: u64) -> Result<(), AppError> {
    ctx.emit_and_execute(|s| s.build_seek_actions(position_ms));
    Ok(())
}

pub fn player_next(ctx: &PlaybackContext) -> Result<(), AppError> {
    ctx.emit_and_execute(
        nekonetease_engine::player::engine::state::PlayerState::build_next_actions,
    );
    Ok(())
}

pub fn player_previous(ctx: &PlaybackContext) -> Result<(), AppError> {
    ctx.emit_and_execute(
        nekonetease_engine::player::engine::state::PlayerState::build_previous_actions,
    );
    Ok(())
}

pub fn player_set_volume(ctx: &PlaybackContext, level: u32) -> Result<(), AppError> {
    // Fast no-op short-circuit: skip the whole with_state_emit / engine /
    // ViewModel-watch dance when nothing would actually change. Catches
    // duplicate slider emits at the boundary.
    {
        let s = lock_state(&ctx.player_state);
        if s.volume == level && !s.is_muted {
            return Ok(());
        }
    }
    ctx.emit_and_execute(|s| s.build_set_volume_actions(level));
    // Persistence is intentionally NOT triggered here. The slider fires
    // `set_volume` continuously during drag for live audio; settings.json
    // is flushed once via `commit_player_settings` on slider release.
    Ok(())
}

/// [`player_set_volume`] and its commit at once, for a move that arrives whole rather than as a
/// drag: the OS media panel's, or the device's own control's.
pub async fn player_set_volume_committed(
    ctx: &PlaybackContext,
    level: u32,
) -> Result<(), AppError> {
    player_set_volume(ctx, level)?;
    commit_player_settings(ctx).await
}

pub async fn player_toggle_mute(ctx: &PlaybackContext) -> Result<(), AppError> {
    ctx.emit_and_execute(
        nekonetease_engine::player::engine::state::PlayerState::build_toggle_mute_actions,
    );
    commit_player_settings(ctx).await
}

/// Persist the current `PlayerState`'s volume + `is_muted` into settings.json.
/// Called once from the volume slider's `pointer-event Up` (after a drag or
/// click), and inline from the mute mutators and [`player_set_volume_committed`].
///
/// Writes on a `spawn_blocking` thread so the async runtime worker isn't
/// blocked, under the settings lock so a concurrent `mutate_settings` isn't
/// overwritten. Skips the write when settings already match (common — e.g. a
/// click that didn't change the value).
pub async fn commit_player_settings(ctx: &PlaybackContext) -> Result<(), AppError> {
    let (volume, is_muted) = {
        let s = lock_state(&ctx.player_state);
        (s.volume, s.is_muted)
    };
    let paths = ctx.paths.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        crate::services::settings::mutate_settings_if(&paths, |s| {
            let changed = s.volume != volume || s.playback.is_muted != is_muted;
            s.volume = volume;
            s.playback.is_muted = is_muted;
            changed
        })
    })
    .await
    .map_err(|e| AppError::io("commit_player_settings join", e))?
}

pub fn player_set_playback_speed(ctx: &PlaybackContext, speed: f64) -> Result<(), AppError> {
    ctx.emit_and_execute(|s| s.build_set_speed_actions(speed));
    Ok(())
}

/// [`player_set_playback_speed`] and the write that has it survive a restart, as
/// [`player_set_volume_committed`] is for the volume.
pub async fn player_set_playback_speed_committed(
    ctx: &PlaybackContext,
    speed: f64,
) -> Result<(), AppError> {
    player_set_playback_speed(ctx, speed)?;
    let paths = Arc::clone(&ctx.paths);
    tokio::task::spawn_blocking(move || crate::library::settings::set_playback_speed(&paths, speed))
        .await
        .map_err(AppError::io_source)?
}

pub fn player_set_gapless(ctx: &PlaybackContext, enabled: bool) -> Result<(), AppError> {
    with_state_emit(&ctx.player_state, &ctx.sinks, |s| {
        s.gapless_enabled = enabled;
    });
    Ok(())
}

/// Arm / disarm the sleep-timer's "End of current track" mode. When armed, the
/// playback monitor pauses at the next end-of-stream boundary instead of
/// advancing the queue (see `crates/nekonetease-engine/src/player/engine/handlers.rs`).
/// Session-only — nothing is persisted. The `with_state_emit` re-publishes the
/// light `ViewModel` so the UI's `Player.vm.sleep_at_track_end` (and thus the
/// overflow-menu sleep row) tracks the flag; the monitor disarms it when it
/// fires, which re-emits and auto-clears the row.
///
pub fn player_set_pause_at_track_end(ctx: &PlaybackContext, armed: bool) -> Result<(), AppError> {
    with_state_emit(&ctx.player_state, &ctx.sinks, |s| {
        s.pause_after_current_track = armed;
    });
    Ok(())
}

// --- Graphic equalizer -----------------------------------------------------
//
// EQ state lives on the playback engine's lock-free shared cell, not the
// `PlayerState` machine, so these bypass `with_state_emit` / `execute_actions`
// and write the shared cell directly — the same place `set_volume`/`set_speed`
// ultimately land. They're synchronous and infallible (no decode, no I/O), and
// apply to both the playing track and the gapless-preloaded one at once.

/// Toggle the graphic equalizer on the live player.
pub fn player_set_eq_enabled(ctx: &PlaybackContext, enabled: bool) {
    ctx.engine.set_eq_enabled(enabled);
}

/// Set a single EQ band's gain (dB) on the live player.
pub fn player_set_eq_band(ctx: &PlaybackContext, index: usize, gain_db: f32) {
    ctx.engine.set_eq_band(index, gain_db);
}

/// Replace all EQ band gains on the live player (preset / reset / hydration).
pub fn player_set_eq_gains(ctx: &PlaybackContext, gains: &[f32]) {
    ctx.engine.set_eq_gains(gains);
}

/// Set the EQ preamp / master gain (dB) on the live player.
pub fn player_set_eq_preamp(ctx: &PlaybackContext, preamp_db: f32) {
    ctx.engine.set_eq_preamp(preamp_db);
}

// --- ReplayGain ------------------------------------------------------------
//
// ReplayGain master state (enabled / mode / preamp / prevent-clipping) lives on
// the same lock-free shared cell as the EQ, so these setters mirror the EQ ones:
// synchronous, infallible, and applied to the playing + gapless-preloaded track
// at once. The *per-track* gain is baked per source at play time (see
// `player::playback::replaygain`), not set here.

/// Toggle `ReplayGain` on the live player.
pub fn player_set_replaygain_enabled(ctx: &PlaybackContext, enabled: bool) {
    ctx.engine.set_replaygain_enabled(enabled);
}

/// Set the `ReplayGain` mode (Track / Album) on the live player.
pub fn player_set_replaygain_mode(
    ctx: &PlaybackContext,
    mode: nekonetease_playback::player::playback::replaygain::RgMode,
) {
    ctx.engine.set_replaygain_mode(mode);
}

/// Set the `ReplayGain` preamp (dB) on the live player.
pub fn player_set_replaygain_preamp(ctx: &PlaybackContext, preamp_db: f32) {
    ctx.engine.set_replaygain_preamp(preamp_db);
}

/// Toggle the static peak-based clip guard on the live player.
pub fn player_set_replaygain_prevent_clipping(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_replaygain_prevent_clipping(on);
}

// --- Crossfade -------------------------------------------------------------
//
// Crossfade settings live on a lock-free shared cell like the EQ and ReplayGain
// ones, so these setters are synchronous and infallible. Unlike those two the
// cell is read by the *control* layer (this backend and the playback monitor),
// never by the audio thread — the per-deck ramp it drives lives in `FadeShared`.

/// Toggle crossfade on the live player.
pub fn player_set_crossfade_enabled(ctx: &PlaybackContext, enabled: bool) {
    ctx.engine.set_crossfade_enabled(enabled);
}

/// Set the crossfade length (ms) on the live player. Clamped by the backend.
pub fn player_set_crossfade_duration_ms(ctx: &PlaybackContext, ms: u32) {
    ctx.engine.set_crossfade_duration_ms(ms);
}

/// Also crossfade on a manual track change (next / previous / picking a track).
pub fn player_set_crossfade_manual(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_crossfade_manual(on);
}

/// Leave same-album transitions gapless.
pub fn player_set_crossfade_skip_same_album(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_crossfade_skip_same_album(on);
}

/// Fade out on pause / user stop, and fade back in on resume.
pub fn player_set_crossfade_fade_on_pause(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_crossfade_fade_on_pause(on);
}

/// Whether following the file's rate can change anything here, so the toggle is offered at all.
pub const FOLLOW_RATE_SUPPORTED: bool = output::FOLLOW_RATE_SUPPORTED;

/// Open the output at each track's own sample rate, from the next track on.
pub fn player_set_follow_rate(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_follow_rate(on);
}

/// Give an exclusive device back once a pause has held it long enough, from the next pause on.
pub fn player_set_release_when_paused(ctx: &PlaybackContext, on: bool) {
    ctx.engine.set_release_when_paused(on);
}

/// Write `hold` of silence after each reopen onto a new rate, from the next one on. Blocking: it
/// waits out any reopen in flight, so it belongs on the blocking pool.
pub fn player_set_resync_hold(ctx: &PlaybackContext, hold: Duration) {
    ctx.engine.set_resync_hold(hold);
}

/// Whether this platform has an exclusive backend. Where it doesn't, an exclusive choice falls
/// back to shared on every open, so the picker isn't offered at all.
pub const EXCLUSIVE_SUPPORTED: bool = output::EXCLUSIVE_SUPPORTED;

/// Whether an exclusive claim can be polled instead of event-driven here, which only matters to
/// the drivers that stutter under events.
pub const POLLING_SUPPORTED: bool = output::POLLING_SUPPORTED;

/// Whether an exclusive claim can carry the volume on the device's own control here.
pub const HARDWARE_VOLUME_SUPPORTED: bool = output::HARDWARE_VOLUME_SUPPORTED;

/// Whether an exclusive claim can keep a device lacking the file's rate by converting to one it
/// has, here.
pub const RATE_FALLBACK_SUPPORTED: bool = output::RATE_FALLBACK_SUPPORTED;

/// Whether the device picker chooses where shared output plays too, rather than exclusive alone.
pub const SHARED_DEVICE_SUPPORTED: bool = output::SHARED_DEVICE_SUPPORTED;

/// The devices an exclusive claim can be aimed at, and where [`SHARED_DEVICE_SUPPORTED`] a shared
/// stream too. Blocking: it asks every device.
pub fn output_devices() -> Vec<OutputDevice> {
    output::devices()
}

/// Take the card for ourselves or give it back, now. Blocking: it opens a device, so it belongs
/// on the blocking pool rather than the UI thread.
pub fn player_set_output_choice(ctx: &PlaybackContext, choice: OutputChoice) {
    ctx.engine.set_output_choice(choice);
}

/// Turn off everything the user chose that changes the samples on their way to the device, and
/// follow the file's rate where that does anything, returning the volume it left. The live half
/// only; `library::settings::reset_for_bit_perfect` persists the same set.
///
/// **A volume on the device's own control is left where it is**, only unmuted: it changes no
/// sample, and raising it would be raising the user's speakers. Blocking: it reads the output,
/// which a reopen holds for as long as a device takes to open.
pub fn player_make_bit_perfect(ctx: &PlaybackContext) -> u32 {
    player_set_eq_enabled(ctx, false);
    player_set_replaygain_enabled(ctx, false);
    let volume_on_device =
        ctx.engine.negotiated().is_some_and(|negotiated| negotiated.hardware_volume);
    let mut volume = MAX_VOLUME;
    ctx.emit_and_execute(|s| {
        if volume_on_device {
            volume = s.volume;
        }
        let mut actions = s.build_set_speed_actions(1.0);
        actions.extend(s.build_set_volume_actions(volume));
        actions
    });
    player_set_follow_rate(ctx, true);
    volume
}

// The visualizer has no setter here on purpose: its tap is armed by the
// Now-Playing view's visibility rather than by a persisted setting, so
// `nekonetease-views`' `ui/visualizer/` calls `VisualizerShared::set_enabled` on the cell it
// already holds — `PlaybackEngine::visualizer()` — for snapshotting.

#[cfg(test)]
#[path = "tests/playback_tests.rs"]
mod tests;
