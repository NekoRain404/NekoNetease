//! The "Recent comments" preview the cloud playlist and album details share.
//!
//! The band has no scroller, so both surfaces cap the thread at the same count; the projection
//! lives here so they cannot drift on how one comment reads.

use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::ui::util::clamp_i64_to_i32;
use nekonetease_core::entities::cloud::Comment;
use nekonetease_ui::PlaylistCommentRow as UiCommentRow;

pub(crate) const COMMENT_PREVIEW_LIMIT: usize = 4;

pub(crate) fn comment_preview_model(comments: &[Comment]) -> ModelRc<UiCommentRow> {
    let preview = comments.get(..comments.len().min(COMMENT_PREVIEW_LIMIT)).unwrap_or(comments);
    let rows = preview
        .iter()
        .map(|comment| UiCommentRow {
            id: clamp_i64_to_i32(comment.id),
            remote_id: SharedString::from(comment.id.to_string()),
            author: SharedString::from(comment.author.as_str()),
            content: SharedString::from(comment.content.as_str()),
            time_ms: clamp_i64_to_i32(comment.time_ms),
            liked: comment.liked,
            liked_count: clamp_i64_to_i32(comment.liked_count),
        })
        .collect::<Vec<_>>();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// The row's exact wire id, for the like request. `PlaylistCommentRow.id` aliases every value past
/// `i32::MAX` to one number, so it can guard the row but must not address the server.
pub(crate) fn comment_wire_id(
    model: &ModelRc<UiCommentRow>,
    index: usize,
    comment_id: i32,
) -> Option<i64> {
    let row = model.row_data(index)?;
    if row.id != comment_id {
        return None;
    }
    crate::ui::util::parse_cloud_id(row.remote_id.as_str())
}

/// What the optimistic write replaced, kept so a failed request can put the row back.
pub(crate) struct CommentLikeSnapshot {
    index: usize,
    remote_id: SharedString,
    previous_liked: bool,
    previous_count: i32,
    applied_liked: bool,
    applied_count: i32,
}

/// Flips one row's heart before the request runs. The snapshot is `None` when the row's `id` no
/// longer matches the click, so a stale index cannot patch a neighbour.
pub(crate) fn apply_like_toggle(
    model: &ModelRc<UiCommentRow>,
    index: usize,
    comment_id: i32,
    next: bool,
) -> Option<CommentLikeSnapshot> {
    if index >= model.row_count() {
        return None;
    }
    let mut row = model.row_data(index)?;
    if row.id != comment_id {
        return None;
    }
    let snapshot = CommentLikeSnapshot {
        index,
        remote_id: row.remote_id.clone(),
        previous_liked: row.liked,
        previous_count: row.liked_count,
        applied_liked: next,
        applied_count: if next {
            row.liked_count.saturating_add(1)
        } else {
            row.liked_count.saturating_sub(1)
        },
    };
    row.liked = snapshot.applied_liked;
    row.liked_count = snapshot.applied_count;
    model.set_row_data(index, row);
    Some(snapshot)
}

/// Puts a failed toggle back, unless a later click on the same row already overwrote it.
pub(crate) fn rollback_like_toggle(model: &ModelRc<UiCommentRow>, snapshot: &CommentLikeSnapshot) {
    if snapshot.index >= model.row_count() {
        return;
    }
    let Some(mut row) = model.row_data(snapshot.index) else {
        return;
    };
    if row.remote_id != snapshot.remote_id
        || row.liked != snapshot.applied_liked
        || row.liked_count != snapshot.applied_count
    {
        return;
    }
    row.liked = snapshot.previous_liked;
    row.liked_count = snapshot.previous_count;
    model.set_row_data(snapshot.index, row);
}
