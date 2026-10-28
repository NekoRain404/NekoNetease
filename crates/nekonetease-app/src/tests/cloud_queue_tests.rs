use std::sync::Arc;

use super::queue::{CloudQueuePlan, CloudQueueShuffle, playback_candidates, queue_cloud_tracks};
use super::source::track_summary;
use nekonetease_core::entities::cloud::Song;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_engine::player::engine::queue::QueueState;
use nekonetease_engine::player::engine::types::RepeatMode;
use rand::{SeedableRng, rngs::StdRng};

fn replace(queue: &mut QueueState, shuffle: CloudQueueShuffle, start_index: usize) {
    CloudQueuePlan {
        tracks: vec![track(19), track(20), track(21), track(22)],
        start_index,
        shuffle,
    }
    .apply(queue, &mut StdRng::seed_from_u64(42));
}

#[test]
fn explicit_ordered_playback_leaves_shuffle_and_starts_at_the_selected_row() {
    let mut queue = QueueState::default();
    queue.set_repeat_mode(RepeatMode::All);
    replace(&mut queue, CloudQueueShuffle::Shuffle, 2);
    assert!(queue.shuffle_enabled);
    assert_eq!(queue.get_current().map(|track| track.id), Some(21));

    replace(&mut queue, CloudQueueShuffle::Ordered, 0);

    assert!(!queue.shuffle_enabled);
    assert_eq!(ids(&queue), [19, 20, 21, 22]);
    assert_eq!(queue.get_current().map(|track| track.id), Some(19));
    assert_eq!(queue.repeat_mode, RepeatMode::All);
    assert_eq!(
        playback_candidates(4, 1, RepeatMode::Off, CloudQueueShuffle::Ordered.enabled(true)),
        [1, 2, 3]
    );
}

#[test]
fn row_activation_preserves_both_transport_shuffle_preferences() {
    let mut queue = QueueState::default();
    replace(&mut queue, CloudQueueShuffle::Shuffle, 0);

    replace(&mut queue, CloudQueueShuffle::Preserve, 2);
    assert!(queue.shuffle_enabled);
    assert_eq!(queue.get_current().map(|track| track.id), Some(21));

    replace(&mut queue, CloudQueueShuffle::Ordered, 0);
    replace(&mut queue, CloudQueueShuffle::Preserve, 2);
    assert!(!queue.shuffle_enabled);
    assert_eq!(ids(&queue), [19, 20, 21, 22]);
    assert_eq!(queue.get_current().map(|track| track.id), Some(21));
}

fn track(id: i64) -> Arc<TrackSummary> {
    Arc::new(track_summary(Song { id, name: format!("Song {id}"), ..Song::default() }, None, false))
}

fn ids(queue: &QueueState) -> Vec<i64> {
    queue.tracks_in_play_order().iter().map(|track| track.id).collect()
}

#[test]
fn play_next_preserves_large_ids_and_the_batch_order() {
    let mut queue = QueueState::default();
    queue.add_tracks(vec![track(19), track(20)]);
    queue.current_index = Some(0);

    queue_cloud_tracks(&mut queue, vec![track(2_147_483_648), track(3_000_000_000)], true);

    assert_eq!(ids(&queue), [19, 2_147_483_648, 3_000_000_000, 20]);
    assert_eq!(queue.current_index, Some(0));
}

#[test]
fn add_to_queue_appends_without_replacing_the_current_track() {
    let mut queue = QueueState::default();
    queue.add_tracks(vec![track(19), track(20)]);
    queue.current_index = Some(0);

    queue_cloud_tracks(&mut queue, vec![track(2_147_483_648), track(3_000_000_000)], false);

    assert_eq!(ids(&queue), [19, 20, 2_147_483_648, 3_000_000_000]);
    assert_eq!(queue.current_index, Some(0));
}

#[test]
fn play_next_on_an_empty_queue_keeps_the_exact_ids() {
    let mut queue = QueueState::default();

    queue_cloud_tracks(&mut queue, vec![track(2_147_483_648), track(3_000_000_000)], true);

    assert_eq!(ids(&queue), [2_147_483_648, 3_000_000_000]);
    assert_eq!(queue.current_index, None);
}
