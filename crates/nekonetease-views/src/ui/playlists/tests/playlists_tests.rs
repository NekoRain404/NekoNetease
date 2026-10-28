//! The Playlists grid's filter + sort pass — the last of the four entity
//! grids to get one. Same three cases its siblings pin, since all four run
//! the identical `row_match::Needle::contains` walk over a single name.
//!
//! Plus the four things drag-to-reorder needs to stay alive, none of which any
//! other pin can see go missing, and the cloud/local store split the CRUD
//! commits take.

use super::detail::{POSITION_FIELD, is_manual_order, sort_playlist_tracks};
use super::detail_order::moved_order;
use super::*;
use nekonetease_core::entities::track::TrackListRow;
use nekonetease_testkit::code_tokens;

const DRAGGABLE_LIST: &str = include_str!(
    "../../../../../nekonetease-ui/ui/components/track-list/draggable-track-list.slint"
);
const QUEUE_SHEET: &str = include_str!("../../../../../nekonetease-ui/ui/views/queue-sheet.slint");
const DETAIL_VIEW: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/playlist-detail.slint");
const SHARED_DETAIL_WIRING: &str = include_str!("../../callbacks/track_detail.rs");
const DETAIL: &str = include_str!("../detail.rs");
const DETAIL_CLOUD: &str = include_str!("../detail_cloud.rs");
const TAB_PILLS: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/tab-pills.slint");
const DIALOG_GLOBAL: &str = include_str!("../../../../../nekonetease-ui/ui/globals/dialog.slint");
const CLOUD_TRACKS: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/cloud-playlist-tracks.slint");
const PLAYLISTS_TAB: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/playlists-tab.slint");
const PLAYLIST_GRID: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/grid/playlist-grid.slint");
const PLAYLIST_DETAIL_GLOBAL: &str =
    include_str!("../../../../../nekonetease-ui/ui/globals/playlists.slint");
const GRID_CALLBACKS: &str = include_str!("../callbacks/grid.rs");
const ROAMING_CALLBACKS: &str = include_str!("../callbacks/categories.rs");
const ROAMING_TAB: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/my-library/roaming-tab.slint");
const CRUD_CALLBACKS: &str = include_str!("../callbacks/crud.rs");
const ADD_PICKER_CALLBACKS: &str = include_str!("../callbacks/files/add_picker.rs");
const DETAIL_CALLBACKS: &str = include_str!("../callbacks/detail.rs");
const CLOUD_MENU: &str = include_str!(
    "../../../../../nekonetease-ui/ui/components/track-list/cloud-track-context-menu.slint"
);
const TRACK_LIST: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/track-list/track-list.slint");
const TRACK_ROW: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/track-list/track-list-row.slint");
const DETAIL_ORDER: &str = include_str!("../detail_order.rs");

/// Minimal `PlaylistStats` builder — only the fields the grid filter / sort
/// read matter. Regular playlist, so `smart_criteria` never parses.
fn playlist(id: i64, name: &str, track_count: i32, updated_at: &str) -> PlaylistStats {
    PlaylistStats {
        id,
        name: name.to_owned(),
        description: None,
        thumbnail_path: None,
        is_smart: false,
        smart_criteria: None,
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: updated_at.to_owned(),
        custom_thumbnail: false,
        track_count,
        total_duration_ms: 0,
    }
}

fn names(data: &GridData, indices: &[usize]) -> Vec<String> {
    indices.iter().map(|&i| data.playlists[i].name.clone()).collect()
}

fn track(id: i64, title: &str, album: &str, favorite: bool, length: i64) -> TrackListRow {
    TrackListRow {
        id,
        file_path: String::new(),
        file_name: title.to_owned(),
        title: title.to_owned(),
        artist: Some("Artist".to_owned()),
        album_artist: None,
        album: Some(album.to_owned()),
        genre: None,
        track_number: None,
        disc_number: None,
        year: None,
        duration_ms: length,
        artwork_path: None,
        is_favorite: favorite,
        rating: 0,
        album_id: None,
        artist_id: None,
        genre_id: None,
        date_added: String::new(),
        sort_key: None,
        mv_id: None,
    }
}

