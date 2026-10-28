//! Projection from the wire entity to the Slint row.

use nekonetease_core::entities::comment::ThreadComment;
use nekonetease_ui::CommentRow;

use crate::ui::util::clamp_i64_to_i32;

/// `viewer_id` is the account the delete action is offered to; the row carries the author's own
/// id, so an anonymous thread offers none.
pub(super) fn to_row(comment: &ThreadComment, viewer_id: i64) -> CommentRow {
    CommentRow {
        id: clamp_i64_to_i32(comment.comment.id),
        remote_id: comment.comment.id.to_string().into(),
        author: comment.comment.author.as_str().into(),
        content: comment.comment.content.as_str().into(),
        display_time: display_time(comment.comment.time_ms).into(),
        liked: comment.comment.liked,
        liked_count: clamp_i64_to_i32(comment.comment.liked_count),
        reply_count: count_to_i32(comment.reply_count),
        parent_id: comment.parent_id.to_string().into(),
        owned: viewer_id > 0 && comment.author_id == viewer_id,
        avatar_url: comment.avatar_url.as_str().into(),
    }
}

pub(super) fn count_to_i32(count: u64) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// Formatting happens here because Slint's `int` is 32-bit and a `NetEase` timestamp is past
/// `i32::MAX`; the surface gets a label rather than a number it would truncate.
fn display_time(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
