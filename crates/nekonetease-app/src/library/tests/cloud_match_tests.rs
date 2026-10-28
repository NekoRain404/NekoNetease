//! The matcher's identity rule: what counts as the same title, artist line and length, and what
//! must stay apart. A wrong pair here would put the wrong track in a synced playlist, so the
//! negative cases carry as much weight as the positive ones.

use nekonetease_core::entities::cloud::{Artist, Song};
use nekonetease_core::error::AppError;
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries::fixtures::insert_test_track;

use super::*;

fn song(id: i64, title: &str, artists: &[&str], duration_ms: u64) -> Song {
    Song {
        id,
        name: title.to_owned(),
        artists: artists
            .iter()
            .map(|name| Artist { name: (*name).to_owned(), ..Artist::default() })
            .collect(),
        duration_ms,
        ..Song::default()
    }
}

fn candidate(id: i64, title: &str, artist: &str, duration_ms: i64) -> MatchCandidate {
    MatchCandidate { id, title: title.to_owned(), artist: Some(artist.to_owned()), duration_ms }
}

#[test]
fn width_case_whitespace_and_artist_order_do_not_make_another_recording() {
    let cloud = song(1, "ＳＯＮＧ　ＴＩＴＬＥ", &["B", "A"], 180_000);
    let local = [candidate(4, "song title", "A / B", 178_000)];

    let found = match_songs(&[cloud], &local);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].local_track_id, 4);
    assert_eq!(found[0].duration_delta_ms, 2_000);
}

#[test]
fn the_duration_tolerance_is_inclusive_at_three_seconds() {
    let cloud = song(1, "Song", &["Artist"], 180_000);
    let at_limit = [candidate(4, "Song", "Artist", 183_000)];
    let past_limit = [candidate(4, "Song", "Artist", 183_001)];

    assert_eq!(match_songs(std::slice::from_ref(&cloud), &at_limit).len(), 1);
    assert_eq!(match_songs(&[cloud], &past_limit).len(), 0);
}

#[test]
fn the_closest_candidate_wins() {
    let cloud = song(1, "Song", &["Artist"], 180_000);
    let local = [candidate(4, "Song", "Artist", 181_500), candidate(9, "Song", "Artist", 179_000)];

    let found = match_songs(&[cloud], &local);

    assert_eq!(found[0].local_track_id, 9);
    assert_eq!(found[0].duration_delta_ms, 1_000);
}

#[test]
fn a_tie_breaks_on_the_lower_local_id() {
    let cloud = song(1, "Song", &["Artist"], 180_000);
    let local = [candidate(9, "Song", "Artist", 179_000), candidate(4, "Song", "Artist", 181_000)];

    let found = match_songs(&[cloud], &local);

    assert_eq!(found[0].local_track_id, 4);
}

#[test]
fn a_different_title_or_artist_is_a_different_recording() {
    let cloud = song(1, "Song", &["Artist"], 180_000);
    let other_title = [candidate(4, "Other Song", "Artist", 180_000)];
    let other_artist = [candidate(4, "Song", "Other Artist", 180_000)];

    assert_eq!(match_songs(std::slice::from_ref(&cloud), &other_title).len(), 0);
    assert_eq!(match_songs(&[cloud], &other_artist).len(), 0);
}

#[test]
fn feat_markers_and_separators_name_the_same_artists() {
    let cloud = song(1, "Song", &["A feat. B"], 180_000);
    let local = [candidate(4, "Song", "B / A", 180_000)];

    assert_eq!(match_songs(&[cloud], &local).len(), 1);
}

#[test]
fn an_untagged_artist_line_matches_an_empty_one() {
    let cloud = song(1, "Song", &[], 180_000);
    let local =
        [MatchCandidate { id: 4, title: "Song".to_owned(), artist: None, duration_ms: 180_000 }];

    assert_eq!(match_songs(&[cloud], &local).len(), 1);
}

#[test]
fn songs_without_a_candidate_are_absent_from_the_result() {
    let songs = [song(1, "Song", &["Artist"], 180_000), song(2, "Missing", &["Artist"], 180_000)];
    let local = [candidate(4, "Song", "Artist", 180_000)];

    let found = match_songs(&songs, &local);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cloud_song_id, 1);
}

#[tokio::test]
async fn the_library_lookup_matches_against_stored_tracks() -> Result<(), AppError> {
    let db = DbPool::test_pool().await?;
    let local_id =
        insert_test_track(&db, "/music/known.mp3", "Known Song", "Known Artist", "Album", "Rock")
            .await?;
    let songs = [
        song(1, "Known Song", &["Known Artist"], 181_000),
        song(2, "Unknown Song", &["Known Artist"], 180_000),
    ];

    let found = match_cloud_disk(&db, &songs).await?;

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cloud_song_id, 1);
    assert_eq!(found[0].local_track_id, local_id);
    Ok(())
}