#[test]
fn personal_roaming_dislike_is_guarded_and_skips_only_after_trash_succeeds() {
    let callbacks = nekonetease_testkit::strip_line_comments(ROAMING_CALLBACKS);
    let tab = nekonetease_testkit::strip_line_comments(ROAMING_TAB);
    let handler = callbacks
        .split_once("on_dislike_roaming_current")
        .map(|(_, rest)| rest)
        .unwrap_or_default();

    assert!(tab.contains("if Playlists.roaming-mode == 0"));
    assert!(tab.contains("Playlists.dislike-roaming-current();"));
    assert!(callbacks.contains("fn current_personal_roaming_song("));
    assert!(handler.contains("current_personal_roaming_song("));
    assert!(handler.contains("state_task.cloud.fm_trash(song_id)"));
    assert!(handler.contains("state_task.playback_ctx().emit_and_execute"));
    assert!(handler.matches("current_personal_roaming_song(").count() >= 2);

    let helper = callbacks
        .split_once("async fn trash_then_skip")
        .map(|(_, helper)| helper)
        .unwrap_or_default();
    let trash = helper.find("trash.await?").unwrap_or(usize::MAX);
    let advance = helper.find("skip()").unwrap_or(usize::MAX);
    assert!(trash < advance, "FM must be trashed successfully before playback advances");
}

#[test]
fn compute_indices_with_empty_filter_keeps_all_playlists() {
    let data = GridData::new(vec![
        playlist(1, "Alpha", 1, "2026-01-02T00:00:00Z"),
        playlist(2, "Bravo", 1, "2026-01-01T00:00:00Z"),
    ]);
    let idx = compute_indices(&data, "name", "asc", "");
    assert_eq!(names(&data, &idx), ["Alpha", "Bravo"]);
}

#[test]
fn compute_indices_filter_matches_name_case_insensitively() {
    let data = GridData::new(vec![
        playlist(1, "Late Night Drive", 1, "2026-01-03T00:00:00Z"),
        playlist(2, "Night Shift", 1, "2026-01-02T00:00:00Z"),
        playlist(3, "Morning Coffee", 1, "2026-01-01T00:00:00Z"),
    ]);
    let by_name = compute_indices(&data, "name", "asc", "NIGHT");
    assert_eq!(names(&data, &by_name), ["Late Night Drive", "Night Shift"]);
}

#[test]
fn compute_indices_filter_ignores_accents_the_way_the_search_view_does() {
    // A playlist name is whatever the user typed, so it carries whatever
    // their keyboard makes easy — and the query later may not.
    let data = GridData::new(vec![
        playlist(1, "Café Sessions", 1, "2026-01-02T00:00:00Z"),
        playlist(2, "Morning Coffee", 1, "2026-01-01T00:00:00Z"),
    ]);
    let by_name = compute_indices(&data, "name", "asc", "cafe");
    assert_eq!(names(&data, &by_name), ["Café Sessions"]);
}

#[test]
fn compute_indices_default_sort_is_most_recently_updated_first() {
    // `playlist_stats` already returns `updated_at DESC`, so the default arm
    // re-derives that order rather than leaving whatever the filter produced.
    let data = GridData::new(vec![
        playlist(1, "Oldest", 1, "2026-01-01T00:00:00Z"),
        playlist(2, "Newest", 1, "2026-03-01T00:00:00Z"),
        playlist(3, "Middle", 1, "2026-02-01T00:00:00Z"),
    ]);
    let desc = compute_indices(&data, "updated", "desc", "");
    assert_eq!(names(&data, &desc), ["Newest", "Middle", "Oldest"]);
    let asc = compute_indices(&data, "updated", "asc", "");
    assert_eq!(names(&data, &asc), ["Oldest", "Middle", "Newest"]);
}

#[test]
fn cloud_playlist_rows_share_the_standard_header_and_position_is_the_default_sort() {
    assert!(PLAYLISTS_TAB.contains("open-playlist(id) => { Playlists.open-playlist(id); }"));
    assert!(PLAYLIST_GRID.contains("clicked => { root.open-playlist(playlist.remote_id); }"));
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("callback open-playlist(string"));
    assert!(GRID_CALLBACKS.contains("playlist_id.to_string().parse::<i64>()"));
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("in-out property <string> sort-field: \"position\";"));
    assert!(CLOUD_TRACKS.contains("TrackList {"));
    assert!(CLOUD_TRACKS.contains("columns <=> root.columns;"));
    assert!(CLOUD_TRACKS.contains("field == \"track_number\" ? root.position-field : field"));
    assert!(CLOUD_TRACKS.contains("root.play-track(id, index)"));
    assert!(DETAIL_VIEW.contains("PlaylistDetail.play-cloud-row(id, index)"));
}

