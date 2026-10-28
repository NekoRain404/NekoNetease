use std::rc::Rc;

use slint::{Model, ModelRc, VecModel};

use nekonetease_core::error::AppError;
use nekonetease_ui::CommentRow;

use super::like;

fn row(remote_id: &str, liked: bool, liked_count: i32) -> CommentRow {
    CommentRow { remote_id: remote_id.into(), liked, liked_count, ..Default::default() }
}

fn model(rows: Vec<CommentRow>) -> ModelRc<CommentRow> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

#[test]
fn a_like_flips_the_row_before_the_request_runs() {
    let rows = model(vec![row("7", false, 3)]);
    let snapshot = like::apply(&rows, 0, "7", true);

    assert!(snapshot.is_some());
    let patched = rows.row_data(0);
    assert!(matches!(patched, Some(row) if row.liked && row.liked_count == 4));
}

#[test]
fn a_rollback_puts_back_the_count_and_the_flag() -> Result<(), AppError> {
    let rows = model(vec![row("7", false, 3)]);
    let Some(snapshot) = like::apply(&rows, 0, "7", true) else {
        return Err(AppError::NotFound("the fixture row must match the clicked id".to_owned()));
    };
    like::rollback(&rows, &snapshot);

    let restored = rows.row_data(0);
    assert!(matches!(restored, Some(row) if !row.liked && row.liked_count == 3));
    Ok(())
}

#[test]
fn a_stale_index_cannot_patch_the_row_that_took_its_place() {
    let rows = model(vec![row("7", false, 3), row("8", false, 1)]);
    assert!(like::apply(&rows, 0, "8", true).is_none());

    let untouched = rows.row_data(0);
    assert!(matches!(untouched, Some(row) if !row.liked && row.liked_count == 3));
}

#[test]
fn an_out_of_range_index_is_refused() {
    let rows = model(vec![row("7", false, 3)]);
    assert!(like::apply(&rows, 4, "7", true).is_none());
}

#[test]
fn unliking_never_drives_the_count_below_zero() {
    let rows = model(vec![row("7", true, 0)]);
    let snapshot = like::apply(&rows, 0, "7", false);

    assert!(snapshot.is_some());
    let patched = rows.row_data(0);
    assert!(matches!(patched, Some(row) if !row.liked && row.liked_count == 0));
}

#[test]
fn a_rollback_after_the_rows_moved_leaves_the_new_row_alone() -> Result<(), AppError> {
    let rows = model(vec![row("7", false, 3)]);
    let Some(snapshot) = like::apply(&rows, 0, "7", true) else {
        return Err(AppError::NotFound("the fixture row must match the clicked id".to_owned()));
    };
    rows.set_row_data(0, row("9", true, 12));
    like::rollback(&rows, &snapshot);

    let other = rows.row_data(0);
    assert!(matches!(other, Some(row) if row.remote_id.as_str() == "9" && row.liked_count == 12));
    Ok(())
}
