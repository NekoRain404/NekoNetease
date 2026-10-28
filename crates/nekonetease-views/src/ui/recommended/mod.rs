//! The recommended page: the account's suggested playlists over the shared card grid.
//!
//! The page is the retired My Library Recommendations category moved out. Its two special cards
//! open the daily and radar playlists through `Playlists.open-special`, and the grid is
//! `PlaylistGrid`, the virtualized component My Library's Playlists tab draws. The flat rows are
//! kept beside the chunked model so a window resize re-chunks them without asking the directory
//! again.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_compat::Compat;
use parking_lot::Mutex;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::ui::grid_rows::chunk_rows;
use crate::ui::view_ctx::ViewCtx;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Nav, PlaylistGridRow, PlaylistRow, Recommended};

/// This page's `Nav.selected-index`.
pub const NAV_RECOMMENDED: i32 = 13;

/// Bumped per fetch and again on leave; a response resolving after either writes nothing.
static RECOMMENDED_REQUEST: AtomicU64 = AtomicU64::new(0);

/// The flat cards behind the chunked model. `PlaylistGrid` virtualizes by row, so a column
/// change has to rebuild the rows from something that outlives the model it just replaced.
struct RecommendedUi {
    playlists: Mutex<Vec<PlaylistRow>>,
}

/// Wire the page's callbacks. Mounting the view is the shell's job.
pub fn install(cx: ViewCtx<'_>) {
    let ViewCtx { app, state, .. } = cx;
    let state = state.clone();
    let recommended_ui = Arc::new(RecommendedUi { playlists: Mutex::new(Vec::new()) });

    let weak = app.as_weak();
    let section_state = state.clone();
    let section_ui = recommended_ui.clone();
    app.global::<Recommended>().on_section_active_changed(move |active| {
        let Some(ui) = weak.upgrade() else { return };
        if active {
            load(&ui, &section_state, &section_ui);
        } else {
            release(&ui, &section_ui);
        }
    });

    let weak = app.as_weak();
    let retry_state = state.clone();
    let retry_ui = recommended_ui.clone();
    app.global::<Recommended>().on_retry(move || {
        let Some(ui) = weak.upgrade() else { return };
        // A retry is a fresh visit: the guard below would otherwise see the finished one.
        ui.global::<Recommended>().set_loaded(false);
        load(&ui, &retry_state, &retry_ui);
    });

    let weak = app.as_weak();
    let columns_ui = recommended_ui.clone();
    app.global::<Recommended>().on_columns_changed(move |columns| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.global::<Recommended>().get_loading() {
            return;
        }
        let chunks = {
            let playlists = columns_ui.playlists.lock();
            chunk(&playlists, columns)
        };
        ui.global::<Recommended>().set_grid_rows(ModelRc::from(Rc::new(VecModel::from(chunks))));
    });

    // The suggestions are account-scoped, so a sign-in or sign-out has to drop them — the
    // section gate sees nothing while the page is the one on screen.
    let mut account_rx = state.cloud_playback.subscribe_account();
    let account_ui = recommended_ui.clone();
    let account_state = state.clone();
    let weak = app.as_weak();
    let _ = slint::spawn_local(Compat::new(async move {
        while account_rx.changed().await.is_ok() {
            let Some(ui) = weak.upgrade() else { return };
            release(&ui, &account_ui);
            if ui.global::<Nav>().get_selected_index() == NAV_RECOMMENDED {
                load(&ui, &account_state, &account_ui);
            }
        }
    }));

    // The gate's `ChangeTracker` baselines inside `AppWindow::new()`, before `install_views` seeds
    // the persisted index, so a boot that lands on this page never sees an enter edge.
    if app.global::<Nav>().get_selected_index() == NAV_RECOMMENDED {
        load(app, &state, &recommended_ui);
    }
}

fn release(ui: &AppWindow, recommended_ui: &RecommendedUi) {
    RECOMMENDED_REQUEST.fetch_add(1, Ordering::AcqRel);
    recommended_ui.playlists.lock().clear();
    let g = ui.global::<Recommended>();
    g.set_grid_rows(ModelRc::default());
    g.set_loading(false);
    g.set_loaded(false);
    g.set_failed(false);
}

/// Fetch the suggestions unless this visit already holds an answer.
fn load(ui: &AppWindow, state: &AppState, recommended_ui: &Arc<RecommendedUi>) {
    let g = ui.global::<Recommended>();
    if g.get_loading() || g.get_loaded() {
        return;
    }
    g.set_loading(true);
    g.set_failed(false);

    let token = RECOMMENDED_REQUEST.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let runtime = state.runtime.clone();
    let state = state.clone();
    let recommended_ui = recommended_ui.clone();
    let weak = ui.as_weak();
    runtime.spawn(async move {
        let recommendations = state.cloud.recommendations().await;
        if RECOMMENDED_REQUEST.load(Ordering::Acquire) != token {
            return;
        }
        let result = recommendations.map(|playlists| playlist_rows(&playlists));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if RECOMMENDED_REQUEST.load(Ordering::Acquire) != token {
                return;
            }
            let g = ui.global::<Recommended>();
            g.set_loading(false);
            match result {
                Ok(rows) => {
                    let chunks = chunk(&rows, g.get_columns());
                    *recommended_ui.playlists.lock() = rows;
                    g.set_grid_rows(ModelRc::from(Rc::new(VecModel::from(chunks))));
                    g.set_failed(false);
                }
                Err(error) => {
                    log::warn!("recommended::recommendations: {}", describe(&error));
                    g.set_failed(true);
                }
            }
            g.set_loaded(true);
        });
    });
}

/// The flat cards plus their chunked projection. The chunk borrows the flat list, which the
/// page keeps; `chunk_built_rows` would consume rows a resize still has to re-chunk.
fn chunk(playlists: &[PlaylistRow], columns: i32) -> Vec<PlaylistGridRow> {
    chunk_rows(playlists, columns, Clone::clone, |playlists| PlaylistGridRow { playlists })
}

/// A card keeps the exact 64-bit id as text — Slint's `int` cannot carry it — while `id` stays
/// the bounded row index the grid's own callbacks use.
fn playlist_rows(playlists: &[Playlist]) -> Vec<PlaylistRow> {
    playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| PlaylistRow {
            id: i32::try_from(index).unwrap_or(i32::MAX),
            remote_id: SharedString::from(playlist.id.to_string()),
            name: SharedString::from(playlist.name.as_str()),
            description: SharedString::default(),
            artwork_path: SharedString::from(remote_cover(playlist.cover_url.as_deref())),
            track_count: i32::try_from(playlist.track_count).unwrap_or(i32::MAX),
            total_duration_ms: 0,
            is_smart: false,
        })
        .collect()
}

/// The grid's cover tier fetches remote artwork only, so a local path would name a file it
/// cannot read.
fn remote_cover(url: Option<&str>) -> String {
    url.filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
#[path = "tests/recommended_tests.rs"]
mod tests;