#[test]
fn cloud_playlist_tags_edit_crosses_the_remote_id_and_refreshes() {
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("property <string> cloud-tags"));
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("callback update-cloud-tags("));
    assert!(TAB_PILLS.contains("Dialog.open-edit-playlist-tags"));
    assert!(DIALOG_GLOBAL.contains("DialogKind.edit-playlist-tags"));
    assert!(CRUD_CALLBACKS.contains("update_playlist_tags(id, &tags)"));
    assert!(CRUD_CALLBACKS.contains("parse_cloud_id"));
}

#[test]
fn cloud_playlist_collect_toggle_writes_through_the_service() {
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("property <bool> cloud-subscribed"));
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("callback toggle-cloud-subscribe();"));
    assert!(TAB_PILLS.contains("PlaylistDetail.toggle-cloud-subscribe()"));
    assert!(TAB_PILLS.contains("PlaylistDetail.cloud-owned"));
    assert!(DETAIL.contains("playlist.subscribed"));
    assert!(DETAIL_CALLBACKS.contains("cloud.subscribe_playlist("));
    assert!(DETAIL_CALLBACKS.contains("set_cloud_subscribed(next)"));
}

#[test]
fn cloud_playlist_rows_keep_the_full_netease_id_across_the_slint_boundary() {
    let row =
        to_slint_playlist_row(&playlist(3_000_000_000, "Large ID", 0, "2026-01-01T00:00:00Z"));
    assert_eq!(row.id, i32::MAX, "legacy UI handle remains bounded");
    assert_eq!(row.remote_id.to_string(), "3000000000");
}

#[test]
fn cloud_playlist_detail_exposes_recent_comments_without_changing_track_identity() {
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("property <[PlaylistCommentRow]> comments"));
    assert!(PLAYLIST_DETAIL_GLOBAL.contains("callback retry-comments();"));
    assert!(DETAIL_VIEW.contains("Recent comments"));
    // The band keeps its loading and failure states, and the retry reaches the loader rather
    // than dismissing the section.
    assert!(DETAIL_VIEW.contains("PlaylistDetail.comments-loading"));
    assert!(DETAIL_VIEW.contains("PlaylistDetail.comments-error"));
    assert!(DETAIL_VIEW.contains("PlaylistDetail.retry-comments()"));
    assert!(DETAIL_CLOUD.contains("playlist_comments(playlist_id, 0)"));
    assert!(DETAIL_CLOUD.contains("load_playlist_comments"));
    assert!(DETAIL_VIEW.contains("PlaylistDetail.play-cloud-row(id, index)"));
}

#[test]
fn cloud_track_rows_keep_distinct_full_width_song_ids_for_playback_and_sorting() {
    let first = crate::ui::tracks::to_slint_track_list_rows(&[track(
        2_147_483_648,
        "First",
        "Album",
        false,
        100,
    )]);
    let second = crate::ui::tracks::to_slint_track_list_rows(&[track(
        3_000_000_000,
        "Second",
        "Album",
        false,
        200,
    )]);

    assert_eq!(first[0].id, i32::MAX);
    assert_eq!(second[0].id, i32::MAX);
    assert_eq!(first[0].remote_id.to_string(), "2147483648");
    assert_eq!(second[0].remote_id.to_string(), "3000000000");
    assert_ne!(crate::ui::tracks::model_key(&first[0]), crate::ui::tracks::model_key(&second[0]));
}

