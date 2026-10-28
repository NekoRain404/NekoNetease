//! Favorites All-Songs row selection: modifier-aware clicks, clear, and the per-row `selected`
//! flag writer that drives row highlight + checkbox state.
//!
//! Thin adapter over [`crate::ui::list_selection`] — the `TrackList` component reads each row's
//! `selected: bool` for its checkbox tick and accent-tinted background, so a click that only
//! updates `selected-ids` (without re-stamping the per-row flag) leaves the row visually
//! un-selected even though it counts toward the "{n} selected" chip.

use crate::ui::list_selection;
use nekonetease_ui::{Favorites, TrackListRow as UiTrackListRow};

/// UI-thread-only: re-stamp selection onto a freshly-built row list before it's pushed into the
/// Slint model. Invoked from `songs::apply_filtered_tracks` so a filter change / library refresh
/// doesn't drop the user's existing selection.
pub fn restamp_rows(g: &Favorites, rows: &mut [UiTrackListRow]) {
    list_selection::restamp_curated_rows(g, rows);
}
