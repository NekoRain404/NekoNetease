//! Removing entries must leave `current_index` and the deck naming the same track.
//!
//! The pair drifts when the current entry goes: the queue shifts the index onto the successor
//! while the deck still holds the removed track, so the next end-of-stream advances from an index
//! one entry ahead of what was heard.

use std::sync::Arc;

use super::*;
use crate::player::engine::queue::{QueueState, RemovalOutcome};
use nekonetease_core::entities::track::TrackSummary;

fn track(id: i64) -> Arc<TrackSummary> {
    Arc::new(TrackSummary {
        id,
        file_path: format!("/music/{id}.mp3"),
        file_name: format!("{id}.mp3"),
        title: format!("Track {id}"),
        artist: None,
        album: None,
        duration_ms: 180_000,
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

fn queued(track_ids: &[i64]) -> QueueState {
    let mut queue = QueueState::default();
    queue.add_tracks(track_ids.iter().copied().map(track).collect());
    queue
}

fn playing_state(track_ids: &[i64], current: usize) -> PlayerState {
    let mut queue = queued(track_ids);
    queue.current_index = Some(current);
    PlayerState {
        source: queue.get_current().cloned(),
        queue,
        status: PlaybackStatus::Playing,
        position_ms: 42_000,
        duration_ms: 180_000,
        ..PlayerState::default()
    }
}

fn play_media_path(actions: &[PlayerAction]) -> Option<&str> {
    actions.iter().find_map(|action| match action {
        PlayerAction::PlayMedia { file_path, .. } => Some(file_path.as_str()),
        _ => None,
    })
}

fn pause_fade_ms(actions: &[PlayerAction]) -> Option<u64> {
    actions.iter().find_map(|action| match action {
        PlayerAction::Pause { fade_ms } => Some(*fade_ms),
        _ => None,
    })
}

fn stop_fade_ms(actions: &[PlayerAction]) -> Option<u64> {
    actions.iter().find_map(|action| match action {
        PlayerAction::Stop { fade_ms } => Some(*fade_ms),
        _ => None,
    })
}

// --- the queue half of the report -----------------------------------------

#[test]
fn removing_the_current_entry_names_its_successor() {
    let mut queue = queued(&[1, 2, 3]);
    queue.current_index = Some(1);

    assert_eq!(queue.remove_at(1), RemovalOutcome::Removed);
    assert_eq!(queue.current_index, Some(1));
    assert_eq!(queue.get_current().map(|t| t.id), Some(3));
}

#[test]
fn removing_the_current_entry_at_the_tail_reports_no_successor() {
    let mut queue = queued(&[1, 2, 3]);
    queue.current_index = Some(2);

    assert_eq!(queue.remove_at(2), RemovalOutcome::RemovedAtTail);
    assert_eq!(queue.current_index, Some(1));
    assert_eq!(queue.get_current().map(|t| t.id), Some(2));
}

#[test]
fn removing_an_entry_before_the_current_one_shifts_the_index_with_it() {
    let mut queue = queued(&[1, 2, 3]);
    queue.current_index = Some(2);

    assert_eq!(queue.remove_at(0), RemovalOutcome::Kept);
    assert_eq!(queue.current_index, Some(1));
    assert_eq!(queue.get_current().map(|t| t.id), Some(3));
}

#[test]
fn removing_an_entry_after_the_current_one_leaves_the_index_alone() {
    let mut queue = queued(&[1, 2, 3]);
    queue.current_index = Some(0);

    assert_eq!(queue.remove_at(2), RemovalOutcome::Kept);
    assert_eq!(queue.current_index, Some(0));
    assert_eq!(queue.get_current().map(|t| t.id), Some(1));
}

#[test]
fn batch_removal_containing_the_current_entry_names_its_survivor() {
    let mut queue = queued(&[1, 2, 3, 4, 5]);
    queue.current_index = Some(2);

    assert_eq!(queue.remove_batch(&[1, 2]), RemovalOutcome::Removed);
    assert_eq!(queue.current_index, Some(1));
    assert_eq!(queue.get_current().map(|t| t.id), Some(4));
}

#[test]
fn batch_removal_of_the_current_tail_reports_no_successor() {
    let mut queue = queued(&[1, 2, 3, 4, 5]);
    queue.current_index = Some(3);

    assert_eq!(queue.remove_batch(&[3, 4]), RemovalOutcome::RemovedAtTail);
    assert_eq!(queue.current_index, Some(2));
    assert_eq!(queue.get_current().map(|t| t.id), Some(3));
}

#[test]
fn batch_removal_before_the_current_entry_shifts_it_with_them() {
    let mut queue = queued(&[1, 2, 3, 4, 5]);
    queue.current_index = Some(3);

    assert_eq!(queue.remove_batch(&[0, 1]), RemovalOutcome::Kept);
    assert_eq!(queue.current_index, Some(1));
    assert_eq!(queue.get_current().map(|t| t.id), Some(4));
}

// --- the deck half ---------------------------------------------------------

#[test]
fn removing_the_playing_track_plays_its_successor_then_end_of_stream_plays_the_next() {
    let mut state = playing_state(&[1, 2, 3], 0);

    let removal = state.build_remove_actions(0);
    assert_eq!(play_media_path(&removal), Some("/music/2.mp3"));
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(2));
    assert_eq!(state.queue.current_index, Some(0));

    let end = state.build_end_of_stream_actions(EndOfStreamDecision::of(&state));
    assert_eq!(play_media_path(&end), Some("/music/3.mp3"));
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(3));
}

#[test]
fn removing_a_track_before_the_playing_one_keeps_it_on_the_deck() {
    let mut state = playing_state(&[1, 2, 3], 1);

    let actions = state.build_remove_actions(0);

    assert_eq!(actions.len(), 0);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(2));
    assert_eq!(state.position_ms, 42_000);
    assert_eq!(state.queue.current_index, Some(0));
    assert_eq!(state.queue.get_current().map(|t| t.id), Some(2));
}

