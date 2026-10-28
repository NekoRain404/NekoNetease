//! Paging arithmetic, kept pure so the ordering and cursor rules are testable without a window.

use std::collections::HashSet;

use nekonetease_ui::CommentRow;

/// Appends a page, dropping rows the model already holds.
///
/// A hot thread can repeat a comment across two offsets, and a duplicate would both render
/// twice and give the second copy the first one's index in the like callback. Rows without an
/// id (a locally completed post the server did not echo) are never deduplicated.
pub(super) fn append_unique(rows: &mut Vec<CommentRow>, page: Vec<CommentRow>) -> usize {
    let mut seen: HashSet<String> =
        rows.iter().map(|row| row.remote_id.to_string()).filter(|id| !id.is_empty()).collect();
    let mut added = 0;
    for row in page {
        let id = row.remote_id.to_string();
        if id.is_empty() || seen.insert(id) {
            rows.push(row);
            added += 1;
        }
    }
    added
}

/// Advances the cursor by what the server returned, not by the requested limit: a short hot
/// page would otherwise skip exactly as many rows as it came up short.
pub(super) fn advance_offset(offset: i32, returned: usize) -> i32 {
    let returned = i32::try_from(returned).unwrap_or(i32::MAX);
    offset.saturating_add(returned)
}

/// Whether a sort pick moved the ordering. The two routes count offsets independently, so a
/// move has to reset the cursor and drop the rows rather than continue this one.
pub(super) fn sort_changed(current_hot: bool, next_hot: bool) -> bool {
    current_hot != next_hot
}
