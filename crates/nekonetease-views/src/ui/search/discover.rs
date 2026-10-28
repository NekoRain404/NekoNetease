//! The empty-query discovery strip: `NetEase`'s trending searches, recommended playlists and the
//! full chart catalogue.
//!
//! All three are public endpoints, so they answer as a guest and fail as ordinary network errors.
//! A failed feed logs, leaves its model empty and raises its own flag, so its section paints a
//! retry prompt while the others and the Recent chips keep working — the page stays useful offline.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::ui::grid_rows::chunk_rows;
use crate::ui::search::{SearchUi, fetch};
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::entities::netease_discovery::HotSearchItem;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Search, SearchEntityRow, SearchGridRow};

/// The endpoint returns a ranked feed far longer than an empty page should push Recent down.
pub(super) const HOT_SEARCH_LIMIT: usize = 12;
/// A discovery strip, not the playlist directory: enough to invite a click, cheap to paint.
/// The chart catalogue is the full list and is deliberately not capped by this.
pub(super) const DISCOVER_LIMIT: usize = 8;

/// Which of the three feeds came back empty-handed. Published in the same UI write as the rows,
/// so a section never repaints twice to say what it has and what it is missing.
#[derive(Clone, Copy)]
struct DiscoveryFailures {
    hot: bool,
    playlists: bool,
    charts: bool,
}

pub(super) fn to_playlist_rows(playlists: &[Playlist]) -> Vec<SearchEntityRow> {
    playlists.iter().take(DISCOVER_LIMIT).map(fetch::playlist_entity_row).collect()
}

/// Every chart, in the server's own order.
///
/// `/api/toplist/detail` publishes no category field. `tags` are per-chart keywords — a chart may
/// carry three, most carry none — so grouping on one of them would invent a taxonomy and scatter
/// the curated order the directory already ranks its charts in. That order is the only one the
/// server actually states, so it is the one the 全部榜单 list keeps.
pub(super) fn to_chart_rows(charts: &[Playlist]) -> Vec<SearchEntityRow> {
    charts.iter().map(chart_entity_row).collect()
}

/// A chart's subtitle is every tag the server gave it, and nothing when it gave none — where the
/// playlist projection would have shown the chart's blurb.
fn chart_entity_row(chart: &Playlist) -> SearchEntityRow {
    let mut row = fetch::playlist_entity_row(chart);
    row.subtitle = SharedString::from(chart.tags.join(" · "));
    row
}

pub(super) fn to_hot_words(items: &[HotSearchItem]) -> Vec<SharedString> {
    items.iter().take(HOT_SEARCH_LIMIT).map(|item| SharedString::from(item.word.as_str())).collect()
}

/// Spawn this section visit's discovery fetch, when the empty-query state is what is on screen.
///
/// Every entry point goes through here: the section-active edge, the boot mount and the box
/// going empty all ask the same question, so none of them can forget the query or visibility
/// half of it.
pub(super) fn ensure_loaded(state: &AppState, search_ui: &Arc<SearchUi>, weak: &Weak<AppWindow>) {
    if !search_ui.section_active() {
        return;
    }
    let Some(ui) = weak.upgrade() else {
        return;
    };
    if !ui.global::<Search>().get_query().trim().is_empty() {
        return;
    }
    {
        let mut loaded = search_ui.state().discovery_loaded.lock();
        if *loaded {
            return;
        }
        // Claimed here rather than in the spawned future: two entry points in one frame would
        // both read an unclaimed flag, and the loser's loading line could outlive the winner's
        // answer.
        *loaded = true;
    }
    let token = search_ui.discover_token.fetch_add(1, Ordering::AcqRel).wrapping_add(1);

    {
        // Written before the spawn so the line is up on the frame the request starts, rather than
        // when the runtime first polls it.
        let g = ui.global::<Search>();
        g.set_discovery_loading(true);
        g.set_hot_searches_failed(false);
        g.set_discovery_failed(false);
        g.set_charts_failed(false);
    }

    let state = state.clone();
    let search_ui = search_ui.clone();
    let weak = weak.clone();
    let runtime = state.runtime.clone();
    runtime.spawn(async move {
        // Three concurrent requests push this future past clippy's inline-size budget.
        Box::pin(kick_discovery(&state, &search_ui, &weak, token)).await;
    });
}

