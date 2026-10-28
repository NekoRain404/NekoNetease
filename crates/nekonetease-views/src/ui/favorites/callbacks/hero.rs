//! Shuffle actions for the Favorites hero.

use std::sync::Arc;

use rand::RngExt;
use slint::ComponentHandle;

use crate::ui::favorites::FavoritesUi;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_ui::{AppWindow, Favorites};

/// Wire the two hero shuffle callbacks.
pub(super) fn wire(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let g = ui.global::<Favorites>();

    {
        let s = state.clone();
        let fu = fav_ui.clone();
        g.on_shuffle_all(move || {
            let ids = fu.filtered_track_ids();
            if ids.is_empty() {
                return;
            }
            let id = ids[rand::rng().random_range(..ids.len())];
            let s = s.clone();
            s.runtime.spawn(async move {
                let (volume, speed) = {
                    let player = lock_state(&s.player_state);
                    (player.effective_volume(), player.playback_speed)
                };
                if let Err(error) = s.cloud_playback.play_queue(&ids, id, true, volume, speed).await
                {
                    log::warn!("favorites::shuffle_cloud_song: {}", describe(&error));
                }
            });
        });
    }

    {
        g.on_shuffle_most_played(|| {});
    }
}
