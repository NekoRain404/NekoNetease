//! An owned cloud playlist's reorder writes — the row menu's "Move up"/"Move down" and a drag.
//!
//! The drag hands over display indices, which a `NetEase` row has no id for. The canonical order
//! is kept here as remote song ids, captured before the display sort, so either gesture asks the
//! service for the playlist's own order.

use std::sync::Arc;

use slint::ComponentHandle;

use super::detail::{apply_optimistic_reorder, is_manual_order, rollback_reorder};
use super::{PlaylistsUi, refresh_detail};
use crate::ui::util::parse_cloud_id;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::DAILY_PLAYLIST_ID;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, PlaylistDetail};

/// The open cloud playlist's song ids in playlist order, tagged with the playlist they belong
/// to so a fetch that loses the account-generation race cannot reorder the one now open.
pub(super) struct CloudOrder {
    pub(super) playlist_id: i64,
    pub(super) song_ids: Vec<i64>,
}

pub(super) fn publish(playlists_ui: &PlaylistsUi, playlist_id: i64, song_ids: Vec<i64>) {
    *playlists_ui.cloud_order.lock() = Some(CloudOrder { playlist_id, song_ids });
}

pub(super) fn clear(playlists_ui: &PlaylistsUi) {
    *playlists_ui.cloud_order.lock() = None;
}

/// Move `song_id` by `delta` places, clamped at both ends. `None` when the song is absent or
/// already at the end the move points at.
pub(super) fn moved_order(order: &[i64], song_id: i64, delta: i32) -> Option<Vec<i64>> {
    let index = order.iter().position(|&id| id == song_id)?;
    let target = index.saturating_add_signed(isize::try_from(delta).ok()?).min(order.len() - 1);
    if target == index {
        return None;
    }
    let mut next = order.to_vec();
    let moved = next.remove(index);
    next.insert(target, moved);
    Some(next)
}

/// Wire the row menu's two entries. A move is refused unless the rows on screen are the
/// playlist's own order: a filter or any other sort makes the displayed neighbours someone
/// else's, and the user is told nothing because a move they cannot map is not an error.
pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let state = state.clone();
    let playlists_ui = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_move_cloud_track(move |remote_id, delta| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<PlaylistDetail>();
        let field = g.get_sort_field();
        let dir = g.get_sort_dir();
        if !g.get_cloud_owned()
            || !is_manual_order(&field, &dir)
            || playlists_ui.detail.cache.is_filtered()
        {
            return;
        }
        let Some(song_id) = parse_cloud_id(remote_id.as_str()) else {
            log::warn!("playlists::move_cloud_track: invalid NetEase song ID");
            return;
        };
        let playlist_id = playlists_ui.detail_playlist_id();
        if playlist_id < 0 || !writable_playlist(playlist_id) {
            return;
        }
        if !can_reorder(&playlists_ui, playlist_id) {
            return;
        }
        let Some((previous, next)) = take_move(&playlists_ui, playlist_id, song_id, delta) else {
            return;
        };
        let state = state.clone();
        let playlists_ui = playlists_ui.clone();
        let weak = weak.clone();
        state.runtime.clone().spawn(async move {
            if let Err(error) = state.cloud.reorder_playlist_tracks(playlist_id, &next).await {
                let detail = describe(&error);
                log::warn!("playlists::move_cloud_track({playlist_id}): {detail}");
                toast::notify(ToastKind::OperationFailed, detail);
                restore(&playlists_ui, playlist_id, Some(previous));
                return;
            }
            if let Err(error) = refresh_detail(&state, &playlists_ui, weak, playlist_id).await {
                log::warn!("playlists::move_cloud_track refresh: {}", describe(&error));
            }
        });
    });
}

/// Splice the new order in and hand back `(previous, next)` for the write and its rollback.
fn take_move(
    playlists_ui: &PlaylistsUi,
    playlist_id: i64,
    song_id: i64,
    delta: i32,
) -> Option<(Vec<i64>, Vec<i64>)> {
    let mut guard = playlists_ui.cloud_order.lock();
    let order = guard.as_mut()?;
    if order.playlist_id != playlist_id {
        return None;
    }
    let next = moved_order(&order.song_ids, song_id, delta)?;
    let previous = std::mem::replace(&mut order.song_ids, next.clone());
    Some((previous, next))
}

