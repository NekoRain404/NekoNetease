//! `Search.*` query callbacks: keystroke debounce, commit → cloud fetch,
//! and the compact/full Songs toggle. See [`super::wire`].

use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, SharedString};

use crate::ui::callbacks::macros::spawn_logged;
use crate::ui::search::{self as search_ui_mod, SearchUi, apply, discover, fetch};
use nekonetease_app::state::AppState;
use nekonetease_ui::{AppWindow, Search};

/// Wire the query / debounce / show-all callbacks.
pub(super) fn wire(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();

    // Every keystroke: mirror the trimmed text into Rust state, reset
    // `show-all-tracks` (Tauri parity: every new query starts compact),
    // bump the history token (defers any pending 2-second history-add
    // for a previous query), and fast-path the empty case so the
    // recent-searches branch re-mounts as soon as the user clears
    // the SearchBar.
    //
    // Empty-input also releases the strip-tier LRUs + cached results
    // off-thread: the user explicitly cleared the field, so we treat
    // it like a soft section-leave (mirrors `release_section_state`'s
    // ordering but without the section-active bail). UI-side
    // selection is dropped synchronously so the leftover
    // "{N} selected" pill from the previous query doesn't linger.
    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_query_changed(move |text| {
            su.history_token.fetch_add(1, Ordering::Relaxed);
            if let Some(ui) = weak.upgrade() {
                ui.global::<Search>().set_show_all_tracks(false);
            }
            if text.trim().is_empty() {
                su.fetch_token.fetch_add(1, Ordering::AcqRel);
                su.suggestion_token.fetch_add(1, Ordering::AcqRel);
                su.state().last_query.lock().clear();
                *su.state().cloud_search_offset.lock() = 0;
                su.state().cloud_results.lock().take();
                su.state().cloud_song_rows.lock().clear();
                su.state().cloud_entity_rows.lock().clear();
                su.state().cloud_favorite_overrides.lock().clear();
                apply::clear_results_on_ui(&su, &weak);
                if let Some(ui) = weak.upgrade() {
                    search_ui_mod::clear_selection(&ui, &su);
                }
                let release_ui = su.clone();
                s.runtime.spawn_blocking(move || release_ui.release_for_empty_query());
                // Clearing the box is the other way into the empty state, so it owes the same
                // discovery ask as entering the section does.
                crate::ui::search::discover::ensure_loaded(&s, &su, &weak);
            }
        });
    }
    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_commit_search(move |text| {
            let s = s.clone();
            let su = su.clone();
            let weak = weak.clone();
            let q = text.to_string();
            // The commit, not `query-changed` beside it, which fires per
            // keystroke. Behind the view's 300 ms debounce or an Enter press.
            log::debug!("search: {q:?}");
            if let Some(ui) = weak.upgrade() {
                ui.global::<Search>().set_committed_query(SharedString::from(q.as_str()));
            }
            // Reset selection if this commit targets a different
            // query than the one whose results currently populate
            // the model. Previous selection ids reference tracks
            // that may not appear in the new result set, so the
            // "{N} selected" pill would lie. Compare against the
            // cached `last_query` (not `Search.query`, which is
            // the live keystroke text — they're equal at commit
            // time only when the throttle Timer just fired against
            // a quiet field).
            if q.trim() != su.state().last_query.lock().as_str()
                && let Some(ui) = weak.upgrade()
            {
                search_ui_mod::clear_selection(&ui, &su);
                *su.state().cloud_search_offset.lock() = 0;
            }
            let request = fetch::SearchRequest::new(
                *su.state().cloud_search_kind.lock(),
                *su.state().cloud_search_offset.lock(),
            );
            spawn_logged!(
                s,
                "search::commit",
                fetch::kick_search_request(&s, &su, &weak, q, request)
            );
        });
    }
    {
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_toggle_show_all_tracks(move || {
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<Search>();
                g.set_show_all_tracks(!g.get_show_all_tracks());
            }
            fetch::reapply_cached_results(&su, &weak);
        });
    }
    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_search_kind_changed(move |key| {
            let Some(kind) = fetch::search_kind_from_key(key.as_str()) else { return };
            *su.state().cloud_search_kind.lock() = kind;
            *su.state().cloud_search_offset.lock() = 0;
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Search>();
            let query = g.get_query().to_string();
            if query.trim().is_empty() {
                return;
            }
            // A kind pick searches the box's text, so it is committed like any other search and
            // the suggestion popup has nothing left to offer over it.
            g.set_committed_query(SharedString::from(query.as_str()));
            let s = s.clone();
            let su = su.clone();
            let weak = weak.clone();
            let request = fetch::SearchRequest::new(kind, 0);
            spawn_logged!(
                s,
                "search::change_kind",
                fetch::kick_search_request(&s, &su, &weak, query, request)
            );
        });
    }
    wire_request_page(ui, state, search_ui);
    wire_retry_search(ui, state, search_ui);
    wire_retry_discovery(ui, state, search_ui);
    wire_hot_search_pick(ui);
}

fn wire_request_page(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();
    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_request_page(move |direction| {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Search>();
            let current = *su.state().cloud_search_offset.lock();
            let next = match direction {
                -1 if g.get_has_previous_page() => current.saturating_sub(fetch::SEARCH_PAGE_SIZE),
                1 if g.get_has_next_page() => current.saturating_add(fetch::SEARCH_PAGE_SIZE),
                _ => return,
            };
            let kind = *su.state().cloud_search_kind.lock();
            let request = fetch::SearchRequest::new(kind, next);
            *su.state().cloud_search_offset.lock() = next;
            let query = g.get_query().to_string();
            if query.trim().is_empty() {
                return;
            }
            let s = s.clone();
            let su = su.clone();
            let weak = weak.clone();
            spawn_logged!(
                s,
                "search::page",
                fetch::kick_search_request(&s, &su, &weak, query, request)
            );
        });
    }
}

fn wire_retry_search(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();
    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_retry_search(move || {
            let query = su.state().last_query.lock().clone();
            if query.trim().is_empty() {
                return;
            }
            let s = s.clone();
            let su = su.clone();
            let weak = weak.clone();
            let request = fetch::SearchRequest::new(
                *su.state().cloud_search_kind.lock(),
                *su.state().cloud_search_offset.lock(),
            );
            spawn_logged!(
                s,
                "search::retry",
                fetch::kick_search_request(&s, &su, &weak, query, request)
            );
        });
    }
}

fn wire_retry_discovery(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();
    let s = state.clone();
    let su = search_ui.clone();
    g.on_retry_discovery(move || {
        discover::retry(&s, &su, &weak);
    });
}

fn wire_hot_search_pick(ui: &AppWindow) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();
    // A trending chip is the recent-chip path: fill the box and let the view's throttle commit it.
    {
        let weak = weak.clone();
        g.on_hot_search_pick(move |term| {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Search>();
            g.set_query(term);
            g.set_debounce_tick_pending(g.get_debounce_tick_pending() + 1);
        });
    }
}
