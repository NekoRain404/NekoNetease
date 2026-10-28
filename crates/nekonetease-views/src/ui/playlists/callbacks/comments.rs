//! The playlist detail's comment preview: its retry, a row's heart, and the hand-off to the full
//! page. The preview is capped in Rust, so the band owns no paging here.

use std::sync::Arc;

use slint::ComponentHandle;

use crate::ui::detail_comments::{apply_like_toggle, comment_wire_id, rollback_like_toggle};
use crate::ui::playlists::{self as playlists_ui_mod, PlaylistsUi};
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::CommentThread;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, PlaylistDetail};

pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    wire_open(ui, state, playlists_ui);
    wire_comments_retry(ui, state, playlists_ui);
    wire_comment_like(ui, state, playlists_ui);
}

/// The whole thread for the open cloud playlist, over the preview the band shows.
fn wire_open(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_open_comments(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<PlaylistDetail>();
        let playlist_id = pu.detail_playlist_id();
        if !detail.get_cloud_collection() || playlist_id <= 0 {
            return;
        }
        let title = detail.get_playlist().name;
        crate::ui::comments::open(&ui, &s, CommentThread::Playlist(playlist_id), title.as_str());
    });
}

/// Comments load beside the tracks, so their failure has its own retry rather than failing the
/// playlist open. The current request counter rides along, keeping an older retry from landing on
/// a playlist opened since.
fn wire_comments_retry(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_retry_comments(move || {
        if weak.upgrade().is_none() {
            return;
        }
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 {
            return;
        }
        let request = pu.detail_request.load(std::sync::atomic::Ordering::Acquire);
        let account_generation = s.cloud.status().generation;
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        let runtime = s.runtime.clone();
        runtime.spawn(async move {
            playlists_ui_mod::load_playlist_comments(
                &s,
                &pu,
                &weak,
                playlist_id,
                request,
                account_generation,
            )
            .await;
        });
    });
}

/// A comment heart flips before the request and rolls back on failure, the write being small
/// enough that waiting for the round trip would read as an unresponsive control.
fn wire_comment_like(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_toggle_comment_like(move |index, comment_id, next| {
        let Ok(index) = usize::try_from(index) else { return };
        let playlist_id = pu.detail_playlist_id();
        if playlist_id <= 0 {
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        let comments = ui.global::<PlaylistDetail>().get_comments();
        let Some(wire_id) = comment_wire_id(&comments, index, comment_id) else { return };
        let Some(snapshot) = apply_like_toggle(&comments, index, comment_id, next) else {
            return;
        };
        let request = pu.detail_request.load(std::sync::atomic::Ordering::Acquire);
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            if let Err(error) =
                s.cloud.like_comment(CommentThread::Playlist(playlist_id), wire_id, next).await
            {
                let detail = describe(&error);
                log::warn!("playlists::toggle_comment_like({comment_id}): {detail}");
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    if pu.detail_request.load(std::sync::atomic::Ordering::Acquire) != request {
                        return;
                    }
                    rollback_like_toggle(&ui.global::<PlaylistDetail>().get_comments(), &snapshot);
                    toast::notify(ToastKind::OperationFailed, detail);
                });
            }
        });
    });
}
