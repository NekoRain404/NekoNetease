//! A page that failed is not a thread that failed: the rows already loaded stay, and the notice
//! belongs in the footer where the More pill was.

use nekonetease_testkit::{block_after, code_tokens};

const VIEW: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/comments/comments-view.slint");
const LOAD: &str = include_str!("../load.rs");

#[test]
fn the_empty_state_error_block_is_gated_on_an_empty_thread() {
    let code = code_tokens(VIEW);
    assert!(
        code.contains("if Comments.error != \"\" && Comments.rows.length == 0: GridEmptyState"),
        "the empty-state block paints over the rows it should leave alone"
    );
    assert!(
        code.contains(
            "if Comments.replies-error != \"\" && Comments.replies.length == 0: GridEmptyState"
        ),
        "the reply floor paints its error over the rows it already holds"
    );
}

#[test]
fn a_failed_more_reports_at_the_footer_and_keeps_the_thread() {
    let load = code_tokens(LOAD);
    let body = block_after(&load, "pub(super) fn more");
    assert!(!body.is_empty(), "load::more is missing");

    let failure = block_after(body, "Err(error) =>");
    assert!(failure.contains("set_more_error"), "a failed page never reaches the footer notice");
    assert!(!failure.contains("set_error("), "a failed page takes the empty-state error slot");
    assert!(
        !failure.contains("replace_rows"),
        "a failed page rebuilds the rows it should leave alone"
    );

    let success = block_after(body, "Ok(page) =>");
    assert!(
        success.contains("set_more_error(SharedString::default())"),
        "a landed page leaves the footer notice behind"
    );
}

#[test]
fn the_more_row_retries_the_page_and_returns_after_success() {
    let code = code_tokens(VIEW);
    let row = block_after(&code, "if Comments.more-error != \"\"");
    assert!(!row.is_empty(), "the More row has no failure state");
    assert!(row.contains("Comments.load-more()"), "the failure row cannot retry the page");
    assert!(!row.contains("Comments.reload()"), "retrying a page must not reload the thread");
    assert!(
        code.contains("if Comments.more-error == \"\" && Comments.has-more"),
        "the More pill is not gated on the footer notice clearing"
    );
}
