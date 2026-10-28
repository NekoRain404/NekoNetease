//! The optimistic comment-like patch, shared by the thread and a reply floor.

use slint::{Model, ModelRc, SharedString};

use nekonetease_ui::CommentRow;

pub(super) struct LikeSnapshot {
    index: usize,
    remote_id: SharedString,
    liked: bool,
    liked_count: i32,
}

/// Flips one row before the request runs. The remote id must still match the click, so a
/// stale index cannot patch a neighbour after a page shifted the rows.
pub(super) fn apply(
    model: &ModelRc<CommentRow>,
    index: usize,
    remote_id: &str,
    next: bool,
) -> Option<LikeSnapshot> {
    if remote_id.is_empty() {
        return None;
    }
    let row = model.row_data(index)?;
    if row.remote_id.as_str() != remote_id {
        return None;
    }
    let snapshot = LikeSnapshot {
        index,
        remote_id: row.remote_id.clone(),
        liked: row.liked,
        liked_count: row.liked_count,
    };
    let delta = if next { 1 } else { -1 };
    model.set_row_data(
        index,
        CommentRow {
            liked: next,
            liked_count: row.liked_count.saturating_add(delta).max(0),
            ..row
        },
    );
    Some(snapshot)
}

pub(super) fn rollback(model: &ModelRc<CommentRow>, snapshot: &LikeSnapshot) {
    let Some(row) = model.row_data(snapshot.index) else { return };
    if row.remote_id != snapshot.remote_id {
        return;
    }
    model.set_row_data(
        snapshot.index,
        CommentRow { liked: snapshot.liked, liked_count: snapshot.liked_count, ..row },
    );
}