/// The daily recommendations page is assembled under a sentinel id rather than naming a stored
/// playlist, so every reorder against it would be answered with a failure. Its synthetic header
/// also reports no creator and no subscription, which reads as ownership.
fn writable_playlist(playlist_id: i64) -> bool {
    playlist_id != DAILY_PLAYLIST_ID
}

/// Whether the detail holds every row the playlist does.
///
/// Both reorder paths write the ids the detail currently holds, so a fetch that dropped some
/// would make the service delete them from the playlist. The grid's listing is the only place the
/// server's own count survives — the detail header's `track_count` is overwritten with the rows
/// it fetched — and an unknown count cannot prove the write complete, so it refuses too.
fn can_reorder(playlists_ui: &PlaylistsUi, playlist_id: i64) -> bool {
    let rows = playlists_ui.detail.position_order.lock().len();
    let served = playlists_ui.grid_stats_by_id(playlist_id).map(|stats| stats.track_count);
    if rows_are_complete(rows, served) {
        return true;
    }
    log::warn!(
        "playlists({playlist_id}): {rows} rows on screen against {served:?} served; \
         refusing to reorder"
    );
    false
}

fn rows_are_complete(rows: usize, served: Option<i32>) -> bool {
    served.and_then(|count| usize::try_from(count).ok()) == Some(rows)
}

fn restore(playlists_ui: &PlaylistsUi, playlist_id: i64, previous: Option<Vec<i64>>) {
    let Some(previous) = previous else { return };
    let mut guard = playlists_ui.cloud_order.lock();
    if let Some(order) = guard.as_mut()
        && order.playlist_id == playlist_id
    {
        order.song_ids = previous;
    }
}

/// A drag committed against the cloud detail, with both canonical orders already permuted
/// optimistically. The write is the caller's; [`rollback_drag`] is what a failed one owes.
pub(super) struct ReorderCommit {
    pub(super) playlist_id: i64,
    pub(super) next: Vec<i64>,
    previous_cloud: Option<Vec<i64>>,
    previous_position: Vec<i64>,
}

/// Apply a drag drop to the visible order *and* the remote order it must mirror.
///
/// The drag hands over display indices, so the same guard as the explicit moves applies: a
/// filter or any sort but the playlist's own makes the neighbours someone else's. The two caches
/// are always fetched together, and leaving `cloud_order` stale would make a Move up/down right
/// after a drag write an order the user never saw.
pub(super) fn apply_drag(
    ui: &AppWindow,
    playlists_ui: &PlaylistsUi,
    from: usize,
    to: usize,
) -> Option<ReorderCommit> {
    let g = ui.global::<PlaylistDetail>();
    let field = g.get_sort_field();
    let dir = g.get_sort_dir();
    if !g.get_cloud_owned()
        || !is_manual_order(&field, &dir)
        || playlists_ui.detail.cache.is_filtered()
    {
        return None;
    }
    let playlist_id = playlists_ui.detail_playlist_id();
    if playlist_id < 0 || !writable_playlist(playlist_id) {
        return None;
    }
    if !can_reorder(playlists_ui, playlist_id) {
        return None;
    }
    let previous_position = apply_optimistic_reorder(ui, playlists_ui, from, to)?;
    let next = playlists_ui.detail.position_order.lock().clone();
    let previous_cloud = {
        let mut guard = playlists_ui.cloud_order.lock();
        guard
            .as_mut()
            .filter(|order| order.playlist_id == playlist_id)
            .map(|order| std::mem::replace(&mut order.song_ids, next.clone()))
    };
    Some(ReorderCommit { playlist_id, next, previous_cloud, previous_position })
}

/// Put both caches back when the service refuses the reorder.
pub(super) fn rollback_drag(ui: &AppWindow, playlists_ui: &PlaylistsUi, commit: ReorderCommit) {
    restore(playlists_ui, commit.playlist_id, commit.previous_cloud);
    rollback_reorder(ui, playlists_ui, commit.previous_position);
}

#[cfg(test)]
#[path = "tests/cloud_drag_tests.rs"]
mod tests;
