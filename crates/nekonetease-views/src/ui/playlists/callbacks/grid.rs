//! `Playlists.*` grid callbacks: lazy cover lookup, re-chunk on column
//! count, client-side filter, the open-playlist drill-in, and the
//! per-id name / description lookups backing the grid card's Rename overlay.

use std::sync::Arc;

use slint::{ComponentHandle, SharedString};

use crate::ui::callbacks::macros::spawn_logged;
use crate::ui::playlists::{self as playlists_ui_mod, PlaylistsUi};
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Playlists};

/// Wire the `Playlists` grid callbacks. See [`super::wire`].
pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let playlists = ui.global::<Playlists>();
    let weak = ui.as_weak();

    // request-cover: lazy per-card cover lookup off the shared grid tier. Only on-screen cards
    // invoke this, and `generation` is read for its effect on the binding, never its value.
    playlists
        .on_request_cover(|path, _generation| crate::ui::grid_prewarm::grid_cover(path.as_str()));

    {
        let state = state.clone();
        let pu = playlists_ui.clone();
        let weak = weak.clone();
        playlists.on_retry_cloud(move || {
            let state = state.clone();
            let pu = pu.clone();
            let weak = weak.clone();
            let runtime = state.runtime.clone();
            runtime.spawn(async move {
                if let Err(error) = playlists_ui_mod::fetch_grid(&state, &pu, weak).await {
                    log::warn!("playlists::retry_cloud: {}", describe(&error));
                }
            });
        });
    }

    {
        let pu = playlists_ui.clone();
        let weak = weak.clone();
        playlists.on_columns_changed(move |_cols| {
            let Some(ui) = weak.upgrade() else { return };
            playlists_ui_mod::rebuild_grid(&ui, &pu);
        });
    }

    {
        let pu = playlists_ui.clone();
        let weak = weak.clone();
        playlists.on_apply_filter(move |_text| {
            let Some(ui) = weak.upgrade() else { return };
            playlists_ui_mod::rebuild_grid(&ui, &pu);
        });
    }

    {
        let s = state.clone();
        let pu = playlists_ui.clone();
        let weak = weak.clone();
        playlists.on_open_playlist(move |playlist_id| {
            let id = match playlist_id.to_string().parse::<i64>() {
                Ok(id) if id > 0 => id,
                _ => {
                    log::warn!("playlists::open_playlist: invalid NetEase playlist id");
                    return;
                }
            };

            let s_fetch = s.clone();
            let pu_fetch = pu.clone();
            let weak_fetch = weak.clone();
            spawn_logged!(
                s_fetch,
                "playlists::open_playlist",
                playlists_ui_mod::open_playlist(
                    &s_fetch,
                    &pu_fetch,
                    weak_fetch,
                    id,
                    nekonetease_ui::NavEnterFrom::Right
                )
            );

            s.runtime.spawn_blocking(crate::ui::grid_prewarm::hand_back_covers);

            let s_disk = s.clone();
            s.runtime.spawn_blocking(move || {
                if let Err(e) = library::settings::set_last_detail_id(
                    &s_disk.paths,
                    crate::ui::track_list_view::view_id::PLAYLIST_DETAIL,
                    Some(id),
                ) {
                    log::warn!("playlists::open_playlist persist: {}", describe(&e));
                }
            });
        });
    }

    // row-name / row-description: per-id lookups against the cached
    // grid `PlaylistStats` list — back the grid card's "Rename" overlay
    // which only carries the playlist id (the `PlaylistGridRow` doesn't
    // include description).
    {
        let pu = playlists_ui.clone();
        playlists.on_row_name(move |id| {
            pu.grid_stats_by_id(i64::from(id))
                .map(|p| SharedString::from(p.name.as_str()))
                .unwrap_or_default()
        });
    }
    {
        let pu = playlists_ui.clone();
        playlists.on_row_description(move |id| {
            pu.grid_stats_by_id(i64::from(id))
                .and_then(|p| p.description)
                .map(SharedString::from)
                .unwrap_or_default()
        });
    }
}
