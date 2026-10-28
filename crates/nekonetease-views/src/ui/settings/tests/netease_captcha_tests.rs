use std::time::Duration;

use super::cooldown_remaining;

#[test]
fn a_fresh_request_holds_the_button_for_the_whole_delay() {
    assert_eq!(cooldown_remaining(Duration::ZERO), 60);
}

#[test]
fn every_passed_second_shortens_the_wait_by_one() {
    assert_eq!(cooldown_remaining(Duration::from_secs(1)), 59);
    assert_eq!(cooldown_remaining(Duration::from_secs(30)), 30);
    assert_eq!(cooldown_remaining(Duration::from_secs(59)), 1);
}

#[test]
fn the_button_frees_only_on_the_tick_that_reaches_the_delay() {
    assert_eq!(cooldown_remaining(Duration::from_millis(59_999)), 1);
    assert_eq!(cooldown_remaining(Duration::from_secs(60)), 0);
}

#[test]
fn an_overrun_never_reads_negative() {
    assert_eq!(cooldown_remaining(Duration::from_secs(61)), 0);
    assert_eq!(cooldown_remaining(Duration::from_secs(7_200)), 0);
}