#[test]
fn cloud_playlist_sort_covers_position_title_album_favorite_and_length() {
    let seed = || {
        vec![
            track(1, "Bravo", "Zulu", true, 300),
            track(2, "Alpha", "Able", false, 100),
            track(3, "Charlie", "Mango", true, 200),
        ]
    };
    let ids = |rows: &[TrackListRow]| rows.iter().map(|row| row.id).collect::<Vec<_>>();

    let mut rows = seed();
    sort_playlist_tracks(&mut rows, &[3, 1, 2], POSITION_FIELD, "asc");
    assert_eq!(ids(&rows), [3, 1, 2]);
    sort_playlist_tracks(&mut rows, &[3, 1, 2], POSITION_FIELD, "desc");
    assert_eq!(ids(&rows), [2, 1, 3]);

    let mut rows = seed();
    sort_playlist_tracks(&mut rows, &[], "title", "asc");
    assert_eq!(ids(&rows), [2, 1, 3]);
    sort_playlist_tracks(&mut rows, &[], "title", "desc");
    assert_eq!(ids(&rows), [3, 1, 2]);

    let mut rows = seed();
    sort_playlist_tracks(&mut rows, &[], "album", "asc");
    assert_eq!(ids(&rows), [2, 3, 1]);

    let mut rows = seed();
    sort_playlist_tracks(&mut rows, &[], "favorite", "asc");
    assert_eq!(ids(&rows), [2, 1, 3]);
    sort_playlist_tracks(&mut rows, &[], "favorite", "desc");
    assert_eq!(ids(&rows), [3, 1, 2]);

    let mut rows = seed();
    sort_playlist_tracks(&mut rows, &[], "length", "asc");
    assert_eq!(ids(&rows), [2, 3, 1]);
    sort_playlist_tracks(&mut rows, &[], "length", "desc");
    assert_eq!(ids(&rows), [1, 3, 2]);
}

/// A row drag and the list's own drag-pan are one gesture, and whichever element
/// owns it the other gets nothing: a pan intercepts the row's grab mid-drag, so
/// the drop never commits. The list around the rows opts out, and so does the
/// queue sheet's. Nothing else catches a regression here — it compiles,
/// reads clean, and only misbehaves on a list long enough to scroll.
///
/// The value was `!reorder-enabled`, which reads as leaving a `true` default alone
/// and is not one: every style but Material publishes this off, so that spelling
/// *enabled* the pan on every sort that retires the drag.
#[test]
fn every_draggable_list_opts_out_of_drag_panning() {
    // The binding rather than the token: the property reads as an *enable*, so
    // `mouse-drag-pan-enabled: true` is the likeliest wrong edit and a bare
    // occurrence count can't fail on it.
    let list = code_tokens(DRAGGABLE_LIST);
    assert_eq!(
        list.matches("mouse-drag-pan-enabled: false").count(),
        1,
        "`inner-list` must opt out, the only scroller the rows sit in"
    );
    assert!(
        code_tokens(QUEUE_SHEET).contains("mouse-drag-pan-enabled: false"),
        "every row in the queue sheet is draggable, so its ListView never gets the gesture"
    );
}

/// All four terms, since each fails differently and only the first is obvious.
/// `sort_playlist_tracks` reverses on `"desc"`, and the drag writes display
/// indices straight into `position_order`, so a reversed position sort lands
/// every drop at the mirrored slot; a filter makes those indices name a
/// different track entirely; and a smart playlist has no curated order to
/// reorder into. Dropping one of the last three costs no wrong write —
/// `apply_optimistic_reorder` refuses the filtered case and the query the empty
/// item set — but the drag then starts, paints a ghost and a drop line, and does
/// nothing on release, which is worse than a list that never armed.
#[test]
fn the_reorder_gate_reads_every_term_the_drag_depends_on() {
    let src = code_tokens(DETAIL_VIEW);
    assert!(
        src.contains(
            "reorder-enabled: PlaylistDetail.sort-field == \"position\" \
             && PlaylistDetail.sort-dir != \"desc\" \
             && PlaylistDetail.filter == \"\" \
             && !PlaylistDetail.playlist.is_smart"
        ),
        "playlist-detail.slint must gate reorder on an *ascending* position sort, \
         an unfiltered list and a playlist with a curated order"
    );
}

