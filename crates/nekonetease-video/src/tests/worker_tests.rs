use super::{mpv_seconds, start_option};

#[test]
fn a_zero_start_leaves_mpvs_own_default_alone() {
    assert_eq!(start_option(0), None);
}

/// A quality switch hands the position over through `loadfile`, so the option is the only
/// thing keeping the viewer where they were.
#[test]
fn a_restart_carries_the_position_in_seconds() {
    assert_eq!(start_option(1_500), Some("start=1.500".to_owned()));
    assert_eq!(start_option(61_234), Some("start=61.234".to_owned()));
}

/// The carry across a whole second is what a hand-rolled split gets wrong.
#[test]
fn a_position_keeps_its_milliseconds() {
    assert_eq!(mpv_seconds(0), "0.000");
    assert_eq!(mpv_seconds(999), "0.999");
    assert_eq!(mpv_seconds(1_000), "1.000");
}
