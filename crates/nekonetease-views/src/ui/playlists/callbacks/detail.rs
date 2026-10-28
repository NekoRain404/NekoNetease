//! `PlaylistDetail.*` callbacks: close-detail, drag-reorder, the edit-artwork picker open and
//! track removal here, the rest through [`crate::ui::callbacks::track_detail`].

use std::path::PathBuf;
use std::sync::Arc;

use async_compat::Compat;
use slint::{ComponentHandle, Global as _, Model, ModelExt};

use super::artwork::open_edit_artwork;
use crate::ui::callbacks::track_detail;
use crate::ui::callbacks::{DialogClaim, collect_track_ids};
use crate::ui::file_dialog;
use crate::ui::playlists::{self as playlists_ui_mod, PlaylistsUi, detail_order};
use nekonetease_app::cloud_playback::CloudQueueShuffle;
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_ui::{AppWindow, PlayerVm, PlaylistDetail, PlaylistPlaybackAction, TrackListRow};

/// Wire the `PlaylistDetail` callbacks. See [`super::wire`].
pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    track_detail::wire(&ui.global::<PlaylistDetail>().as_weak(), state, playlists_ui);
    wire_cloud_playback(ui, state, playlists_ui);
    wire_close(ui, state, playlists_ui);
    wire_reorder(ui, state, playlists_ui);
    wire_edit_artwork(ui, state, playlists_ui);
    wire_remove_track(ui, state, playlists_ui);
    wire_remove_cloud_track(ui, state, playlists_ui);
    detail_order::wire(ui, state, playlists_ui);
    wire_cloud_subscribe(ui, state, playlists_ui);
    super::comments::wire(ui, state, playlists_ui);
}

/// Cloud playlist rows do not exist in `SQLite`, so activation, Play All and Shuffle seat the
/// queue from the rows the detail is already showing — the same songs `song/detail` would return —
/// which leaves only the stream ticket between the click and the first sound.
fn wire_cloud_playback(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    ui.global::<PlaylistDetail>().on_resolve_playback_action(|player, rows, cloud, ready| {
        playback_action(&player, &rows, cloud, ready)
    });
    let s = state.clone();
    let pu = playlists_ui.clone();
    ui.global::<PlaylistDetail>().on_play_cloud_row(move |remote_id, _index| {
        let Some(song_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            log::warn!("playlists::play_cloud_song: invalid NetEase song ID");
            return;
        };
        play_cloud_track(&s, &pu, song_id, CloudQueueShuffle::Preserve);
    });

    // Shuffle All: one row at random, shuffle forced on, and the pill row follows the choice it
    // just made so the next Play pressing the same button starts the same way.
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_shuffle_all(move || {
        use rand::RngExt;
        let rows = pu.detail.cache.displayed_rows();
        if rows.is_empty() {
            return;
        }
        let first = rows[rand::rng().random_range(..rows.len())].id;
        set_play_mode(&weak, PLAY_MODE_SHUFFLE);
        play_cloud_track(&s, &pu, first, CloudQueueShuffle::Shuffle);
    });

    // Play All: the list in its own order, from the first row.
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_play_all(move || {
        let Some(first) = pu.detail.cache.displayed_rows().first().map(|row| row.id) else {
            return;
        };
        set_play_mode(&weak, PLAY_MODE_ORDERED);
        play_cloud_track(&s, &pu, first, CloudQueueShuffle::Ordered);
    });

    // The one button that has to know what "play" means right now: with one of this playlist's
    // rows already playing it toggles the transport, and otherwise it starts the list the way the
    // pill row is set. Stopped is the third case — a restored cloud queue with no source.
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_toggle_playback(move || {
        let Some(ui) = weak.upgrade() else { return };
        let action = ui.global::<PlaylistDetail>().get_playback_action();
        if action == PlaylistPlaybackAction::Unavailable {
            return;
        }
        if matches!(action, PlaylistPlaybackAction::Pause | PlaylistPlaybackAction::Resume) {
            let state = s.clone();
            s.runtime.spawn(async move {
                if let Err(error) =
                    library::playback::player_toggle_play_pause(&state.playback_ctx())
                {
                    log::warn!("playlists::toggle_playback: {}", describe(&error));
                }
            });
            return;
        }
        let shuffle = ui.global::<PlaylistDetail>().get_play_mode() == PLAY_MODE_SHUFFLE;
        let rows = pu.detail.cache.displayed_rows();
        let Some(first) = rows.first().map(|row| row.id) else { return };
        let first = if shuffle {
            use rand::RngExt;
            rows[rand::rng().random_range(..rows.len())].id
        } else {
            first
        };
        let mode = if shuffle { CloudQueueShuffle::Shuffle } else { CloudQueueShuffle::Ordered };
        play_cloud_track(&s, &pu, first, mode);
    });
}

