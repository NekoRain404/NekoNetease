//! UI-thread writes for cloud search results and request state.

use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use std::sync::Arc;

use crate::ui::search::SearchUi;
use nekonetease_core::entities::cloud::SearchKind;
use nekonetease_ui::{AppWindow, Search, SearchEntityRow, TrackListRow as UiTrackListRow};

pub fn apply_cloud_search_results(
    search_ui: &Arc<SearchUi>,
    token: u64,
    weak: &Weak<AppWindow>,
    kind: SearchKind,
    songs: Vec<UiTrackListRow>,
    entities: Vec<SearchEntityRow>,
    page: super::fetch::SearchPageState,
    page_index: i32,
) {
    let search_ui = search_ui.clone();
    let kind_key = super::fetch::search_kind_key(kind);
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        g.set_search_kind(kind_key.into());
        g.set_page_index(page_index);
        g.set_results_total(i32::try_from(page.total).unwrap_or(i32::MAX));
        g.set_results_summary(page.summary.into());
        g.set_has_previous_page(page.has_previous);
        g.set_has_next_page(page.has_next);
        g.set_search_error(SharedString::default());
        g.set_loading(false);
        g.set_show_all_tracks(false);
        let songs_total = i32::try_from(songs.len()).unwrap_or(i32::MAX);
        g.set_tracks_total(songs_total);
        set_tracks(
            &g,
            if g.get_show_all_tracks() {
                songs
            } else {
                songs.into_iter().take(super::state::COMPACT_TRACK_LIMIT).collect()
            },
        );
        g.set_entity_results(ModelRc::from(Rc::new(VecModel::from(entities))));
        g.set_album_rows(ModelRc::from(Rc::new(VecModel::default())));
        g.set_artist_rows(ModelRc::from(Rc::new(VecModel::default())));
        clear_top_result(&g);
    });
}

pub fn apply_cached_cloud_rows(search_ui: &Arc<SearchUi>, weak: &Weak<AppWindow>) {
    let rows = search_ui.state().cloud_song_rows.lock().clone();
    let total_rows = i32::try_from(rows.len()).unwrap_or(i32::MAX);
    let token = search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire);
    let search_ui = search_ui.clone();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        let take = if g.get_show_all_tracks() {
            rows.len()
        } else {
            rows.len().min(super::state::COMPACT_TRACK_LIMIT)
        };
        let mut rows = rows;
        rows.truncate(take);
        set_tracks(&g, rows);
        g.set_tracks_total(total_rows);
    });
}

pub fn apply_cloud_top_result(
    search_ui: &Arc<SearchUi>,
    token: u64,
    weak: &Weak<AppWindow>,
    top: Option<super::fetch::CloudTopResult>,
) {
    let search_ui = search_ui.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        let Some(top) = top else {
            clear_top_result(&g);
            return;
        };
        g.set_top_kind(top.kind.into());
        g.set_top_id(i32::try_from(top.id).unwrap_or(i32::MAX));
        g.set_top_remote_id(top.remote_id.into());
        g.set_top_title(top.title.into());
        g.set_top_subtitle(top.subtitle.into());
        g.set_top_artwork_path(top.artwork_path.into());
    });
}

pub fn clear_results_on_ui(search_ui: &Arc<SearchUi>, weak: &Weak<AppWindow>) {
    let token = search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire);
    let search_ui = search_ui.clone();
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        set_tracks(&g, Vec::new());
        g.set_entity_results(ModelRc::from(Rc::new(VecModel::default())));
        g.set_tracks_total(0);
        g.set_results_total(0);
        g.set_results_summary(SharedString::default());
        g.set_page_index(0);
        g.set_has_previous_page(false);
        g.set_has_next_page(false);
        g.set_search_error(SharedString::default());
        g.set_loading(false);
        g.set_show_all_tracks(false);
        g.set_album_rows(ModelRc::from(Rc::new(VecModel::default())));
        g.set_artist_rows(ModelRc::from(Rc::new(VecModel::default())));
        g.set_suggestions(ModelRc::from(Rc::new(VecModel::default())));
        g.set_suggestions_query(SharedString::default());
        g.set_suggestions_pending(false);
        g.set_committed_query(SharedString::default());
        clear_top_result(&g);
    });
}

