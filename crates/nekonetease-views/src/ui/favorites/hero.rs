//! Hero stats + the banner artwork refresh.
//!
//! The Favorites view derives its count and artwork from fetched `NetEase` songs and composes their
//! first four distinct covers into one collage through [`crate::ui::mosaic_hero`].

use std::sync::Arc;

use slint::{ComponentHandle, Weak};

use super::FavoritesUi;
use crate::ui::detail_view::impl_curated_hero_helpers;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::track::FavoriteStats;
use nekonetease_core::entities::track::TrackListRow;
use nekonetease_ui::{AppWindow, Favorites};

impl_curated_hero_helpers!(Favorites, FavoritesUi, crate::ui::hero_chips::publish_favorites);

/// Derive the banner's count, running time and artwork from the `NetEase` rows already fetched for
/// the Songs tab. `generation` prevents a slow collage from an old account being published after
/// an account transition.
pub async fn refresh_from_tracks(
    state: &AppState,
    fav_ui: &Arc<FavoritesUi>,
    weak: &Weak<AppWindow>,
    rows: &[TrackListRow],
    generation: u64,
    liked_revision: u64,
    animate: bool,
) {
    let mut artwork_paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total_duration_ms = 0_i64;
    for row in rows {
        total_duration_ms = total_duration_ms.saturating_add(row.duration_ms.max(0));
        if let Some(path) = row.artwork_path.as_deref()
            && !path.is_empty()
            && seen.insert(path.to_owned())
        {
            artwork_paths.push(path.to_owned());
            if artwork_paths.len() == 4 {
                break;
            }
        }
    }
    let stats = FavoriteStats {
        count: i64::try_from(rows.len()).unwrap_or(i64::MAX),
        total_duration_ms,
        artwork_paths,
    };

    // A leave that landed while the query was in flight has already wiped `stats` and
    // forgotten the collage guard, so the store and the push below would fill both back
    // in behind a view nobody can see. The leave set `mark_dirty`, so the next enter
    // re-fetches — the guard `refresh_grids` and `refresh_tracks` carry, in the same
    // place, after the slow part.
    if !fav_ui.section_active()
        || state.cloud.status().generation != generation
        || state.cloud.liked_ids_revision() != liked_revision
    {
        return;
    }

    {
        // Under the section gate: `release_section_state` wipes this beside the
        // caches, so the two must not interleave. Synchronous store only.
        let _gate = fav_ui.gate();
        *fav_ui.state().stats.lock() = stats.clone();
    }
    push_stats_to_slint(&stats, fav_ui, weak);

    let paths = stats.artwork_paths.clone();

    // Skip the compose when those covers are already the ones on screen — a library or
    // stats tick usually returns the same top four, and an unchanged banner is still
    // correct. The matching *claim* is in `publish_hero_artwork`, for `MosaicGuard`'s
    // reason.
    if fav_ui.active_tab() != super::FavoritesTab::Songs
        || !fav_ui.state().last_mosaic_paths.is_stale(&paths)
    {
        return;
    }

    let Some(pair) =
        crate::ui::mosaic_hero::compose_off_thread(state, paths.clone(), fav_ui.hero_blur).await
    else {
        return;
    };
    if state.cloud.status().generation != generation
        || state.cloud.liked_ids_revision() != liked_revision
        || !fav_ui.section_active()
        || fav_ui.active_tab() != super::FavoritesTab::Songs
    {
        return;
    }
    publish_hero_artwork(fav_ui, weak, pair, animate, paths);
}

fn push_stats_to_slint(stats: &FavoriteStats, fav_ui: &Arc<FavoritesUi>, weak: &Weak<AppWindow>) {
    let count = i32::try_from(stats.count).unwrap_or(i32::MAX);
    let fav_ui = fav_ui.clone();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        // The leave can land while this post is in flight, and it rewinds the count on its
        // way out — the same guard, in the same place, as `songs::apply_filtered_tracks`.
        if !fav_ui.section_active() {
            return;
        }
        ui.global::<Favorites>().set_track_count(count);
        // Order-free: the chips take their facts off the handle's own state,
        // not back off the properties written around them.
        crate::ui::hero_chips::publish_favorites(&ui, &fav_ui);
    });
}
