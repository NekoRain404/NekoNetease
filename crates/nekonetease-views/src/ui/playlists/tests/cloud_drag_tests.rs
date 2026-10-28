//! The cloud playlist's drag-to-reorder, pinned at its three seams: the mount, the shared
//! reorder callback's branch, and the pair of orders a drop permutes.

use super::{rows_are_complete, writable_playlist};
use nekonetease_core::entities::cloud::DAILY_PLAYLIST_ID;
use nekonetease_testkit::code_tokens;

const DRAGGABLE_LIST: &str = include_str!(
    "../../../../../nekonetease-ui/ui/components/track-list/draggable-track-list.slint"
);
const DETAIL_VIEW: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/playlist-detail.slint");
const DETAIL_CALLBACKS: &str = include_str!("../callbacks/detail.rs");
const DETAIL_ORDER: &str = include_str!("../detail_order.rs");

/// The cloud detail used to mount the drag-free wrapper, whose rows are not draggable at all.
/// The drag is armed only where the account owns the playlist and the rows are the playlist's
/// own order — the same three terms the local list asks, with ownership in place of "not smart".
#[test]
fn cloud_playlists_reorder_through_the_draggable_list_when_owned() {
    let src = code_tokens(DETAIL_VIEW);
    let cloud = src
        .split_once("cloud-tl := DraggableTrackList")
        .and_then(|(_, rest)| rest.split_once("TrackListScrollbars"))
        .map_or("", |(body, _)| body);
    assert!(!cloud.is_empty(), "the cloud branch must reach the drag through `DraggableTrackList`");
    assert!(
        cloud.contains(
            "reorder-enabled: PlaylistDetail.cloud-owned \
             && PlaylistDetail.sort-field == \"position\" \
             && PlaylistDetail.sort-dir != \"desc\" \
             && PlaylistDetail.filter == \"\""
        ),
        "a drop is only meaningful on an owned, unfiltered, ascending position sort"
    );
    assert!(
        cloud.contains("cloud-playlist-reorderable: PlaylistDetail.cloud-owned"),
        "the row menu's explicit moves and the drag share the ownership gate"
    );
    assert!(
        cloud.contains("reorder(from, to) => { PlaylistDetail.reorder(from, to); }"),
        "the drag's drop is the same callback the local list commits through"
    );
}

/// `DraggableTrackList` now hosts a cloud row, so it owes the contract `TrackList` publishes:
/// without it the row loses its remote id, its cloud menu, or both.
#[test]
fn the_draggable_list_publishes_the_cloud_row_contract() {
    let list = code_tokens(DRAGGABLE_LIST);
    assert!(
        list.contains("in property <bool> cloud-rows: false;"),
        "the cloud host must be able to turn the row's cloud behaviour on"
    );
    assert!(
        list.contains("callback move-cloud-row(string"),
        "the row menu's explicit moves have to reach the dragging host"
    );
    assert!(
        list.contains("favorite-clickable: !root.cloud-rows || root.cloud-favorite-clickable;"),
        "a cloud row's heart is the host's choice, not the local list's default"
    );
    assert!(
        list.contains("cloud-playlist-reorderable: root.cloud-playlist-reorderable;"),
        "the row needs its own flag for the menu's move entries"
    );
}

/// A cloud detail's canonical order is remote, so its drag write is `reorder_playlist_tracks`
/// and the local `reorder_playlist` would reorder rows the playlist never reads. A failed write
/// puts both the display and the remote order back, then says why.
#[test]
fn cloud_reorders_cross_the_remote_track_ids_and_roll_back() {
    let src = code_tokens(DETAIL_CALLBACKS);
    assert!(
        src.contains("if ui.global::<PlaylistDetail>().get_cloud_collection()"),
        "the shared reorder callback has to route a cloud detail before reading local ids"
    );
    assert!(
        src.contains("reorder_cloud(&ui, &s, &pu, from_u, to_u)"),
        "the cloud branch must not fall through to `reorder_playlist`"
    );

    let helper = src
        .split_once("fn reorder_cloud")
        .and_then(|(_, rest)| rest.split_once("fn wire_edit_artwork"))
        .map_or("", |(body, _)| body);
    assert!(
        helper.contains("detail_order::apply_drag(ui, playlists_ui, from, to)"),
        "the drag has to permute both orders before the write"
    );
    assert!(
        helper.contains("s.cloud.reorder_playlist_tracks(playlist_id, &next)"),
        "the write crosses remote song ids, not local row ids"
    );
    assert!(
        helper.contains("detail_order::rollback_drag(&ui, &pu, commit)")
            && helper.contains("toast::notify(ToastKind::OperationFailed, detail)"),
        "a refused write has to restore both orders and tell the user"
    );
    assert!(
        helper.contains("refresh_detail(&s, &pu, weak, playlist_id)"),
        "the server's answer is the order a later move or drag must start from"
    );
}

