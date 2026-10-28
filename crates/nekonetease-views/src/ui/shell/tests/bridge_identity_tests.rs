use super::to_slint_track;
use nekonetease_artwork::media::image::cover_thumbs::CoverThumbs;
use nekonetease_core::entities::track::TrackSummary;

fn summary(id: i64) -> TrackSummary {
    TrackSummary {
        id,
        file_path: String::new(),
        file_name: String::new(),
        title: format!("Song {id}"),
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
    }
}

#[test]
fn player_identity_stays_distinct_above_the_slint_integer_boundary() {
    let covers = CoverThumbs::default();
    let boundary = to_slint_track(&summary(2_147_483_647), &covers);
    let next = to_slint_track(&summary(2_147_483_648), &covers);
    let large = to_slint_track(&summary(3_000_000_000), &covers);

    assert_eq!(boundary.remote_id.as_str(), "2147483647");
    assert_eq!(next.remote_id.as_str(), "2147483648");
    assert_eq!(large.remote_id.as_str(), "3000000000");
    assert_ne!(boundary.remote_id, next.remote_id);
    assert_ne!(next.remote_id, large.remote_id);
}

#[test]
fn a_missing_player_track_has_no_highlight_identity() {
    let empty = nekonetease_ui::TrackSummaryRow::default();
    assert!(empty.remote_id.is_empty());
}