/// `"position"` is what drag-to-reorder is gated on and no header cell asks for
/// it, so the sort cycle is the only way back. Drop the natural field and the
/// first click on any column retires reordering for the whole install —
/// persisted, so it survives a restart.
///
/// Two halves in two files: the shared wiring has to hand the view's natural
/// order to the cycle, and this view has to name one.
#[test]
fn the_sort_cycle_still_offers_a_way_back_to_the_curated_order() {
    assert!(
        code_tokens(SHARED_DETAIL_WIRING)
            .contains("next_sort_with_natural(&field, &dir, &clicked, V::NATURAL_SORT)"),
        "the plain `next_sort` has two states and cannot reach `\"position\"`"
    );
    assert!(
        code_tokens(DETAIL)
            .contains("const NATURAL_SORT: Option<&'static str> = Some(POSITION_FIELD);"),
        "the cycle needs the curated order named as its third state"
    );
}

/// `is_manual_order` answers two of `reorder-enabled`'s terms; the filter is the
/// third the drag depends on, and the one that fails silently — filtered,
/// `tracks` is a subset of `position_order`, so a display index is still in
/// range and the DB write lands on a different track than the one dragged.
///
/// Scoped to the guard rather than searched for over the file, so a later site
/// reading the same field elsewhere in `detail.rs` can't satisfy the assertion
/// while the guard itself has stopped asking. Each needle carries its operator
/// for the same reason the drag-pan pin above matches a binding — a dropped `!`
/// or an `&&` in place of the `||` inverts the guard, and a bare token match
/// can't see either.
#[test]
fn the_optimistic_reorder_refuses_a_filtered_list() {
    let src = code_tokens(DETAIL);
    let body = src.split_once("pub fn apply_optimistic_reorder").map_or("", |(_, rest)| rest);
    let guard = body.split_once("let saved =").map_or("", |(head, _)| head);
    assert!(
        guard.contains("!is_manual_order(&field, &dir)"),
        "the order half must be asked before the display indices reach `position_order`"
    );
    assert!(
        guard.contains("|| playlists_ui.detail.cache.is_filtered()"),
        "so must the filter half — a filtered index is in range, so the write lands"
    );
}

#[test]
fn only_an_ascending_position_sort_is_the_manual_order() {
    assert!(is_manual_order(POSITION_FIELD, "asc"));
    assert!(!is_manual_order(POSITION_FIELD, "desc"));
    assert!(!is_manual_order("title", "asc"));
    assert!(!is_manual_order("title", "desc"));
    // `SortDir::from_token`'s rule — only `"desc"` is descending.
    assert!(is_manual_order(POSITION_FIELD, "sideways"));
}

/// The grid's edit affordances are local callbacks; the host is what turns them into the
/// dialogs the CRUD dispatcher commits from. A stub here leaves the button visible and inert.
#[test]
fn playlist_grid_edit_affordances_reach_the_dialogs() {
    let tab = code_tokens(PLAYLISTS_TAB);
    assert!(tab.contains("delete-playlist(id) => { Dialog.open-delete-playlist(id); }"));
    let rename = concat!(
        "rename-playlist(id) => { Dialog.open-rename-playlist(id, Playlists.row-name(id), ",
        "Playlists.row-description(id)); }"
    );
    assert!(tab.contains(rename));
    assert!(tab.contains("delete-playlists(ids) => { Dialog.open-delete-playlists(ids); }"));
}

/// The grid paints one category at a time, so the page's category is the whole ownership
/// answer. A per-row flag would have to be threaded through the model for a distinction the
/// fetch already made; category 1 is the only arm `library_playlists_page` answers with the
/// account's own playlists.
#[test]
fn playlists_grid_is_editable_only_from_the_created_category() {
    let tab = code_tokens(PLAYLISTS_TAB);
    assert!(tab.contains("editable: Playlists.category == 1;"));
    assert!(!tab.contains("editable: false"), "the grid must not be read-only as a whole");
}

/// The grid's rows are `NetEase` playlists, so a signed-in account must take the write, not the
/// local library the callbacks were built against. The local arms stay for the signed-out
/// authoring path.
#[test]
fn playlist_crud_targets_the_cloud_when_signed_in() {
    let crud = code_tokens(CRUD_CALLBACKS);
    assert!(crud.contains("state.cloud.status().account.is_some()"));
    assert_eq!(crud.matches("let cloud = signed_in(").count(), 3, "create, rename and delete");
    assert!(crud.contains("state.cloud.create_playlist(&name)"));
    assert!(crud.contains("state.cloud.rename_playlist(id, &name)"));
    assert!(crud.contains("state.cloud.update_playlist_description("));
    assert!(crud.contains("s.cloud.delete_playlists(&ids)"));
    // A write refreshes the grid, and the detail only where it is one of the changed ids.
    assert_eq!(
        crud.matches("refresh_after_edit(").count(),
        4,
        "create, cloud rename, local rename and tags"
    );
    assert!(crud.contains("playlists_ui_mod::fetch_grid(&s, &pu, weak)"));
    // The local path is still there, byte for byte.
    assert!(crud.contains("library::playlists::create_playlist("));
    assert!(crud.contains("library::playlists::update_playlist("));
    assert!(crud.contains("library::playlists::delete_playlists("));
}

