use super::*;

#[test]
fn a_frame_below_the_cap_is_left_alone() {
    assert_eq!(render_size(640, 360), Some((640, 360)));
    assert_eq!(render_size(960, 540), Some((960, 540)));
}

#[test]
fn a_frame_past_the_cap_scales_on_its_long_edge() {
    assert_eq!(render_size(1920, 1080), Some((960, 540)));
    assert_eq!(render_size(3840, 2160), Some((960, 540)));
    assert_eq!(render_size(1080, 1920), Some((303, 540)));
    assert_eq!(render_size(1000, 100), Some((960, 96)));
}

#[test]
fn a_degenerate_frame_has_no_render_size() {
    assert_eq!(render_size(0, 0), None);
    assert_eq!(render_size(0, 1080), None);
    assert_eq!(render_size(1920, 0), None);
}

#[test]
fn a_ratio_thinner_than_a_pixel_still_gets_two() {
    assert_eq!(render_size(100_000, 1), Some((960, 2)));
    assert_eq!(render_size(1, 100_000), Some((2, 540)));
}

#[test]
fn the_volume_maps_onto_mpvs_hundred_point_scale() {
    assert!((volume_to_mpv(0.0) - 0.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(0.5) - 50.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(1.0) - 100.0).abs() < f64::EPSILON);
}

#[test]
fn a_volume_outside_the_scale_is_clamped_and_nonsense_is_silence() {
    assert!((volume_to_mpv(-1.0) - 0.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(3.0) - 100.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(f64::NAN) - 0.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(f64::INFINITY) - 0.0).abs() < f64::EPSILON);
    assert!((volume_to_mpv(f64::NEG_INFINITY) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn hardware_decoding_is_auto_safe_until_it_is_turned_off() {
    assert_eq!(hwdec_option(true), "auto-safe");
    assert_eq!(hwdec_option(false), "no");
    assert_eq!(hwdec_option(true), HWDEC_ENABLED);
    assert_eq!(hwdec_option(false), HWDEC_DISABLED);
}

#[test]
fn a_stride_is_four_bytes_a_pixel_and_the_length_matches_the_pixels() {
    let frame = VideoFrame { width: 3, height: 2, pixels: vec![0; 24] };
    assert_eq!(frame.stride(), 12);
    assert_eq!(frame.byte_len(), 24);
    assert_eq!(frame.byte_len(), frame.pixels.len());
}

#[test]
fn the_title_is_the_last_path_segment_without_a_query_or_fragment() {
    assert_eq!(title_from_url("https://example.test/a/song.mp4"), "song.mp4");
    assert_eq!(title_from_url("https://example.test/a/song.mp4?token=secret"), "song.mp4");
    assert_eq!(title_from_url("https://example.test/a/song.mp4#t=12"), "song.mp4");
    assert_eq!(title_from_url("file:///music/MV.mp4"), "MV.mp4");
    assert_eq!(title_from_url("stream"), "stream");
    assert_eq!(title_from_url("https://example.test/"), "example.test");
}

#[test]
fn only_an_unfinished_file_keeps_the_overlay_open() {
    for state in [VideoState::Loading, VideoState::Playing, VideoState::Paused] {
        assert!(state.is_open(), "{state:?} should be open");
    }
    for state in [VideoState::Idle, VideoState::Ended, VideoState::Failed("nope".to_owned())] {
        assert!(!state.is_open(), "{state:?} should be closed");
    }
}