fn playback_action(
    player: &PlayerVm,
    rows: &slint::ModelRc<TrackListRow>,
    cloud: bool,
    ready: bool,
) -> PlaylistPlaybackAction {
    rows.model_tracker().track_row_count_changes();
    if !ready || rows.row_count() == 0 {
        return PlaylistPlaybackAction::Unavailable;
    }
    let owns_source = player.has_track
        && player.track.file_path.is_empty() == cloud
        && (0..rows.row_count()).any(|index| {
            rows.row_data_tracked(index).is_some_and(|row| row.remote_id == player.track.remote_id)
        });
    match (owns_source, player.status.as_str()) {
        (true, "playing") => PlaylistPlaybackAction::Pause,
        (true, "paused") => PlaylistPlaybackAction::Resume,
        _ => PlaylistPlaybackAction::Start,
    }
}

/// Which way the detail's Play / Shuffle pills fill the queue. Slint shows the choice; this is the
/// spelling both sides compare, so a typo can't quietly make them disagree.
const PLAY_MODE_ORDERED: &str = "ordered";
const PLAY_MODE_SHUFFLE: &str = "shuffle";

fn set_play_mode(weak: &slint::Weak<AppWindow>, mode: &str) {
    if let Some(ui) = weak.upgrade() {
        ui.global::<PlaylistDetail>().set_play_mode(slint::SharedString::from(mode));
    }
}

fn play_cloud_track(
    state: &AppState,
    playlists_ui: &PlaylistsUi,
    song_id: i64,
    shuffle: CloudQueueShuffle,
) {
    if song_id <= 0 {
        return;
    }
    let tracks = super::super::track_summaries(&playlists_ui.detail.cache.displayed_rows());
    let state = state.clone();
    let runtime = state.runtime.clone();
    runtime.spawn(async move {
        let (volume, speed) = {
            let player = lock_state(&state.player_state);
            (player.effective_volume(), player.playback_speed)
        };
        if let Err(error) =
            state.cloud_playback.play_queue_tracks(tracks, song_id, shuffle, volume, speed).await
        {
            log::warn!("playlists::play_cloud_song({song_id}): {}", describe(&error));
        }
    });
}

#[cfg(test)]
#[path = "tests/detail_tests.rs"]
mod tests;

/// A cloud playlist row's removal. The remote id is parsed before the write, and the refresh
/// preserves the current sort while pruning the removed id from the selection.
fn wire_remove_cloud_track(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_remove_cloud_track(move |remote_id| {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.global::<PlaylistDetail>().get_cloud_owned() {
            return;
        }
        let Some(song_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            log::warn!("playlists::remove_cloud_track: invalid NetEase song ID");
            return;
        };
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 {
            return;
        }
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            if let Err(e) = s.cloud.remove_playlist_tracks(playlist_id, &[song_id]).await {
                let detail = describe(&e);
                log::warn!("playlists::remove_cloud_track({playlist_id}): {detail}");
                toast::notify(ToastKind::OperationFailed, detail);
                return;
            }
            if let Err(e) =
                playlists_ui_mod::refresh_detail(&s, &pu, weak.clone(), playlist_id).await
            {
                log::warn!("playlists::remove_cloud_track refresh: {}", describe(&e));
            }
            if let Err(e) = playlists_ui_mod::fetch_grid(&s, &pu, weak).await {
                log::warn!("playlists::remove_cloud_track refetch grid: {}", describe(&e));
            }
        });
    });
}

