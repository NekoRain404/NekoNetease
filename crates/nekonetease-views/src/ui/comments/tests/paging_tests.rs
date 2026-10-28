use nekonetease_ui::CommentRow;

use super::paging::{advance_offset, append_unique, sort_changed};

fn row(id: &str) -> CommentRow {
    CommentRow { remote_id: id.into(), ..Default::default() }
}

#[test]
fn a_page_appends_what_it_does_not_already_hold() {
    let mut rows = vec![row("1"), row("2")];
    let added = append_unique(&mut rows, vec![row("2"), row("3")]);

    assert_eq!(added, 1);
    let ids: Vec<&str> = rows.iter().map(|row| row.remote_id.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3"]);
}

#[test]
fn a_row_without_a_server_id_is_never_merged_away() {
    let mut rows = Vec::new();
    append_unique(&mut rows, vec![row(""), row("")]);
    assert_eq!(rows.len(), 2);
}

#[test]
fn the_cursor_moves_by_what_the_server_returned() {
    assert_eq!(advance_offset(0, 20), 20);
    assert_eq!(advance_offset(40, 7), 47);
    assert_eq!(advance_offset(i32::MAX, 5), i32::MAX);
}

#[test]
fn a_returned_page_shorter_than_the_request_does_not_skip_rows() {
    // A hot page can come back with twelve rows where twenty were asked for; advancing by the
    // request would step over the eight the server will still answer.
    assert_eq!(advance_offset(20, 12), 32);
}

#[test]
fn only_a_real_ordering_move_counts_as_a_switch() {
    assert!(!sort_changed(true, true));
    assert!(!sort_changed(false, false));
    assert!(sort_changed(true, false));
    assert!(sort_changed(false, true));
}