/// Delete closes the open detail through the one helper both stores use, and does it before the
/// write; a second copy beside the cloud branch is how the two paths would come to disagree
/// about which ids the view had to leave.
#[test]
fn cloud_delete_closes_the_detail_through_the_shared_helper() {
    let crud = code_tokens(CRUD_CALLBACKS);
    let delete = crud.split_once("fn spawn_delete").map_or("", |(_, rest)| rest);
    let delete = delete.split_once("fn close_detail_if_among").map_or(delete, |(head, _)| head);
    assert_eq!(delete.matches("close_detail_if_among(").count(), 1);
    let close = delete.find("close_detail_if_among(").unwrap_or(usize::MAX);
    let write = delete.find("s.cloud.delete_playlists(&ids)").unwrap_or(usize::MAX);
    assert!(close < write, "the detail must close before the cloud write can fail");
}

/// `pending_track_ids` are SQLite row ids. Sending them to `NetEase` would name other songs
/// entirely, so the cloud arm creates the playlist without them and says so in the log.
#[test]
fn cloud_create_does_not_send_local_track_ids() {
    let crud = code_tokens(CRUD_CALLBACKS);
    let create = crud.split_once("async fn cloud_create").map_or("", |(_, rest)| rest);
    let create = create.split_once("async fn cloud_rename").map_or(create, |(head, _)| head);
    assert!(!create.contains("add_playlist_tracks"));
    assert!(create.contains("skipped {} local track ids"));
    assert!(create.contains("pending.len()"));
}

/// A user-initiated cloud write that fails has no other way to reach the user: the grid refetch
/// shows what survived, not what was refused. The log line alone would only be seen by someone
/// already reading a bug report.
#[test]
fn cloud_write_failures_are_toasted_and_logged() {
    let crud = code_tokens(CRUD_CALLBACKS);
    let report = crud.split_once("fn report_failure").map_or("", |(_, rest)| rest);
    let report = report.split_once("async fn cloud_create").map_or(report, |(head, _)| head);
    assert!(report.contains("describe(error)"));
    assert!(report.contains("log::warn!(\"{label}: {detail}\")"));
    assert!(report.contains("toast::notify(ToastKind::OperationFailed, detail)"));
}

/// Both edits cross Slint as decimal text, and both land back through the one detail refresh.
#[test]
fn cloud_playlist_track_edits_use_remote_ids_and_refresh_the_detail() {
    let menu = code_tokens(CLOUD_MENU);
    assert!(menu.contains("Playlists.request-add-cloud-tracks([root.row-data.remote_id], -1)"));
    assert!(menu.contains("label: @tr(\"Remove from Playlist\")"));
    let row = code_tokens(TRACK_ROW);
    assert!(
        row.contains("remove-from-playlist => { root.remove-cloud-row(root.row-data.remote_id); }")
    );
    assert!(code_tokens(TRACK_LIST).contains("callback remove-cloud-row(string"));
    let cloud_tracks = code_tokens(CLOUD_TRACKS);
    assert!(cloud_tracks.contains("cloud-playlist-editable: root.owned;"));
    assert!(cloud_tracks.contains("remove-cloud-row(id) => { root.remove-track(id); }"));
    assert!(
        code_tokens(DETAIL_VIEW)
            .contains("remove-cloud-row(id) => { PlaylistDetail.remove-cloud-track(id); }")
    );

    let picker = code_tokens(ADD_PICKER_CALLBACKS);
    assert!(picker.contains("library_playlists_page(MusicLibraryCategory::Created, 0)"));
    assert!(picker.contains("remote_id: SharedString::from(playlist.id.to_string())"));
    assert!(picker.contains("dlg.set_pending_remote_ids("));
    assert!(picker.contains("state.cloud.add_playlist_tracks(*pid, track_ids)"));

    let detail = code_tokens(DETAIL_CALLBACKS);
    assert!(detail.contains("s.cloud.remove_playlist_tracks(playlist_id, &[song_id])"));
    assert!(detail.contains("playlists_ui_mod::refresh_detail("));
    assert!(detail.contains("toast::notify(ToastKind::OperationFailed, detail)"));
    assert!(code_tokens(DETAIL).contains("set_cloud_owned(owned)"));
}