pub fn set_loading_on_ui(search_ui: &Arc<SearchUi>, token: u64, weak: &Weak<AppWindow>, on: bool) {
    let search_ui = search_ui.clone();
    let weak = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        g.set_loading(on);
        if on {
            g.set_search_error(SharedString::default());
            set_tracks(&g, Vec::new());
            g.set_entity_results(ModelRc::from(Rc::new(VecModel::default())));
            g.set_tracks_total(0);
            g.set_results_total(0);
            g.set_results_summary(SharedString::default());
            g.set_has_previous_page(false);
            g.set_has_next_page(false);
            g.set_album_rows(ModelRc::from(Rc::new(VecModel::default())));
            g.set_artist_rows(ModelRc::from(Rc::new(VecModel::default())));
            clear_top_result(&g);
        }
    });
}

pub fn set_error_on_ui(
    search_ui: &Arc<SearchUi>,
    token: u64,
    weak: &Weak<AppWindow>,
    error: String,
) {
    let search_ui = search_ui.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.fetch_token.load(std::sync::atomic::Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        g.set_search_error(error.into());
        g.set_loading(false);
    });
}

/// Patch a cloud song row by its exact remote identity without rebuilding the current model.
pub fn apply_cloud_row_favorite(weak: &Weak<AppWindow>, id: i64, favorite: bool) {
    patch_favorite(weak, id, favorite);
}

/// Patch a row through either its exact cloud identity or its local Slint id. Search is cloud-only
/// now, but this is kept for the shared row-flag callback shape used by its `TrackList` global.
pub fn apply_row_favorite(weak: &Weak<AppWindow>, id: i64, favorite: bool) {
    patch_favorite(weak, id, favorite);
}

fn patch_favorite(weak: &Weak<AppWindow>, id: i64, favorite: bool) {
    let key = id.to_string();
    let weak = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<Search>();
        let model = g.get_tracks();
        for index in 0..model.row_count() {
            let Some(mut row) = model.row_data(index) else { continue };
            let row_id = row.remote_id.as_str().parse::<i64>().unwrap_or(i64::from(row.id));
            if row_id == id || (row_id == 0 && row.remote_id.as_str() == key) {
                row.is_favorite = favorite;
                model.set_row_data(index, row);
            }
        }
    });
}

pub fn set_tracks(g: &Search, rows: Vec<UiTrackListRow>) {
    g.set_tracks(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn clear_top_result(g: &Search) {
    g.set_top_kind(SharedString::default());
    g.set_top_id(-1);
    g.set_top_remote_id(SharedString::default());
    g.set_top_title(SharedString::default());
    g.set_top_subtitle(SharedString::default());
    g.set_top_artwork_path(SharedString::default());
}

pub fn teardown_models_on_leave(ui: &AppWindow) {
    clear_models(&ui.global::<Search>());
}

fn clear_models(g: &Search) {
    set_tracks(g, Vec::new());
    g.set_entity_results(ModelRc::from(Rc::new(VecModel::default())));
    g.set_tracks_total(0);
    g.set_results_total(0);
    g.set_results_summary(SharedString::default());
    g.set_page_index(0);
    g.set_has_previous_page(false);
    g.set_has_next_page(false);
    g.set_search_error(SharedString::default());
    g.set_loading(false);
    g.set_album_rows(ModelRc::from(Rc::new(VecModel::default())));
    g.set_artist_rows(ModelRc::from(Rc::new(VecModel::default())));
    g.set_suggestions(ModelRc::from(Rc::new(VecModel::default())));
    g.set_suggestions_query(SharedString::default());
    g.set_suggestions_pending(false);
    // `committed-query` survives with `query`: a preserved query is still the one its results
    // answered, and clearing it would make a re-entry read as an uncommitted edit.
    clear_top_result(g);
}
