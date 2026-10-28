use super::{duration_text, mv_card_rows, pick_quality, publish_date, volume_amplitude};
use nekonetease_core::entities::cloud::Mv;

fn mv(id: i64, duration_ms: u64, cover: Option<&str>) -> Mv {
    Mv {
        id,
        name: format!("Video {id}"),
        cover_url: cover.map(str::to_owned),
        duration_ms,
        ..Mv::default()
    }
}

/// Slint's `int` is 32-bit, so the card's identity has to cross as text or a large MV id
/// aliases onto a neighbour's.
#[test]
fn a_mv_card_keeps_its_exact_id_and_formatted_duration() {
    let rows = mv_card_rows(vec![mv(3_000_000_000, 215_000, None)]);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].remote_id.as_str(), "3000000000");
    assert_eq!(rows[0].title.as_str(), "Video 3000000000");
    assert_eq!(rows[0].subtitle.as_str(), "3:35");
    assert_eq!(rows[0].artwork_path.as_str(), "");
}

#[test]
fn only_a_remote_cover_url_reaches_the_artwork_tier() {
    let rows = mv_card_rows(vec![
        mv(1, 0, Some("https://p1.music.126.net/cover.jpg")),
        mv(2, 0, Some("file:///tmp/cover.jpg")),
    ]);

    assert_eq!(rows[0].artwork_path.as_str(), "https://p1.music.126.net/cover.jpg");
    assert_eq!(rows[1].artwork_path.as_str(), "");
}

#[test]
fn an_unknown_publish_time_leaves_the_date_blank() {
    assert_eq!(publish_date(0), "");
    assert_eq!(publish_date(u64::MAX), "");
    assert_eq!(publish_date(1_600_000_000_000), "2020-09-13");
}

#[test]
fn an_unknown_duration_is_omitted_rather_than_shown_as_zero() {
    assert_eq!(duration_text(0), "");
    assert_eq!(duration_text(215_000), "3:35");
    assert_eq!(duration_text(3_600_000), "1:00:00");
}

#[test]
fn only_a_tier_the_picker_offers_survives_the_boundary() {
    assert_eq!(pick_quality(1080), 1080);
    assert_eq!(pick_quality(720), 720);
    assert_eq!(pick_quality(480), 480);
    assert_eq!(pick_quality(240), 240);
    // Nothing else may reach `mv_url`, which rejects a tier it does not know.
    assert_eq!(pick_quality(0), 1080);
    assert_eq!(pick_quality(-720), 1080);
}

fn assert_amplitude(percent: i32, expected: f64) {
    let amplitude = volume_amplitude(percent);
    assert!((amplitude - expected).abs() < 1e-9, "{percent}% became {amplitude}");
}

#[test]
fn the_overlay_percentage_becomes_the_players_amplitude() {
    assert_amplitude(0, 0.0);
    assert_amplitude(100, 1.0);
    assert_amplitude(55, 0.55);
    assert_amplitude(-5, 0.0);
    assert_amplitude(140, 1.0);
}
