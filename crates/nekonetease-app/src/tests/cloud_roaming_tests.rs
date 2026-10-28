use super::trim_history;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_engine::player::engine::queue::QueueState;
use std::sync::Arc;

fn track(id: i64) -> Arc<TrackSummary> {
    Arc::new(TrackSummary {
        id,
        file_path: String::new(),
        file_name: String::new(),
        title: format!("Song {id}"),
        artist: None,
        album: None,
        duration_ms: 1_000,
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

#[test]
fn roaming_reclaims_played_tracks_without_losing_current_or_upcoming_songs() {
    let mut queue = QueueState::default();
    queue.add_tracks((1..=50).map(track).collect());
    queue.current_index = Some(30);
    trim_history(&mut queue);
    assert_eq!(queue.tracks.len(), 40);
    assert_eq!(queue.current_index, Some(20));
    assert_eq!(queue.get_current().map(|track| track.id), Some(31));
    assert_eq!(queue.peek_next().map(|track| track.id), Some(32));
}

#[test]
fn a_song_recommended_again_is_not_pruned_from_the_future_queue() {
    let mut queue = QueueState::default();
    queue.add_tracks((1..=50).map(track).collect());
    queue.add_tracks(vec![track(1)]);
    queue.current_index = Some(30);
    trim_history(&mut queue);
    assert_eq!(queue.tracks.len(), 41);
    assert_eq!(queue.get_current().map(|track| track.id), Some(31));
    assert_eq!(queue.tracks_in_play_order().last().map(|track| track.id), Some(1));
}