/// Fetch all three feeds once. The caller has already claimed `discovery_loaded`, so a second
/// activation in one frame finds it taken; a failed pass releases it for the next visit.
async fn kick_discovery(
    state: &AppState,
    search_ui: &Arc<SearchUi>,
    weak: &Weak<AppWindow>,
    token: u64,
) {
    let (hot, playlists, charts) = tokio::join!(
        state.cloud.hot_search(),
        state.cloud.discover(),
        state.cloud.chart_catalogue()
    );
    if search_ui.discover_token.load(Ordering::Acquire) != token {
        // Left the section, or a newer visit claimed the strip, while the requests were out. The
        // release has already let go of the claim, so a live one belongs to that newer visit.
        return;
    }

    let hot_ok = match hot {
        Ok(items) => {
            *search_ui.state().hot_searches.lock() = items;
            true
        }
        Err(error) => {
            log::warn!("search::hot_search: {}", describe(&error));
            false
        }
    };
    let discover_ok = match playlists {
        Ok(playlists) => {
            *search_ui.state().discovery_rows.lock() = to_playlist_rows(&playlists);
            true
        }
        Err(error) => {
            log::warn!("search::discover: {}", describe(&error));
            false
        }
    };
    let charts_ok = match charts {
        Ok(charts) => {
            *search_ui.state().chart_rows.lock() = to_chart_rows(&charts);
            true
        }
        Err(error) => {
            log::warn!("search::charts: {}", describe(&error));
            false
        }
    };
    if !(hot_ok && discover_ok && charts_ok) {
        // A feed that failed gets another chance on the next section entry.
        *search_ui.state().discovery_loaded.lock() = false;
    }

    let failures = DiscoveryFailures { hot: !hot_ok, playlists: !discover_ok, charts: !charts_ok };
    apply_discovery(search_ui, token, weak, failures);
}

fn apply_discovery(
    search_ui: &Arc<SearchUi>,
    token: u64,
    weak: &Weak<AppWindow>,
    failures: DiscoveryFailures,
) {
    let words = to_hot_words(&search_ui.state().hot_searches.lock());
    let rows = search_ui.state().discovery_rows.lock().clone();
    let chart_rows = search_ui.state().chart_rows.lock().clone();
    let search_ui = search_ui.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.discover_token.load(Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        // The grids are mounted only by the flat lists below, so their chunked models land in the
        // same write; a column count derived since the last visit is honoured here.
        g.set_discovery_grid_rows(grid_model(chunk(&rows, g.get_discovery_columns())));
        g.set_chart_grid_rows(grid_model(chunk(&chart_rows, g.get_chart_columns())));
        g.set_hot_searches(ModelRc::from(Rc::new(VecModel::from(words))));
        g.set_discovery_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        g.set_chart_rows(ModelRc::from(Rc::new(VecModel::from(chart_rows))));
        g.set_discovery_loading(false);
        g.set_hot_searches_failed(failures.hot);
        g.set_discovery_failed(failures.playlists);
        g.set_charts_failed(failures.charts);
    });
}

/// Re-chunk each grid when its width packs a different column count, so a resize never
/// re-requests either feed.
pub(super) fn wire_grids(ui: &AppWindow, search_ui: &Arc<SearchUi>) {
    let weak = ui.as_weak();
    let discovery_ui = search_ui.clone();
    ui.global::<Search>().on_discovery_columns_changed(move |columns| {
        let Some(ui) = weak.upgrade() else { return };
        let rows = discovery_ui.state().discovery_rows.lock();
        ui.global::<Search>().set_discovery_grid_rows(grid_model(chunk(&rows, columns)));
    });

    let weak = ui.as_weak();
    let chart_ui = search_ui.clone();
    ui.global::<Search>().on_chart_columns_changed(move |columns| {
        let Some(ui) = weak.upgrade() else { return };
        let chart_rows = chart_ui.state().chart_rows.lock();
        ui.global::<Search>().set_chart_grid_rows(grid_model(chunk(&chart_rows, columns)));
    });
}

/// The flat cards plus their chunked projection. The chunk borrows the flat list, which the
/// section keeps so a resize re-chunks without touching the network.
pub(super) fn chunk(rows: &[SearchEntityRow], columns: i32) -> Vec<SearchGridRow> {
    chunk_rows(rows, columns, Clone::clone, |entities| SearchGridRow { entities })
}

fn grid_model(rows: Vec<SearchGridRow>) -> ModelRc<SearchGridRow> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// Re-ask the three feeds after a failure. A retry while one is still in flight is a no-op, so a
/// second press can't stack a request over the one it is waiting on.
pub(super) fn retry(state: &AppState, search_ui: &Arc<SearchUi>, weak: &Weak<AppWindow>) {
    let Some(ui) = weak.upgrade() else {
        return;
    };
    if ui.global::<Search>().get_discovery_loading() {
        return;
    }
    // A failed pass already released the claim; clearing it again is what makes the retry
    // unconditional.
    *search_ui.state().discovery_loaded.lock() = false;
    ensure_loaded(state, search_ui, weak);
}

#[cfg(test)]
#[path = "tests/discover_state_tests.rs"]
mod tests;