/// The explicit moves are the only reorder a cloud row gets, so both ends and an absent song
/// have to be no-ops rather than a splice that lands somewhere a user cannot see.
#[test]
fn cloud_playlist_move_clamps_at_both_ends() {
    assert_eq!(moved_order(&[1, 2, 3], 2, -1), Some(vec![2, 1, 3]));
    assert_eq!(moved_order(&[1, 2, 3], 2, 1), Some(vec![1, 3, 2]));
    assert_eq!(moved_order(&[1, 2, 3], 1, -1), None, "already first");
    assert_eq!(moved_order(&[1, 2, 3], 3, 1), None, "already last");
    assert_eq!(moved_order(&[1, 2, 3], 9, 1), None, "song is not in the playlist");
    assert_eq!(moved_order(&[7], 7, 1), None, "one song has nowhere to go");
}

/// Ownership gates the entries at the row, and the remote id is what crosses — a local row id
/// names nothing on `NetEase`.
#[test]
fn cloud_playlist_moves_are_owned_only_and_cross_the_remote_id() {
    let menu = code_tokens(CLOUD_MENU);
    assert!(menu.contains("in property <bool> show-playlist-reorder: false;"));
    assert!(menu.contains("label: @tr(\"Move up\")"));
    assert!(menu.contains("label: @tr(\"Move down\")"));

    let row = code_tokens(TRACK_ROW);
    assert!(row.contains("callback move-cloud-row(string"));
    assert!(row.contains("show-playlist-reorder: root.cloud-playlist-reorderable;"));
    assert!(row.contains("move-up => { root.move-cloud-row(root.row-data.remote_id, -1); }"));
    assert!(row.contains("move-down => { root.move-cloud-row(root.row-data.remote_id, 1); }"));

    let list = code_tokens(TRACK_LIST);
    assert!(list.contains("callback move-cloud-row(string"));
    assert!(list.contains("move-cloud-row(id, delta) => { root.move-cloud-row(id, delta); }"));

    let tracks = code_tokens(CLOUD_TRACKS);
    assert!(tracks.contains("callback move-track(string, int);"));
    assert!(tracks.contains("cloud-playlist-reorderable: root.owned;"));
    assert!(tracks.contains("move-cloud-row(id, delta) => { root.move-track(id, delta); }"));

    assert!(
        code_tokens(DETAIL_VIEW).contains(
            "move-cloud-row(id, delta) => { PlaylistDetail.move-cloud-track(id, delta); }"
        )
    );
    assert!(code_tokens(PLAYLIST_DETAIL_GLOBAL).contains("callback move-cloud-track(string"));
}

/// A filtered or re-sorted list names other neighbours, so the same `is_manual_order` rule the
/// local drag uses has to guard the service write; a failed write puts the captured order back.
#[test]
fn cloud_playlist_move_refuses_a_display_order_that_is_not_the_playlists_own() {
    let order = code_tokens(DETAIL_ORDER);
    let handler = order.split_once("on_move_cloud_track").map_or("", |(_, rest)| rest);
    assert!(handler.contains("g.get_cloud_owned()"));
    assert!(handler.contains("!is_manual_order(&field, &dir)"));
    assert!(handler.contains("playlists_ui.detail.cache.is_filtered()"));
    assert!(handler.contains("state.cloud.reorder_playlist_tracks(playlist_id, &next)"));
    assert!(handler.contains("restore(&playlists_ui, playlist_id, Some(previous))"));
    assert!(handler.contains("refresh_detail(&state, &playlists_ui, weak, playlist_id)"));
    assert!(handler.contains("toast::notify(ToastKind::OperationFailed, detail)"));
}
