//! Actions for the `NetEase` liked-songs list.

use std::sync::Arc;

use slint::{ComponentHandle, SharedString};

use crate::ui::callbacks::collect_track_ids;
use crate::ui::callbacks::{next_sort, persist_view_sort};
use crate::ui::favorites::{self as favorites_ui_mod, FavoritesTab, FavoritesUi};
use crate::ui::track_list_view::view_id;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_ui::{AppWindow, CloudTrackActions, Favorites};

pub(super) fn wire(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let g = ui.global::<Favorites>();
    let weak = ui.as_weak();

    wire_playback(ui, state, fav_ui);

    {
        let weak = weak.clone();
        g.on_toggle_cloud_favorite(move |remote_id, liked| {
            let Some(ui) = weak.upgrade() else { return };
            ui.global::<CloudTrackActions>().invoke_toggle_favorite(remote_id, liked);
        });
    }

    // `wire_playback` installs both activation paths through the same queue loader.

    {
        let state = state.clone();
        let runtime = state.runtime.clone();
        let cloud = state.cloud.clone();
        let fav_ui = fav_ui.clone();
        let weak = weak.clone();
        g.on_toggle_row_favorite(move |ids, liked| {
            let ids = collect_track_ids(&ids);
            if ids.is_empty() {
                return;
            }
            let state = state.clone();
            let cloud = cloud.clone();
            let fav_ui = fav_ui.clone();
            let weak = weak.clone();
            runtime.spawn(async move {
                // A failed write stops the batch. Likes are writes and must not be retried
                // automatically because the server may already have accepted one.
                for id in ids {
                    if let Err(error) = cloud.like(id, liked).await {
                        log::warn!("favorites::netease_like({id}): {}", describe(&error));
                        return;
                    }
                }
                if fav_ui.section_active() {
                    if let Err(error) =
                        favorites_ui_mod::refresh_tracks(&state, &fav_ui, &weak).await
                    {
                        log::warn!("favorites::refresh_after_like: {}", describe(&error));
                    }
                } else {
                    fav_ui.mark_songs_dirty();
                }
            });
        });
    }

    // The cloud list has no local playback queue to mutate.
    g.on_play_next(|_| {});
    g.on_add_to_queue(|_| {});

    {
        let state = state.clone();
        let fav_ui = fav_ui.clone();
        let weak = weak.clone();
        g.on_filter_changed(move |text| {
            let Some(ui) = weak.upgrade() else { return };
            favorites_ui_mod::set_filter(&fav_ui, &text);
            favorites_ui_mod::albums::apply_now(&ui, &fav_ui);
            favorites_ui_mod::saved_artists::apply_now(&ui, &fav_ui);
            favorites_ui_mod::cloud_disk::apply_now(&ui, &state, &fav_ui);
            favorites_ui_mod::apply_filtered_tracks(&fav_ui, &weak);
            favorites_ui_mod::apply_filtered_grids_now(&ui, &fav_ui);
        });
    }

    {
        let state = state.clone();
        let fav_ui = fav_ui.clone();
        let weak = weak.clone();
        g.on_request_sort(move |field| {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Favorites>();
            let (field, direction) =
                next_sort(g.get_sort_field().as_str(), g.get_sort_dir().as_str(), &field);
            g.set_sort_field(SharedString::from(field.as_str()));
            g.set_sort_dir(SharedString::from(direction.as_str()));
            favorites_ui_mod::set_sort(&fav_ui, field.clone(), direction);
            fav_ui.state().tracks_all.resort(&field, direction.as_str());
            favorites_ui_mod::apply_filtered_tracks_now(&ui, &fav_ui);
            persist_view_sort(&state, view_id::FAVORITES, field, direction);
        });
    }
}

fn wire_playback(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let g = ui.global::<Favorites>();
    let state_for_remote = state.clone();
    let fav_ui_for_remote = fav_ui.clone();
    g.on_play_cloud_row(move |remote_id, _index| {
        let Some(song_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else {
            log::warn!("favorites::play_cloud_song: invalid NetEase song ID");
            return;
        };
        // The two lists are separate caches, so the queue has to be the one the row was drawn
        // from; the remote id is the only thing that picks it.
        let ids = if fav_ui_for_remote.active_tab() == FavoritesTab::CloudDisk {
            fav_ui_for_remote.cloud_disk_track_ids()
        } else {
            fav_ui_for_remote.filtered_track_ids()
        };
        play_favorite_song(&state_for_remote, ids, song_id);
    });

    let state = state.clone();
    let fav_ui = fav_ui.clone();
    g.on_play_row(move |track_id, _index| {
        let song_id = i64::from(track_id);
        if song_id > 0 {
            play_favorite_song(&state, fav_ui.filtered_track_ids(), song_id);
        }
    });
}

fn play_favorite_song(state: &AppState, ids: Vec<i64>, song_id: i64) {
    let state = state.clone();
    state.runtime.spawn(async move {
        let (volume, speed) = {
            let player = lock_state(&state.player_state);
            (player.effective_volume(), player.playback_speed)
        };
        if let Err(error) =
            state.cloud_playback.play_queue(&ids, song_id, false, volume, speed).await
        {
            log::warn!("favorites::play_cloud_song({song_id}): {}", describe(&error));
        }
    });
}
