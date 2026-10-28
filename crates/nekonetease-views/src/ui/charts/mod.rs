//! The charts page: the published catalogue as a virtualized card grid of its own.
//!
//! The row type and the cover tier are Search's — `SearchEntityRow`, and the entity-cover callback
//! its empty state paints through — because the same catalogue appears in two places and two row
//! shapes would let them disagree about a chart. What this slice owns is the visit: Search fetches
//! the catalogue beside two other feeds and drops it with its own section, so this page fetches the
//! one feed it needs into a model released on its own leave. Opening a card is Search's `open-chart`
//! hand-off unchanged, so a chart lands on My Library's Playlists tab from here as it does there.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::ui::grid_rows::chunk_rows;
use crate::ui::view_ctx::ViewCtx;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, ChartGridRow, Charts, Nav, SearchEntityRow};

/// This page's `Nav.selected-index`.
pub const NAV_CHARTS: i32 = 12;

/// Bumped per fetch and again on leave; a response resolving after either writes nothing.
static CHARTS_REQUEST: AtomicU64 = AtomicU64::new(0);

/// The flat cards behind the chunked rows, kept so a resize re-chunks without another fetch.
struct ChartsUi {
    charts: Mutex<Vec<SearchEntityRow>>,
}

/// Wire the page's callbacks. Mounting the view is the shell's job.
pub fn install(cx: ViewCtx<'_>) {
    let ViewCtx { app, state, .. } = cx;
    let state = state.clone();
    let charts_ui = Arc::new(ChartsUi { charts: Mutex::new(Vec::new()) });

    let weak = app.as_weak();
    let section_state = state.clone();
    let section_ui = charts_ui.clone();
    app.global::<Charts>().on_section_active_changed(move |active| {
        let Some(ui) = weak.upgrade() else { return };
        if active {
            load(&ui, &section_state, &section_ui);
        } else {
            release(&ui, &section_ui);
        }
    });

    let weak = app.as_weak();
    let retry_state = state.clone();
    let retry_ui = charts_ui.clone();
    app.global::<Charts>().on_retry(move || {
        let Some(ui) = weak.upgrade() else { return };
        load(&ui, &retry_state, &retry_ui);
    });

    let weak = app.as_weak();
    let columns_ui = charts_ui.clone();
    app.global::<Charts>().on_columns_changed(move |columns| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.global::<Charts>().get_loading() {
            return;
        }
        let chunks = {
            let charts = columns_ui.charts.lock();
            chunk(&charts, columns)
        };
        ui.global::<Charts>().set_rows(ModelRc::from(Rc::new(VecModel::from(chunks))));
    });

    // The gate's `ChangeTracker` baselines inside `AppWindow::new()`, before `install_views` seeds
    // the persisted index, so a boot that lands on this page never sees an enter edge.
    if app.global::<Nav>().get_selected_index() == NAV_CHARTS {
        load(app, &state, &charts_ui);
    }
}

fn release(ui: &AppWindow, charts_ui: &ChartsUi) {
    CHARTS_REQUEST.fetch_add(1, Ordering::AcqRel);
    charts_ui.charts.lock().clear();
    let g = ui.global::<Charts>();
    g.set_rows(ModelRc::default());
    g.set_loading(false);
    g.set_failed(false);
}

/// Fetch the catalogue unless this visit already holds one. A leave releases the rows, so a
/// re-entry asks the directory again rather than painting the previous visit's list.
fn load(ui: &AppWindow, state: &AppState, charts_ui: &Arc<ChartsUi>) {
    let g = ui.global::<Charts>();
    if g.get_loading() || !charts_ui.charts.lock().is_empty() {
        return;
    }
    g.set_loading(true);
    g.set_failed(false);

    let token = CHARTS_REQUEST.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let state = state.clone();
    let charts_ui = charts_ui.clone();
    let weak = ui.as_weak();
    state.runtime.spawn(async move {
        let charts = state.cloud.chart_catalogue().await;
        if CHARTS_REQUEST.load(Ordering::Acquire) != token {
            return;
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if CHARTS_REQUEST.load(Ordering::Acquire) != token {
                return;
            }
            let g = ui.global::<Charts>();
            g.set_loading(false);
            match charts {
                Ok(charts) => {
                    let rows = to_rows(&charts);
                    let chunks = chunk(&rows, g.get_columns());
                    *charts_ui.charts.lock() = rows;
                    g.set_rows(ModelRc::from(Rc::new(VecModel::from(chunks))));
                    g.set_failed(false);
                }
                Err(error) => {
                    log::warn!("charts::catalogue: {}", describe(&error));
                    g.set_failed(true);
                }
            }
        });
    });
}

/// The flat cards plus their chunked projection. The chunk borrows the flat list, which the
/// page keeps; `chunk_built_rows` would consume rows a resize still has to re-chunk.
fn chunk(rows: &[SearchEntityRow], columns: i32) -> Vec<ChartGridRow> {
    chunk_rows(rows, columns, Clone::clone, |charts| ChartGridRow { charts })
}

fn to_rows(charts: &[Playlist]) -> Vec<SearchEntityRow> {
    charts.iter().map(chart_row).collect()
}

/// The chart subtitle is every tag the server gave it — Search's own projection — where the
/// playlist row Search paints beside it would have shown the blurb.
fn chart_row(chart: &Playlist) -> SearchEntityRow {
    let artwork = chart
        .cover_url
        .as_deref()
        .filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default();
    SearchEntityRow {
        remote_id: SharedString::from(chart.id.to_string()),
        title: SharedString::from(chart.name.as_str()),
        subtitle: SharedString::from(chart.tags.join(" · ")),
        artwork_path: SharedString::from(artwork),
    }
}

#[cfg(test)]
#[path = "tests/charts_tests.rs"]
mod tests;