/// Collecting another account's playlist is a write, so the button reports its own failure and
/// the optimistic state is only committed after the server acknowledges it.
fn wire_cloud_subscribe(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_toggle_cloud_subscribe(move || {
        let Some(ui) = weak.upgrade() else { return };
        // The exact 64-bit id: `playlist-id` is the bounded legacy handle.
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 {
            return;
        }
        let next = !ui.global::<PlaylistDetail>().get_cloud_subscribed();
        ui.global::<PlaylistDetail>().set_cloud_subscribe_busy(true);
        let s = s.clone();
        let weak = weak.clone();
        let runtime = s.runtime.clone();
        runtime.spawn(async move {
            let result = s.cloud.subscribe_playlist(playlist_id, next).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<PlaylistDetail>();
                g.set_cloud_subscribe_busy(false);
                match result {
                    Ok(()) => g.set_cloud_subscribed(next),
                    Err(error) => {
                        let detail = describe(&error);
                        log::warn!("playlists::toggle_subscribe({playlist_id}): {detail}");
                        toast::notify(ToastKind::OperationFailed, detail);
                    }
                }
            });
        });
    });
}

/// The header's back button.
///
/// No `mark_drill_back`, where its three siblings have one: theirs precedes a `return_to_section`
/// that flips the nav index, and this detail records no origin at all. The body that mounts in
/// its place takes a fixed `below` and reads the global not at all; see `ui::nav_transition`.
fn wire_close(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_close_detail(move || {
        let Some(ui) = weak.upgrade() else { return };
        pu.detail_request.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        ui.global::<PlaylistDetail>().set_cloud_collection(false);
        ui.global::<PlaylistDetail>().set_cloud_subscribed(false);
        ui.global::<PlaylistDetail>().set_cloud_subscribe_busy(false);
        ui.global::<PlaylistDetail>().set_cloud_tags(slint::SharedString::default());
        ui.global::<PlaylistDetail>().set_cloud_owned(false);
        ui.global::<PlaylistDetail>().set_comments(slint::ModelRc::default());
        ui.global::<PlaylistDetail>().set_comments_loading(false);
        ui.global::<PlaylistDetail>().set_comments_error(slint::SharedString::default());
        // A close can land on a header whose rows never arrived, and the next open repaints this
        // anyway — leaving it set would make the grid's next owner show the loading line.
        ui.global::<PlaylistDetail>().set_loading(false);
        track_detail::forget_closed(&ui, &s, &*pu, &ui.global::<PlaylistDetail>());
        super::sync::forget_panel(&ui.global::<PlaylistDetail>(), &pu);

        let pu = pu.clone();
        s.runtime.spawn_blocking(move || {
            pu.release_detail_artwork();
            pu.prewarm_visible_covers();
        });
    });
}

/// A row drag released. Optimistically permutes the visible rows and the canonical order, then
/// writes; a failed write rolls both back.
///
/// A cloud playlist's order is remote, so it writes through `reorder_playlist_tracks` and a
/// rollback owns the explicit-move cache beside `position_order`. `apply_drag` re-asks ownership:
/// a collected playlist is read-only.
fn wire_reorder(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_reorder(move |from, to| {
        let Some(ui) = weak.upgrade() else { return };
        let Ok(from_u) = usize::try_from(from) else {
            return;
        };
        let Ok(to_u) = usize::try_from(to) else {
            return;
        };
        if from_u == to_u {
            return;
        }
        if ui.global::<PlaylistDetail>().get_cloud_collection() {
            reorder_cloud(&ui, &s, &pu, from_u, to_u);
            return;
        }
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 {
            return;
        }
        let Some(previous_order) =
            playlists_ui_mod::apply_optimistic_reorder(&ui, &pu, from_u, to_u)
        else {
            return;
        };
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            if let Err(e) = library::playlists::reorder_playlist(&s.db, playlist_id, from, to).await
            {
                log::warn!("playlists::reorder: {}", describe(&e));
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    playlists_ui_mod::rollback_reorder(&ui, &pu, previous_order);
                });
            }
        });
    });
}

