//! The discovery strip's loading line and per-feed retry prompts, pinned in the Slint tree that
//! paints them and in the fetch that raises the flags.

const SEARCH_GLOBAL: &str = include_str!("../../../../../nekonetease-ui/ui/globals/search.slint");
const SEARCH_VIEW: &str = include_str!("../../../../../nekonetease-ui/ui/views/search-view.slint");
const DISCOVER_SECTION: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/search/discover-section.slint");
const CHARTS_SECTION: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/search/charts-section.slint");
const DISCOVER_RS: &str = include_str!("../discover.rs");
const QUERY_CALLBACKS_RS: &str = include_str!("../callbacks/query.rs");

fn collapsed(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_empty_query_state_paints_loading_and_every_failed_feed_offers_a_retry() {
    let source = collapsed(SEARCH_VIEW);
    // One request feeds all three sections, so one line states that all are still on their way.
    assert!(
        source.contains("if Search.query == \"\" && Search.discovery-loading: HorizontalLayout")
    );
    assert!(source.contains("text: @tr(\"Loading…\");"));

    assert!(SEARCH_GLOBAL.contains("in-out property <bool> discovery-loading: false;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <bool> hot-searches-failed: false;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <bool> discovery-failed: false;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <bool> charts-failed: false;"));
    assert!(SEARCH_GLOBAL.contains("callback retry-discovery();"));

    // Each failed feed states itself where its rows would be; one that still has rows keeps them
    // and stays silent.
    assert!(DISCOVER_SECTION.contains(
        "if Search.hot-searches-failed && Search.hot-searches.length == 0: DiscoverFailure"
    ));
    assert!(DISCOVER_SECTION.contains("@tr(\"Couldn't load trending searches\")"));
    assert!(DISCOVER_SECTION.contains(
        "if Search.discovery-failed && Search.discovery-rows.length == 0: DiscoverFailure"
    ));
    assert!(DISCOVER_SECTION.contains("@tr(\"Couldn't load recommendations\")"));
    assert!(
        CHARTS_SECTION
            .contains("if Search.charts-failed && Search.chart-rows.length == 0: VerticalLayout")
    );
    assert!(CHARTS_SECTION.contains("@tr(\"Couldn't load charts\")"));
    assert!(DISCOVER_SECTION.contains("Search.retry-discovery();"));
    assert!(CHARTS_SECTION.contains("Search.retry-discovery();"));
}

#[test]
fn the_fetch_lifts_loading_before_it_spawns_and_lowers_it_with_the_failures() {
    let loading = DISCOVER_RS.find("g.set_discovery_loading(true)");
    let spawn = DISCOVER_RS.find("runtime.spawn");
    assert!(
        matches!((loading, spawn), (Some(loading), Some(spawn)) if loading < spawn),
        "the loading line must be up before the request starts"
    );
    assert!(DISCOVER_RS.contains("g.set_discovery_loading(false);"));
    assert!(DISCOVER_RS.contains("g.set_hot_searches_failed(failures.hot);"));
    assert!(DISCOVER_RS.contains("g.set_discovery_failed(failures.playlists);"));
    assert!(DISCOVER_RS.contains("g.set_charts_failed(failures.charts);"));
}

#[test]
fn a_column_change_re_chunks_the_cached_rows_without_another_request() {
    assert!(DISCOVER_RS.contains("pub(super) fn wire_grids("));
    assert!(DISCOVER_RS.contains("on_discovery_columns_changed("));
    assert!(DISCOVER_RS.contains("on_chart_columns_changed("));
    assert!(DISCOVER_RS.contains("chunk(&rows, columns)"));
    assert!(DISCOVER_RS.contains("chunk(&chart_rows, columns)"));

    let wire = DISCOVER_RS
        .split("pub(super) fn wire_grids")
        .nth(1)
        .unwrap_or_default()
        .split("/// The flat cards plus their chunked projection")
        .next()
        .unwrap_or_default();
    assert!(!wire.contains("ensure_loaded"));
    assert!(!wire.contains("retry"));
}

#[test]
fn a_finished_fetch_lands_its_rows_chunked_for_the_grids() {
    assert!(DISCOVER_RS.contains(
        "g.set_discovery_grid_rows(grid_model(chunk(&rows, g.get_discovery_columns())));"
    ));
    assert!(
        DISCOVER_RS.contains(
            "g.set_chart_grid_rows(grid_model(chunk(&chart_rows, g.get_chart_columns())));"
        )
    );
}

#[test]
fn a_retry_is_wired_and_refuses_to_stack_on_an_in_flight_fetch() {
    assert!(QUERY_CALLBACKS_RS.contains("g.on_retry_discovery(move || {"));
    assert!(QUERY_CALLBACKS_RS.contains("discover::retry(&s, &su, &weak);"));
    assert!(DISCOVER_RS.contains("pub(super) fn retry("));
    assert!(DISCOVER_RS.contains("if ui.global::<Search>().get_discovery_loading() {"));
}