/// `position_order` and `cloud_order` are fetched together, so a drop that permutes one and not
/// the other leaves the explicit move menu writing an order the user never saw.
#[test]
fn cloud_drags_permute_and_restore_both_orders() {
    let src = code_tokens(DETAIL_ORDER);
    let apply = src.split_once("pub(super) fn apply_drag").map_or("", |(_, rest)| rest);
    assert!(
        apply.contains("g.get_cloud_owned()")
            && apply.contains("!is_manual_order(&field, &dir)")
            && apply.contains("playlists_ui.detail.cache.is_filtered()"),
        "the drag owes the explicit move's order and filter guard"
    );
    assert!(
        apply.contains("apply_optimistic_reorder(ui, playlists_ui, from, to)"),
        "the display order is permuted by the shared local reorder"
    );
    assert!(
        apply.contains("std::mem::replace(&mut order.song_ids, next.clone())"),
        "the remote order has to mirror the display order the same tick"
    );

    let rollback = src.split_once("pub(super) fn rollback_drag").map_or("", |(_, rest)| rest);
    assert!(
        rollback.contains("restore(playlists_ui, commit.playlist_id, commit.previous_cloud)"),
        "a failed write restores the remote order"
    );
    assert!(
        rollback.contains("rollback_reorder(ui, playlists_ui, commit.previous_position)"),
        "and the display order it was painted from"
    );
}

/// Both gestures write the ids the detail holds, so a detail missing rows the playlist still has
/// would make the service delete them. The guard is the count comparison, not a toast: a reorder
/// the app cannot perform is not an error the user caused.
#[test]
fn a_reorder_is_refused_unless_the_detail_holds_the_whole_playlist() {
    let src = code_tokens(DETAIL_ORDER);
    let apply = src.split_once("pub(super) fn apply_drag").map_or("", |(_, rest)| rest);
    assert!(
        apply.contains("can_reorder(playlists_ui, playlist_id)"),
        "a drag must not write a row list shorter than the playlist"
    );

    let wire = src
        .split_once("pub(super) fn wire")
        .and_then(|(_, rest)| rest.split_once("fn take_move"))
        .map_or("", |(body, _)| body);
    assert!(
        wire.contains("can_reorder(&playlists_ui, playlist_id)"),
        "the row menu's move writes the same list and owes the same guard"
    );
}

#[test]
fn completeness_compares_the_details_rows_with_the_served_count() {
    assert!(rows_are_complete(0, Some(0)), "an empty playlist is complete as itself");
    assert!(rows_are_complete(5, Some(5)));
    assert!(!rows_are_complete(4, Some(5)), "the missing row would be deleted by the write");
    assert!(
        !rows_are_complete(6, Some(5)),
        "a row the playlist never had is equally untrustworthy"
    );
    assert!(!rows_are_complete(5, None), "an unknown count cannot prove the list complete");
    assert!(!rows_are_complete(5, Some(-1)), "a negative count names no row list");
}

/// The daily recommendations page wears a sentinel id and a header with neither creator nor
/// subscription, which `cloud_playlist_owned` reads as ownership. Its id names no stored playlist,
/// so a reorder against it could only fail after the rows had already moved on screen.
#[test]
fn the_daily_page_is_not_a_playlist_writes_can_target() {
    assert!(!writable_playlist(DAILY_PLAYLIST_ID));
    assert!(writable_playlist(7));
}