#[test]
fn removing_the_playing_track_hands_the_deck_to_the_entry_that_took_its_slot() {
    let mut state = playing_state(&[1, 2, 3], 1);

    let actions = state.build_remove_actions(1);

    assert_eq!(play_media_path(&actions), Some("/music/3.mp3"));
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(3));
    assert_eq!(state.position_ms, 0);
    assert_eq!(state.queue.current_index, Some(1));
}

#[test]
fn removing_the_paused_track_hands_over_without_audibly_starting_it() {
    let mut state = playing_state(&[1, 2, 3], 1);
    state.status = PlaybackStatus::Paused;

    let actions = state.build_remove_actions(1);

    assert_eq!(play_media_path(&actions), Some("/music/3.mp3"));
    assert_eq!(pause_fade_ms(&actions), Some(0));
    assert_eq!(state.status, PlaybackStatus::Paused);
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(3));
}

#[test]
fn removing_the_last_playing_track_stops_with_nothing_to_advance_from() {
    let mut state = playing_state(&[1, 2, 3], 2);

    let actions = state.build_remove_actions(2);

    assert_eq!(stop_fade_ms(&actions), Some(0));
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.source.is_none());
    assert_eq!(state.duration_ms, 0);
    assert_eq!(state.position_ms, 0);
}

#[test]
fn batch_removal_containing_the_playing_track_hands_the_deck_on() {
    let mut state = playing_state(&[1, 2, 3, 4, 5], 2);

    let actions = state.build_remove_batch_actions(&[1, 2]);

    assert_eq!(play_media_path(&actions), Some("/music/4.mp3"));
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(4));
    assert_eq!(state.queue.current_index, Some(1));
}

#[test]
fn batch_removal_of_the_playing_tail_stops_playback() {
    let mut state = playing_state(&[1, 2, 3, 4, 5], 4);

    let actions = state.build_remove_batch_actions(&[3, 4]);

    assert_eq!(stop_fade_ms(&actions), Some(0));
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.source.is_none());
}

#[test]
fn removing_the_seated_entry_while_stopped_seats_its_successor() {
    let mut state = playing_state(&[1, 2, 3], 0);
    state.status = PlaybackStatus::Stopped;
    state.position_ms = 0;

    let actions = state.build_remove_actions(0);

    assert_eq!(actions.len(), 0);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert_eq!(state.source.as_ref().map(|t| t.id), Some(2));
    assert_eq!(state.duration_ms, 180_000);
    assert_eq!(state.queue.current_index, Some(0));
}
