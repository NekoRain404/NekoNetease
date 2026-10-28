//! The technical chip row: what an absent field renders as, and which chips a row shows.

use super::{to_slint_stream_meta, to_slint_track_meta, visible_chip_texts};
use nekonetease_core::entities::cloud::Quality;
use nekonetease_core::entities::track::{StreamInfo, TrackMeta, TrackSummary};

fn meta() -> TrackMeta {
    TrackMeta {
        id: 7,
        codec: Some("flac".to_owned()),
        bitrate: Some(1024),
        sample_rate: Some(44_100),
        bit_depth: Some(16),
        channels: Some(2),
        year: Some(1959),
        genre: Some("Jazz".to_owned()),
    }
}

fn chips(t: &TrackMeta) -> Vec<String> {
    visible_chip_texts(&to_slint_track_meta(t)).iter().map(ToString::to_string).collect()
}

#[test]
fn a_full_row_renders_every_chip_in_the_order_the_view_declares_them() {
    assert_eq!(
        chips(&meta()),
        ["FLAC", "1024 kbps", "44.1 kHz", "16-bit", "Stereo", "1959", "Jazz"]
    );
}

/// **Empty, not absent.** The view gates each chip on `field != ""`, so an `Option` has to arrive
/// as a string it can compare — a row is a Slint struct with no nullable slot.
#[test]
fn a_field_the_track_has_nothing_for_renders_as_an_empty_string() {
    let bare = TrackMeta {
        id: 7,
        codec: None,
        bitrate: None,
        sample_rate: None,
        bit_depth: None,
        channels: None,
        year: None,
        genre: None,
    };
    let row = to_slint_track_meta(&bare);

    assert_eq!(row.codec.as_str(), "");
    assert_eq!(row.bitrate.as_str(), "");
    assert_eq!(row.year.as_str(), "");
    assert_eq!(visible_chip_texts(&row).len(), 0);
}

/// A year of zero is a tag that said nothing rather than a release in year nought, and a chip
/// stating it reads as a fact the file never carried.
#[test]
fn a_year_of_zero_is_not_a_year() {
    let unyeared = TrackMeta { year: Some(0), ..meta() };

    assert_eq!(to_slint_track_meta(&unyeared).year.as_str(), "");
}

#[test]
fn only_the_fields_a_track_carries_reach_the_strip() {
    let partial = TrackMeta { bitrate: None, bit_depth: None, genre: None, ..meta() };

    assert_eq!(chips(&partial), ["FLAC", "44.1 kHz", "Stereo", "1959"]);
}

/// The codec is uppercased here rather than at the chip, so the Summary tab and the strip cannot
/// come to spell it differently.
#[test]
fn the_codec_is_uppercased_once_on_the_way_in() {
    assert_eq!(to_slint_track_meta(&meta()).codec.as_str(), "FLAC");
}

fn cloud_track(stream: Option<StreamInfo>, duration_ms: i64) -> TrackSummary {
    TrackSummary {
        id: 42,
        file_path: String::new(),
        file_name: "Song".to_owned(),
        title: "Song".to_owned(),
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
        stream,
        mv_id: None,
    }
}

fn stream_row(track: &TrackSummary) -> Vec<String> {
    visible_chip_texts(&to_slint_stream_meta(track)).iter().map(ToString::to_string).collect()
}

fn lossless(size_bytes: Option<u64>) -> StreamInfo {
    StreamInfo { format: "flac".to_owned(), level: Quality::Lossless, size_bytes }
}

/// The rate is measured off the payload, never off the tier that was asked for: a `lossless`
/// request that came back as a 320 kbps stream is exactly the case the tier would lie about.
#[test]
fn a_stream_states_its_own_format_and_the_rate_its_payload_measures() {
    assert_eq!(
        stream_row(&cloud_track(Some(lossless(Some(5_000_000))), 125_000)),
        ["FLAC", "320 kbps"]
    );
}

#[test]
fn a_stream_the_endpoint_did_not_size_states_no_rate() {
    assert_eq!(stream_row(&cloud_track(Some(lossless(None)), 125_000)), ["FLAC"]);
}

#[test]
fn a_zero_length_stream_states_no_rate() {
    assert_eq!(stream_row(&cloud_track(Some(lossless(Some(5_000_000))), 0)), ["FLAC"]);
}

/// The row keeps the id so the view's stale-fetch gate still recognises which track it answers
/// for; what it drops is every chip it would otherwise have to invent.
#[test]
fn a_track_with_no_stream_answer_renders_no_chips_and_keeps_its_id() {
    let row = to_slint_stream_meta(&cloud_track(None, 125_000));

    assert_eq!(row.track_id, 42);
    assert_eq!(visible_chip_texts(&row).len(), 0);
}