fn reorder_cloud(
    ui: &AppWindow,
    state: &AppState,
    playlists_ui: &Arc<PlaylistsUi>,
    from: usize,
    to: usize,
) {
    let Some(commit) = detail_order::apply_drag(ui, playlists_ui, from, to) else {
        return;
    };
    let playlist_id = commit.playlist_id;
    let next = commit.next.clone();
    let weak = ui.as_weak();
    let s = state.clone();
    let pu = playlists_ui.clone();
    s.runtime.clone().spawn(async move {
        if let Err(error) = s.cloud.reorder_playlist_tracks(playlist_id, &next).await {
            let detail = describe(&error);
            log::warn!("playlists::reorder_cloud({playlist_id}): {detail}");
            let _ = weak.upgrade_in_event_loop(move |ui| {
                detail_order::rollback_drag(&ui, &pu, commit);
                toast::notify(ToastKind::OperationFailed, detail);
            });
            return;
        }
        if let Err(error) = playlists_ui_mod::refresh_detail(&s, &pu, weak, playlist_id).await {
            log::warn!("playlists::reorder_cloud refresh: {}", describe(&error));
        }
    });
}

/// Opens the Edit Artwork picker seeding `current-artwork` from `PlaylistDetail.cover`, already
/// decoded for the open detail, so the dialog opens on a preview of the saved state.
///
/// A cloud playlist the account owns takes the native picker instead: its cover is a remote write,
/// so it never reaches the local playlist thumbnail store the dialog commits to.
///
/// Rename and Delete need no handler here: their opens are populated inline in
/// `my-library/tab-pills.slint`, and their commits are `super::crud`'s.
fn wire_edit_artwork(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_request_edit_artwork(move || {
        let id = pu.detail_playlist_id();
        if id < 0 {
            return;
        }
        let Some((is_cloud, is_owned)) = weak.upgrade().map(|ui| {
            let detail = ui.global::<PlaylistDetail>();
            (detail.get_cloud_collection(), detail.get_cloud_owned())
        }) else {
            return;
        };
        if is_cloud {
            if !is_owned {
                return;
            }
            let s = s.clone();
            let pu = pu.clone();
            let weak = weak.clone();
            let _ = slint::spawn_local(Compat::new(async move {
                let dialog = file_dialog::parented(&weak, "Choose Cover Image")
                    .add_filter("Images", &["jpg", "jpeg", "png", "webp"]);
                let Some(handle) = dialog.pick_file().await else { return };
                upload_cloud_cover(&s, &pu, &weak, id, handle.path().to_path_buf());
            }));
            return;
        }
        let Some(claim) = DialogClaim::take_from(&weak) else { return };
        open_edit_artwork(&s, &weak, claim, id, |ui| ui.global::<PlaylistDetail>().get_cover());
    });
}

/// The remote half of the cover picker. The failure is reported where the user asked for the
/// write; a success is the refreshed header, since there is no generic success toast kind.
fn upload_cloud_cover(
    state: &AppState,
    playlists_ui: &Arc<PlaylistsUi>,
    weak: &slint::Weak<AppWindow>,
    id: i64,
    path: PathBuf,
) {
    let state = state.clone();
    let playlists_ui = playlists_ui.clone();
    let weak = weak.clone();
    state.runtime.clone().spawn(async move {
        match state.cloud.upload_playlist_cover(id, &path).await {
            Ok(()) => {
                super::refresh_after_edit(&state, &playlists_ui, weak, &[id], "playlists::cover")
                    .await;
            }
            Err(error) => {
                let detail = describe(&error);
                log::warn!("playlists::cover({id}): {detail}");
                toast::notify(ToastKind::OperationFailed, detail);
            }
        }
    });
}

/// The row menu's removal. One row and a whole selection both arrive as `[int]` and collapse to
/// one batched write; `refresh_detail` prunes the removed ids out of the selection on the way
/// back.
fn wire_remove_track(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_remove_track(move |track_ids| {
        let id_vec = collect_track_ids(&track_ids);
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 || id_vec.is_empty() {
            return;
        }
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            if let Err(e) =
                library::playlists::remove_tracks_from_playlist_batch(&s.db, playlist_id, id_vec)
                    .await
            {
                log::warn!("playlists::remove_track: {}", describe(&e));
                return;
            }
            if let Err(e) =
                playlists_ui_mod::refresh_detail(&s, &pu, weak.clone(), playlist_id).await
            {
                log::warn!("playlists::remove_track refresh: {}", describe(&e));
            }
            if let Err(e) = playlists_ui_mod::fetch_grid(&s, &pu, weak).await {
                log::warn!("playlists::remove_track refetch grid: {}", describe(&e));
            }
        });
    });
}
