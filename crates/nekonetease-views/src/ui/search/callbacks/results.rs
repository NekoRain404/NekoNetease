//! `Search.*` result callbacks: cross-tab open-album / open-artist /
//! open-chart hand-offs, Top Result routing, `TrackList` row actions (play,
//! queue, favorite toggle), sort, column visibility, and selection. See
//! [`super::wire`].

use std::sync::Arc;

use slint::{ComponentHandle, Global as _, SharedString};

use super::NAV_SEARCH;
use crate::ui::albums::AlbumsUi;
use crate::ui::artists::ArtistsUi;
use crate::ui::callbacks::cross_tab_nav;
use crate::ui::callbacks::macros::wire_row_flag;
use crate::ui::callbacks::track_list::{play_displayed, wire_queue_actions, wire_toggle_column};
use crate::ui::callbacks::{collect_track_ids, model_track_ids, next_sort, persist_view_sort};
use crate::ui::search::{self as search_ui_mod, SearchUi, apply, fetch};
use crate::ui::track_list_view::view_id;
use nekonetease_app::library;
use nekonetease_app::services::settings::ViewSort;
use nekonetease_app::state::AppState;
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_ui::{AppWindow, MyLibrary, PlaylistDetail, Playlists, Search};

/// Wire the cross-tab open / Top Result / row-action / sort / selection
/// callbacks.
pub(super) fn wire(
    ui: &AppWindow,
    state: &AppState,
    search_ui: &Arc<SearchUi>,
    albums_ui: &Arc<AlbumsUi>,
    artists_ui: &Arc<ArtistsUi>,
) {
    let g = ui.global::<Search>();
    wire_open_album(ui, state, albums_ui);
    wire_open_artist(ui, state, artists_ui);
    wire_open_chart(ui);
    wire_open_top_result(ui);
    wire_open_top_result_remote(ui, state, albums_ui);
    wire_play_row(ui, state);
    wire_cloud_rows(ui, state, search_ui);
    wire_queue_actions(&g, state, collect_track_ids);
    wire_row_flags(ui, state, search_ui);
    wire_sort(ui, state, search_ui);
    wire_toggle_column(&g.as_weak(), state);
    wire_selection(ui, search_ui);
}

fn wire_open_album(ui: &AppWindow, state: &AppState, albums_ui: &Arc<AlbumsUi>) {
    let s = state.clone();
    let au = albums_ui.clone();
    let weak = ui.as_weak();
    ui.global::<Search>().on_open_album(move |album_id| {
        cross_tab_nav::open_album_cross_tab(
            &s,
            &au,
            &weak,
            i64::from(album_id),
            cross_tab_nav::Origin::section(NAV_SEARCH),
            "search::open_album",
        );
    });
}

fn wire_open_artist(ui: &AppWindow, state: &AppState, artists_ui: &Arc<ArtistsUi>) {
    let s = state.clone();
    let aru = artists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<Search>().on_open_artist(move |artist_id| {
        cross_tab_nav::open_artist_cross_tab(
            &s,
            &aru,
            &weak,
            i64::from(artist_id),
            cross_tab_nav::Origin::section(NAV_SEARCH),
            "search::open_artist",
        );
    });
}

/// A chart is a playlist and Search has no playlist surface, so the row hands off to My Library's
/// Playlists tab, where the shared detail body mounts. The tab flips before the open because the
/// body is gated on that tab and `open_playlist_with` writes `playlist-id` a fetch later.
fn wire_open_chart(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<Search>().on_open_chart(move |remote_id| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            log::warn!("search::open_chart: invalid NetEase playlist id");
            return;
        };
        // A detail left open on an earlier visit would paint for the frame the nav moves.
        ui.global::<PlaylistDetail>().set_playlist_id(-1);
        crate::ui::my_library::go_to_tab(&ui, ui.global::<MyLibrary>().get_tab_playlists());
        ui.global::<Playlists>().invoke_open_playlist(SharedString::from(id.to_string()));
    });
}

/// Resolves to one of three cross-tab handlers based on `top-kind` —
/// the two above plus `go-to-genre`, wired in `cross_tab_nav`.
/// Invoking the global callback directly keeps the origin-stamp +
/// nav-flip + persist all in one place.
fn wire_open_top_result(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<Search>().on_open_top_result(move || {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<Search>();
        let kind = g.get_top_kind();
        let Some(id) = resolve_top_result_id(g.get_top_remote_id().as_str(), g.get_top_id()) else {
            return;
        };
        match kind.as_str() {
            "album" => g.invoke_open_album(i32::try_from(id).unwrap_or(i32::MAX)),
            "artist" => g.invoke_open_artist(i32::try_from(id).unwrap_or(i32::MAX)),
            "genre" => g.invoke_go_to_genre(i32::try_from(id).unwrap_or(i32::MAX)),
            _ => {}
        }
    });
}

fn wire_open_top_result_remote(ui: &AppWindow, state: &AppState, albums_ui: &Arc<AlbumsUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let au = albums_ui.clone();
    ui.global::<Search>().on_open_top_result_remote(move |kind, remote_id| {
        let Ok(id) = remote_id.as_str().parse::<i64>() else { return };
        let Some(_ui) = weak.upgrade() else { return };
        match kind.as_str() {
            "album" => cross_tab_nav::open_album_cross_tab(
                &s,
                &au,
                &weak,
                id,
                cross_tab_nav::Origin::section(NAV_SEARCH),
                "search::open_top_album",
            ),
            // A `NetEase` artist is a remote entity: the local artist detail would look its id up
            // in the library and answer `Artist not found` for every one of them.
            "artist" => {
                if let Some(ui) = weak.upgrade() {
                    crate::ui::callbacks::cloud_tracks::open_entity_detail(
                        &ui,
                        &s,
                        "artist",
                        id.to_string(),
                    );
                }
            }
            _ => {}
        }
    });
}

