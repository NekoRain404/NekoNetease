//! What `build_gapless_actions` makes of the deck's handover, including the control ops that can
//! land in the monitor's gap before the emit lock.
//!
//! The handover happens in the audio callback, so the deck has already taken the staged entry by
//! the time the state hears about it. Each case below therefore asserts where the queue index ends
//! up as well as what was counted: an index left behind makes the next end-of-stream skip a track.

use std::sync::Arc;

use super::*;
use crate::player::engine::types::PlaybackStatus;
use nekonetease_playback::player::playback::replaygain::TrackReplayGain;

fn make_summary(id: i64, title: &str, duration_ms: i64) -> Arc<TrackSummary> {
    Arc::new(TrackSummary {
        id,
        file_path: format!("/music/{id}.mp3"),
        file_name: format!("{id}.mp3"),
        title: title.to_owned(),
        artist: None,
        album: None,
        duration_ms,
        artwork_path: None,
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

/// Three queued tracks playing the first, with the second already staged on the deck — the state
/// the monitor reads when the handover lands.
fn gapless_state() -> (PlayerState, StagedTrack) {
    let mut state = PlayerState::default();
    let tracks = [
        make_summary(1, "One", 100_000),
        make_summary(2, "Two", 120_000),
        make_summary(3, "Three", 140_000),
    ];
    state.source = Some(Arc::clone(&tracks[0]));
    state.queue.add_tracks(tracks.to_vec());
    state.queue.current_index = Some(0);
    state.status = PlaybackStatus::Playing;
    state.duration_ms = 100_000;
    state.position_ms = 99_000;
    let staged = StagedTrack {
        slot: 1,
        track: Arc::clone(&tracks[1]),
        replaygain: TrackReplayGain::default(),
    };
    (state, staged)
}

#[test]
fn gapless_handover_advances_exactly_one_entry() {
    let (mut state, staged) = gapless_state();
    let decision = GaplessDecision::of(&state);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert_eq!(state.queue.current_index, Some(1));
    assert_eq!(state.current_track().map(|track| track.id), Some(2));
    assert_eq!(state.position_ms, 0);
    assert_eq!(state.duration_ms, 120_000);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(actions, vec![PlayerAction::UpdatePlayCount(1)]);
}

#[test]
fn gapless_handover_after_a_pick_leaves_the_picked_track_playing() {
    let (mut state, staged) = gapless_state();
    let decision = GaplessDecision::of(&state);
    let picked = Arc::clone(&state.queue.tracks[2]);
    let _ = play_track_inner(&mut state, picked, None);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert!(actions.is_empty(), "the track playback left must not be credited to the pick");
    assert_eq!(state.current_track().map(|track| track.id), Some(3));
    assert_eq!(state.queue.current_index, Some(2), "the index must follow the deck onto the pick");
    assert_eq!(state.to_view_model_light().current_track.map(|track| track.id), Some(3));
    assert_eq!(state.status, PlaybackStatus::Playing);
}

#[test]
fn gapless_handover_after_a_pause_seats_the_handed_over_track_without_counting_a_play() {
    let (mut state, staged) = gapless_state();
    let decision = GaplessDecision::of(&state);
    let _ = state.build_pause_actions(0);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert!(actions.is_empty(), "a play the user interrupted is not counted");
    assert_eq!(state.queue.current_index, Some(1), "the deck already took the staged entry");
    assert_eq!(state.current_track().map(|track| track.id), Some(2));
    assert_eq!(state.to_view_model_light().current_track.map(|track| track.id), Some(2));
    assert_eq!(state.status, PlaybackStatus::Paused);
}

#[test]
fn gapless_handover_after_a_seek_leaves_the_position_and_index_alone() {
    let (mut state, staged) = gapless_state();
    let decision = GaplessDecision::of(&state);
    let _ = state.build_seek_actions(40_000);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert!(actions.is_empty(), "a seek must not move the queue");
    assert_eq!(state.position_ms, 40_000);
    assert_eq!(state.queue.current_index, Some(0));
    assert_eq!(state.current_track().map(|track| track.id), Some(1));
}

#[test]
fn gapless_handover_into_a_track_the_queue_dropped_stops_without_naming_it() {
    let (mut state, staged) = gapless_state();
    let decision = GaplessDecision::of(&state);
    state.queue.remove_at(1);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert_eq!(state.current_track(), None);
    assert_eq!(state.duration_ms, 0);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(actions.iter().any(|action| matches!(action, PlayerAction::Stop { .. })));
}

#[test]
fn gapless_handover_into_the_second_copy_of_a_duplicated_track_keeps_its_slot() {
    let mut state = PlayerState::default();
    let repeated = make_summary(1, "Looped", 100_000);
    state.queue.add_tracks(vec![Arc::clone(&repeated), Arc::clone(&repeated)]);
    state.queue.current_index = Some(0);
    state.source = Some(Arc::clone(&repeated));
    state.status = PlaybackStatus::Playing;
    state.duration_ms = 100_000;
    state.position_ms = 99_000;
    assert_eq!(state.queue.peek_next_entry().map(|(slot, _)| slot), Some(1));
    let staged = StagedTrack {
        slot: 1,
        track: Arc::clone(&repeated),
        replaygain: TrackReplayGain::default(),
    };
    let decision = GaplessDecision::of(&state);

    let actions = state.build_gapless_actions(decision, Some(staged));

    assert_eq!(state.queue.current_index, Some(1), "the copy the deck took, not the one it left");
    assert_eq!(actions, vec![PlayerAction::UpdatePlayCount(1)]);
}
