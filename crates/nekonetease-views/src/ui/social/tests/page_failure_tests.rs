//! A page that failed is not a surface that failed: what already loaded stays on screen, and the
//! notice belongs in the footer where the More pill was.

use nekonetease_testkit::{block_after, code_tokens};

const NOTIFICATIONS: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/social/notifications-view.slint");
const PROFILE: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/social/profile-view.slint");
const FETCH: &str = include_str!("../fetch.rs");

#[test]
fn the_empty_state_error_block_is_gated_on_an_empty_list() {
    let notifications = code_tokens(NOTIFICATIONS);
    assert!(
        notifications.contains(
            "if Social.notices-error != \"\" && Social.notices.length == 0: GridEmptyState"
        ),
        "the empty-state block paints over the rows it should leave alone"
    );

    let profile = code_tokens(PROFILE);
    assert!(
        profile.contains(
            "if Social.events-error != \"\" && Social.events.length == 0: GridEmptyState"
        ),
        "the empty-state block paints over the rows it should leave alone"
    );
}

#[test]
fn a_failed_page_reports_at_the_footer_and_keeps_the_loaded_rows() {
    let fetch = code_tokens(FETCH);

    for (function, notice, first_page_error) in [
        ("fn apply_events", "set_events_more_error", "set_events_error"),
        ("fn apply_notices", "set_notices_more_error", "set_notices_error"),
    ] {
        let body = block_after(&fetch, function);
        assert!(!body.is_empty(), "{function} is missing");

        let failure = block_after(body, "Err(error) =>");
        assert!(
            failure.contains(&format!(
                "if more {{ social_ui.{notice}(message); }} else {{ social_ui.{first_page_error}(message); }}"
            )),
            "{function} routes a failed page away from the footer notice"
        );
        assert!(
            !failure.contains("replace_rows"),
            "{function}'s failed page rebuilds the rows it should leave alone"
        );

        let success = block_after(body, "Ok(page) =>");
        assert!(
            success.contains(&format!("{notice}(SharedString::default())")),
            "{function} leaves a stale footer notice after a page lands"
        );
    }
}

#[test]
fn the_failed_page_footer_retries_the_page_and_returns_after_success() {
    for (view, notice, retry) in [
        (NOTIFICATIONS, "Social.notices-more-error", "Social.load-more-notices()"),
        (PROFILE, "Social.events-more-error", "Social.load-more-events()"),
    ] {
        let code = code_tokens(view);
        let row = block_after(&code, &format!("if {notice} != \"\""));
        assert!(!row.is_empty(), "{notice} has no footer row");
        assert!(row.contains(retry), "{notice}'s footer does not retry the page it failed on");
        assert!(
            !row.contains("Social.retry()"),
            "retrying a page must not reload the surface and drop its rows"
        );
        assert!(
            code.contains(&format!("if {notice} == \"\" && ")),
            "the More pill is not gated on the footer notice clearing"
        );
    }
}