pub(crate) fn resolve_top_result_id(remote_id: &str, legacy_id: i32) -> Option<i64> {
    if !remote_id.trim().is_empty() {
        return remote_id.parse::<i64>().ok().filter(|id| *id >= 0);
    }
    (legacy_id >= 0).then_some(i64::from(legacy_id))
}

/// Loads the visible results into the queue and starts on the
/// clicked track. Search keeps no Rust-side cache of what's on screen (the
/// sort and the `show-all-tracks` cap are applied at render), so the ids
/// come off the live model.
fn wire_play_row(ui: &AppWindow, state: &AppState) {
    let s = state.clone();
    let weak = ui.as_weak();
    ui.global::<Search>().on_play_row(move |track_id, idx| {
        let Some(ui) = weak.upgrade() else { return };
        let ids = model_track_ids(&ui.global::<Search>().get_tracks());
        play_displayed(&s, view_id::SEARCH, ids, track_id, idx);
    });
}

/// Cloud search rows keep their exact `NetEase` IDs and start the current result set through the
/// buffered cloud playback path.
fn wire_cloud_rows(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let s = state.clone();
    let su = search_ui.clone();
    ui.global::<Search>().on_play_cloud_row(move |remote_id, _index| {
        let Some(song_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            return;
        };
        let ids = su
            .state()
            .cloud_song_rows
            .lock()
            .iter()
            .filter_map(|row| crate::ui::util::parse_cloud_id(row.remote_id.as_str()))
            .collect::<Vec<_>>();
        if ids.is_empty() || !ids.contains(&song_id) {
            return;
        }
        let s = s.clone();
        s.runtime.clone().spawn(async move {
            let (volume, speed) = {
                let player = lock_state(&s.player_state);
                (player.effective_volume(), player.playback_speed)
            };
            if let Err(error) =
                s.cloud_playback.play_queue(&ids, song_id, false, volume, speed).await
            {
                log::warn!(
                    "search::play_cloud_song({song_id}): {}",
                    nekonetease_core::error::describe(&error)
                );
            }
        });
    });

    let s = state.clone();
    let su = search_ui.clone();
    let weak = ui.as_weak();
    ui.global::<Search>().on_toggle_cloud_favorite(move |remote_id, liked| {
        let Some(song_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            return;
        };
        let s = s.clone();
        let su = su.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            let generation = s.cloud.status().generation;
            if let Err(error) = s.cloud.like(song_id, liked).await {
                log::warn!(
                    "search::set_cloud_favorite({song_id}): {}",
                    nekonetease_core::error::describe(&error)
                );
                return;
            }
            if s.cloud.status().generation != generation {
                return;
            }
            su.flip_favorite(song_id, liked);
            apply::apply_cloud_row_favorite(&weak, song_id, liked);
        });
    });
}

/// Write through, then patch the cached result set
/// and the visible row. Search has no `library_changed` subscriber to fall back on, so
/// without the patch the pip never moves and the next click recomputes the same `!false`
/// and re-sends a value the database already holds.
fn wire_row_flags(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();
    {
        let su = search_ui.clone();
        wire_row_flag!(g, on_toggle_row_favorite, state, "search::set_favorite",
        library::favorites::set_favorite, collect_track_ids,
        captures: [weak, su],
        after: |id_vec, fav| {
            for id in &id_vec {
                su.flip_favorite(*id, fav);
                apply::apply_row_favorite(&weak, *id, fav);
            }
        });
    }
}

fn wire_sort(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let s = state.clone();
    let su = search_ui.clone();
    let weak = ui.as_weak();
    ui.global::<Search>().on_request_sort(move |field| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<Search>();
        let (new_field, new_dir) =
            next_sort(g.get_sort_field().as_str(), g.get_sort_dir().as_str(), &field);
        g.set_sort_field(SharedString::from(new_field.as_str()));
        g.set_sort_dir(SharedString::from(new_dir.as_str()));
        *su.state().sort.lock() = ViewSort { field: new_field.clone(), dir: new_dir };
        persist_view_sort(&s, view_id::SEARCH, new_field, new_dir);

        // Re-derive the visible Songs slice from the cached
        // results — no DB hit. If there are no cached results
        // yet (no commit ran since startup) this is a no-op.
        fetch::reapply_cached_results(&su, &weak);
    });
}

fn wire_selection(ui: &AppWindow, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    {
        let weak = ui.as_weak();
        let su = search_ui.clone();
        g.on_select_row(move |idx, id, shift, ctrl| {
            let Some(ui) = weak.upgrade() else { return };
            search_ui_mod::handle_select_row(&ui, &su, idx, id, shift, ctrl);
        });
    }
    {
        let weak = ui.as_weak();
        let su = search_ui.clone();
        g.on_select_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            search_ui_mod::select_all(&ui, &su);
        });
    }
    {
        let weak = ui.as_weak();
        let su = search_ui.clone();
        g.on_clear_selection(move || {
            let Some(ui) = weak.upgrade() else { return };
            search_ui_mod::clear_selection(&ui, &su);
        });
    }
}
