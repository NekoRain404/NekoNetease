//! Favorite reads and writes.

use std::sync::Arc;

use crate::state::{AppState, Signal};
use nekonetease_core::entities::{artist, track};
use nekonetease_core::error::AppError;
use nekonetease_engine::player::engine::event_sink::PlayerSinks;
use nekonetease_engine::player::engine::state::{
    PlayerAction, PlayerStateHandle, lock_state, sync_current_track_if_in, with_state_emit,
};
use nekonetease_store::database::{DbPool, queries};

/// What a favorite write reaches: the row, the cached copy the Now-Playing surfaces
/// read, and the subscribers that re-fetch off it.
///
/// Owned clones rather than an `&AppState`, for `tasks::rating_writeback::Writeback`'s
/// reason. Every field is cheap to clone, and naming them is what lets the order these
/// two writers share be driven by a test at all.
struct FavoriteWrite {
    db: DbPool,
    player_state: Arc<PlayerStateHandle>,
    sinks: Arc<PlayerSinks>,
    library_changed: Signal,
}

impl FavoriteWrite {
    fn from_state(state: &AppState) -> Self {
        Self {
            db: state.db.clone(),
            player_state: state.player_state.clone(),
            sinks: state.sinks.clone(),
            library_changed: state.library_changed.clone(),
        }
    }

    async fn set(&self, ids: &[i64], favorite: bool) -> Result<(), AppError> {
        queries::track::set_favorite(&self.db, ids, favorite).await?;
        // After the write, so the line means it landed rather than was attempted.
        log::debug!("favorite: {} track(s) → {favorite}", ids.len());
        // If the currently-playing track was one of the toggled ids, mirror the new
        // flag onto `current_track` so the Now-Playing heart updates without waiting
        // for the next track load (parity with `toggle_current`).
        sync_current_track_if_in(&self.player_state, &self.sinks, ids, |t| {
            t.is_favorite = favorite;
        });
        self.library_changed.bump();
        Ok(())
    }

    async fn toggle_current(&self) -> Result<Option<(i64, bool)>, AppError> {
        let Some((id, new_fav)) = ({
            let g = lock_state(&self.player_state);
            g.current_track().map(|t| (t.id, !t.is_favorite))
        }) else {
            return Ok(None);
        };

        queries::track::set_favorite(&self.db, &[id], new_fav).await?;
        log::debug!("favorite: playing track {id} → {new_fav}");

        with_state_emit(&self.player_state, &self.sinks, |s| {
            // Guard against a track change between the id read above and here: only
            // flip the cached flag if `current_track` is still the track we wrote.
            if let Some(track) = s.current_track_mut()
                && track.id == id
            {
                Arc::make_mut(track).is_favorite = new_fav;
            }
            Vec::<PlayerAction>::new()
        });

        self.library_changed.bump();

        Ok(Some((id, new_fav)))
    }
}

pub async fn set_favorite(state: &AppState, ids: Vec<i64>, favorite: bool) -> Result<(), AppError> {
    FavoriteWrite::from_state(state).set(&ids, favorite).await
}

/// Toggle the favorite flag on the currently playing track. Persists to DB,
/// flips `is_favorite` on `PlayerState.current_track` (so the next emit
/// rebuilds the view-model with the new value), and returns the affected
/// `(id, new_fav)` so callers can mirror the change into other UI surfaces
/// (e.g. the tracks list) without re-locking.
pub async fn toggle_current_favorite(state: &AppState) -> Result<Option<(i64, bool)>, AppError> {
    FavoriteWrite::from_state(state).toggle_current().await
}

pub async fn get_favorite_tracks(db: &DbPool) -> Result<Vec<track::TrackListRow>, AppError> {
    queries::track::get_favorite_tracks_for_list(db).await
}

pub async fn get_favorite_stats(db: &DbPool) -> Result<track::FavoriteStats, AppError> {
    queries::track::get_favorite_stats(db).await
}

pub async fn get_favorite_artists(db: &DbPool) -> Result<Vec<artist::FavoriteArtist>, AppError> {
    queries::artist::get_favorite_artists(db).await
}

/// Favorite tracks ranked by play count — the whole set, since the Most Played
/// tab is a virtualized grid and has no reason to truncate.
pub async fn get_most_played_favorites(
    db: &DbPool,
) -> Result<Vec<track::MostPlayedFavorite>, AppError> {
    queries::track::get_most_played_favorites(db).await
}

#[cfg(test)]
#[path = "tests/favorites_tests.rs"]
mod tests;
