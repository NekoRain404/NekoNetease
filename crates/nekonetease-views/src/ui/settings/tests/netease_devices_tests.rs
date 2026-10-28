use super::{device_details, device_label, last_active_phrase};

const NOW_MS: i64 = 1_700_000_000_000;
const MINUTE_MS: i64 = 60_000;

#[test]
fn a_named_device_prefers_its_name_over_its_platform() {
    assert_eq!(device_label("Pixel 8", "Android"), "Pixel 8");
}

#[test]
fn an_unnamed_device_falls_back_to_its_platform() {
    assert_eq!(device_label("  ", " Android "), "Android");
    assert_eq!(device_label("", ""), "未知设备");
}

#[test]
fn details_join_only_the_parts_the_row_carries() {
    let details = device_details("Android", "Hangzhou", NOW_MS, NOW_MS);
    assert_eq!(details, "Android · Hangzhou · 刚刚");
}

#[test]
fn a_missing_place_leaves_no_empty_separator() {
    assert_eq!(device_details("Windows", "", NOW_MS, NOW_MS), "Windows · 刚刚");
    assert_eq!(device_details("", "", NOW_MS, NOW_MS), "刚刚");
}

#[test]
fn an_absent_timestamp_draws_nothing() {
    assert_eq!(last_active_phrase(NOW_MS, 0), "");
    assert_eq!(last_active_phrase(NOW_MS, -1), "");
    assert_eq!(device_details("Windows", "Shanghai", 0, NOW_MS), "Windows · Shanghai");
}

#[test]
fn activity_inside_a_minute_reads_as_just_now() {
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS), "刚刚");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - MINUTE_MS + 1), "刚刚");
}

#[test]
fn minutes_hours_and_days_switch_at_their_own_boundaries() {
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - MINUTE_MS), "1 分钟前");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - 59 * MINUTE_MS), "59 分钟前");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - 60 * MINUTE_MS), "1 小时前");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - 23 * 60 * MINUTE_MS), "23 小时前");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - 24 * 60 * MINUTE_MS), "1 天前");
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS - 6 * 24 * 60 * MINUTE_MS), "6 天前");
}

#[test]
fn a_week_old_session_draws_its_date_instead() {
    let phrase = last_active_phrase(NOW_MS, NOW_MS - 7 * 24 * 60 * MINUTE_MS);

    assert_eq!(phrase.len(), 10);
    assert_eq!(phrase.as_bytes().get(4), Some(&b'-'));
    assert_eq!(phrase.as_bytes().get(7), Some(&b'-'));
}

#[test]
fn a_future_timestamp_reads_as_just_now_rather_than_underflowing() {
    assert_eq!(last_active_phrase(NOW_MS, NOW_MS + 5 * MINUTE_MS), "刚刚");
}
