//! The cached result set a favourite or rating edit has to reach, since "Show all", a re-sort and a
//! reapply all repaint from it rather than from the rows on screen.

use nekonetease_core::entities::cloud::{Album, Artist, Playlist, SearchKind, SearchResult, Song};
use nekonetease_core::entities::netease_discovery::HotSearchItem;
use slint::Model;

use super::*;

fn cloud_result() -> SearchResult {
    SearchResult {
        songs: vec![Song {
            id: 4_294_967_297,
            name: "Cloud song".to_owned(),
            artists: vec![Artist {
                id: 4_294_967_298,
                name: "Cloud artist".to_owned(),
                ..Artist::default()
            }],
            album: Album { id: 4_294_967_299, name: "Cloud album".to_owned(), ..Album::default() },
            duration_ms: 65_000,
            ..Song::default()
        }],
        albums: vec![Album {
            id: 4_294_967_301,
            name: "Search album".to_owned(),
            ..Album::default()
        }],
        artists: vec![Artist {
            id: 4_294_967_302,
            name: "Search artist".to_owned(),
            ..Artist::default()
        }],
        playlists: vec![Playlist {
            id: 4_294_967_303,
            name: "Search playlist".to_owned(),
            ..Playlist::default()
        }],
        song_count: 42,
        album_count: 12,
        artist_count: 13,
        playlist_count: 14,
    }
}

#[test]
fn cloud_song_projection_keeps_remote_and_link_ids_as_exact_strings() {
    let song = cloud_result().songs.remove(0);

    let row = super::fetch::to_cloud_song_row(&song, true);

    assert_eq!(row.remote_id.as_str(), "4294967297");
    assert_eq!(row.album_remote_id.as_str(), "4294967299");
    assert_eq!(row.artist_remote_id.as_str(), "4294967298");
    assert_eq!(row.id, 0);
    assert!(row.is_favorite);
}

#[test]
fn a_cloud_song_without_a_music_video_leaves_the_menu_field_empty() {
    let song = Song { mv: 0, ..Song::default() };

    assert_eq!(super::fetch::to_cloud_song_row(&song, false).mv_remote_id.as_str(), "");
}

#[test]
fn a_cloud_song_keeps_a_wide_music_video_id_as_exact_text() {
    // Above 2^32, so a Slint `int` would clamp it.
    let song = Song { mv: 5_436_712_345_678_901, ..Song::default() };

    assert_eq!(
        super::fetch::to_cloud_song_row(&song, false).mv_remote_id.as_str(),
        "5436712345678901"
    );
}

#[test]
fn cloud_entity_projection_keeps_each_remote_id_as_exact_strings() {
    let result = cloud_result();

    let albums = super::fetch::to_search_entity_rows(&result, SearchKind::Albums);
    let artists = super::fetch::to_search_entity_rows(&result, SearchKind::Artists);
    let playlists = super::fetch::to_search_entity_rows(&result, SearchKind::Playlists);

    assert_eq!(albums.first().map(|row| row.remote_id.as_str()), Some("4294967301"));
    assert_eq!(artists.first().map(|row| row.remote_id.as_str()), Some("4294967302"));
    assert_eq!(playlists.first().map(|row| row.remote_id.as_str()), Some("4294967303"));
}

#[test]
fn page_state_uses_api_total_and_reports_available_directions() {
    let first = super::fetch::SearchPageState::new(0, 61, 30);
    assert_eq!(
        (first.total, first.has_previous, first.has_next, first.summary.as_str()),
        (61, false, true, "1–30 / 61")
    );

    let last = super::fetch::SearchPageState::new(60, 61, 1);
    assert_eq!(
        (last.total, last.has_previous, last.has_next, last.summary.as_str()),
        (61, true, false, "61–61 / 61")
    );
}

#[test]
fn page_request_keeps_kind_and_offset_snapshot() {
    let request = super::fetch::SearchRequest::new(SearchKind::Albums, 60);
    assert_eq!(request.kind, SearchKind::Albums);
    assert_eq!(request.offset, 60);
}

#[test]
fn cloud_top_result_prefers_exact_album_then_artist() {
    let result = cloud_result();
    let top = super::fetch::compute_cloud_top_result(&result, "Search album");
    assert!(top.is_some(), "an exact album match must produce a top result");
    let Some(top) = top else { return };
    assert_eq!((top.kind, top.id), ("album", 4_294_967_301));
}

#[test]
fn cloud_song_results_supply_album_top_result_candidates() {
    let mut result = cloud_result();
    result.albums.clear();
    result.artists.clear();
    result.songs[0].album.name = "Cloud song".to_owned();
    let top = super::fetch::compute_cloud_top_result(&result, "Cloud song");
    assert!(top.is_some(), "a song's album must supply a top result");
    let Some(top) = top else { return };
    assert_eq!(top.kind, "album");
    assert_eq!(top.id, result.songs[0].album.id);
}

#[test]
fn cloud_top_result_preserves_remote_ids_above_slint_int_range() {
    let mut result = cloud_result();
    result.albums[0].id = i64::from(i32::MAX) + 42;
    let top = super::fetch::compute_cloud_top_result(&result, "Search album");
    assert!(top.is_some(), "the exact album match survives an id above i32::MAX");
    let Some(top) = top else { return };
    assert_eq!(top.id.to_string(), "2147483689");
    assert_eq!(top.remote_id, "2147483689");
    assert_eq!(
        super::callbacks::resolve_top_result_id(&top.remote_id, i32::MAX),
        Some(2_147_483_689)
    );
}

#[test]
fn reentering_with_query_and_empty_models_requires_refetch() {
    assert!(super::callbacks::should_refetch_on_enter("metal", false, 0, 0));
    assert!(!super::callbacks::should_refetch_on_enter("", false, 0, 0));
    assert!(!super::callbacks::should_refetch_on_enter("metal", true, 0, 0));
    assert!(!super::callbacks::should_refetch_on_enter("metal", false, 1, 0));
}

#[test]
fn recent_mutation_token_invalidates_older_completion() {
    let ui = SearchUi::new();
    let first = ui.recent_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    let second = ui.recent_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    assert_ne!(first, second);
    assert_ne!(ui.recent_token.load(std::sync::atomic::Ordering::Acquire), first);
}

#[test]
fn cloud_search_kind_keys_cover_every_remote_category() {
    assert_eq!(super::fetch::search_kind_from_key("songs"), Some(SearchKind::Songs));
    assert_eq!(super::fetch::search_kind_from_key("albums"), Some(SearchKind::Albums));
    assert_eq!(super::fetch::search_kind_from_key("artists"), Some(SearchKind::Artists));
    assert_eq!(super::fetch::search_kind_from_key("playlists"), Some(SearchKind::Playlists));
    assert_eq!(super::fetch::search_kind_from_key("local"), None);
}

#[test]
fn a_favourite_flip_patches_the_cached_cloud_row_it_names_and_no_other() {
    let ui = SearchUi::new();
    let first = cloud_result().songs.remove(0);
    let second = Song { id: 2, name: "Other".to_owned(), ..Song::default() };
    *ui.inner.cloud_song_rows.lock() = vec![
        super::fetch::to_cloud_song_row(&first, false),
        super::fetch::to_cloud_song_row(&second, false),
    ];

    ui.flip_favorite(first.id, true);

    let rows = ui.inner.cloud_song_rows.lock();
    assert!(rows[0].is_favorite);
    assert!(!rows[1].is_favorite);
    assert_eq!(ui.favorite_override(first.id), Some(true));
}

fn chart(id: i64, name: &str, tags: &[&str]) -> Playlist {
    Playlist {
        id,
        name: name.to_owned(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        ..Playlist::default()
    }
}

#[test]
fn chart_rows_keep_the_server_order_and_every_remote_id() {
    let rows = super::discover::to_chart_rows(&[
        chart(1, "飙升榜", &[]),
        chart(2, "新歌榜", &["华语", "榜单"]),
        chart(4_294_967_297, "古典音乐榜", &["古典"]),
    ]);

    let ids = rows.iter().map(|row| row.remote_id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, ["1", "2", "4294967297"]);
    let titles = rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>();
    assert_eq!(titles, ["飙升榜", "新歌榜", "古典音乐榜"]);
}

#[test]
fn chart_rows_show_every_server_tag_and_no_invented_category() {
    let rows = super::discover::to_chart_rows(&[
        chart(1, "古典音乐榜", &["古典", "榜单"]),
        chart(2, "飙升榜", &[]),
    ]);

    assert_eq!(rows[0].subtitle.as_str(), "古典 · 榜单");
    assert_eq!(rows[1].subtitle.as_str(), "");
}

#[test]
fn the_chart_catalogue_is_not_capped_like_the_recommended_strip() {
    let charts = (0..65)
        .map(|index| chart(i64::from(index) + 1, &format!("Chart {index}"), &[]))
        .collect::<Vec<_>>();

    let rows = super::discover::to_chart_rows(&charts);

    assert_eq!(rows.len(), 65);
}

const SEARCH_GLOBAL: &str = include_str!("../../../../../nekonetease-ui/ui/globals/search.slint");
const SEARCH_VIEW: &str = include_str!("../../../../../nekonetease-ui/ui/views/search-view.slint");
const DISCOVER_SECTION: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/search/discover-section.slint");
const CHARTS_SECTION: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/search/charts-section.slint");
const DISCOVER_RS: &str = include_str!("../discover.rs");

#[test]
fn the_global_carries_every_discovery_model_and_the_trending_pick() {
    assert!(SEARCH_GLOBAL.contains("in-out property <[string]> hot-searches;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <[SearchEntityRow]> discovery-rows;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <[SearchEntityRow]> chart-rows;"));
    assert!(SEARCH_GLOBAL.contains("callback hot-search-pick(string);"));
    assert!(SEARCH_GLOBAL.contains("callback open-chart(string /*remote ID*/);"));
}

#[test]
fn the_global_carries_a_grid_row_type_and_a_column_count_for_each_feed() {
    assert!(SEARCH_GLOBAL.contains("export struct SearchGridRow {"));
    assert!(SEARCH_GLOBAL.contains("entities: [SearchEntityRow],"));
    assert!(SEARCH_GLOBAL.contains("in-out property <[SearchGridRow]> discovery-grid-rows;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <[SearchGridRow]> chart-grid-rows;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <int> discovery-columns: 4;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <int> chart-columns: 4;"));
    assert!(SEARCH_GLOBAL.contains("callback discovery-columns-changed(int /*columns*/);"));
    assert!(SEARCH_GLOBAL.contains("callback chart-columns-changed(int /*columns*/);"));
}

#[test]
fn the_empty_query_branch_mounts_the_discovery_strip_rather_than_the_result_branches() {
    let source = SEARCH_VIEW.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        source.contains("import { SearchDiscoverSection } from \"search/discover-section.slint\";")
    );
    assert!(source.contains(
        "if Search.query == \"\" && (Search.hot-searches.length > 0 || Search.discovery-rows.length > 0 || Search.hot-searches-failed || Search.discovery-failed): SearchDiscoverSection"
    ));
    // The catalogue is uncapped, so it mounts as its own section rather than inside the strip.
    assert!(source.contains(
        "if Search.query == \"\" && (Search.chart-rows.length > 0 || Search.charts-failed): ChartsSection"
    ));
    assert!(source.contains("import { ChartsSection } from \"search/charts-section.slint\";"));
}

#[test]
fn the_strip_routes_chips_and_playlists_through_the_existing_actions() {
    assert!(DISCOVER_SECTION.contains("Search.hot-search-pick(self.term);"));
    assert!(DISCOVER_SECTION.contains("Playlists.open-playlist(id);"));
    // The card's remote ID stays text in the callback, as the result rows' does.
    assert!(DISCOVER_SECTION.contains("callback open(string /*exact NetEase ID*/);"));
    assert!(CHARTS_SECTION.contains("Search.open-chart(id);"));
    assert!(CHARTS_SECTION.contains("Search.blur-search-tick += 1;"));
}

#[test]
fn both_feeds_draw_the_shared_card_grid_rather_than_a_row_list() {
    let discover = DISCOVER_SECTION.split_whitespace().collect::<Vec<_>>().join(" ");
    let charts = CHARTS_SECTION.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(discover.contains("export component DiscoverCardGrid inherits Rectangle"));
    assert!(discover.contains("rows: Search.discovery-grid-rows;"));
    assert!(charts.contains("import { DiscoverCardGrid } from \"discover-section.slint\";"));
    assert!(charts.contains("rows: Search.chart-grid-rows;"));
    // A column count the width derives is reported once, so Rust re-chunks from the cache.
    assert!(discover.contains("Search.discovery-columns-changed(c);"));
    assert!(charts.contains("Search.chart-columns-changed(c);"));
    // The section is a plain column; only the grid's own rows line its cards up.
    assert!(discover.contains("alignment: start;"));
    assert!(!discover.contains("alignment: center"));
}

const RESULTS_RS: &str = include_str!("../callbacks/results.rs");

#[test]
fn a_chart_row_mounts_the_playlists_tab_before_the_detail_arrives() {
    assert!(RESULTS_RS.contains("on_open_chart("));
    assert!(RESULTS_RS.contains("invoke_open_playlist("));
    // `open_playlist_with` writes `playlist-id` a fetch later, so the tab has to move first.
    let flip = RESULTS_RS.find("go_to_tab(&ui, ui.global::<MyLibrary>().get_tab_playlists())");
    let open = RESULTS_RS.find("invoke_open_playlist(");
    assert!(matches!((flip, open), (Some(flip), Some(open)) if flip < open));
}

#[test]
fn discovery_fetches_every_public_feed_once_and_logs_each_failure() {
    assert!(DISCOVER_RS.contains("state.cloud.hot_search()"));
    assert!(DISCOVER_RS.contains("state.cloud.discover()"));
    assert!(DISCOVER_RS.contains("state.cloud.chart_catalogue()"));
    assert!(DISCOVER_RS.contains("tokio::join!"));
    assert!(DISCOVER_RS.contains("log::warn!(\"search::hot_search: {}\""));
    assert!(DISCOVER_RS.contains("log::warn!(\"search::discover: {}\""));
    assert!(DISCOVER_RS.contains("log::warn!(\"search::charts: {}\""));
}

#[test]
fn both_feeds_chunk_into_the_grid_column_count() {
    let playlists = (0..5)
        .map(|index| Playlist {
            id: i64::from(index),
            name: format!("Playlist {index}"),
            ..Playlist::default()
        })
        .collect::<Vec<_>>();
    let rows = super::discover::to_playlist_rows(&playlists);

    let grid = super::discover::chunk(&rows, 2);

    assert_eq!(grid.len(), 3);
    let widths = grid.iter().map(|row| row.entities.row_count()).collect::<Vec<_>>();
    assert_eq!(widths, [2, 2, 1]);
}

#[test]
fn recommended_playlist_rows_keep_exact_remote_ids_and_cap_the_strip() {
    let playlists = (0..(super::discover::DISCOVER_LIMIT + 4))
        .map(|index| Playlist {
            id: 4_294_967_297 + i64::try_from(index).unwrap_or(i64::MAX),
            name: format!("Playlist {index}"),
            ..Playlist::default()
        })
        .collect::<Vec<_>>();

    let rows = super::discover::to_playlist_rows(&playlists);

    assert_eq!(rows.len(), super::discover::DISCOVER_LIMIT);
    assert_eq!(rows.first().map(|row| row.remote_id.as_str()), Some("4294967297"));
}

#[test]
fn hot_search_words_keep_the_feed_order_and_cap_the_strip() {
    let items = (0..(super::discover::HOT_SEARCH_LIMIT + 3))
        .map(|index| HotSearchItem { word: format!("term-{index}"), ..HotSearchItem::default() })
        .collect::<Vec<_>>();

    let words = super::discover::to_hot_words(&items);

    assert_eq!(words.len(), super::discover::HOT_SEARCH_LIMIT);
    assert_eq!(words.first().map(slint::SharedString::as_str), Some("term-0"));
}

#[test]
fn leaving_the_section_releases_the_chart_rows_with_the_other_discovery_feeds() {
    let ui = SearchUi::new();
    *ui.inner.chart_rows.lock() = super::discover::to_chart_rows(&[
        chart(4_294_967_297, "Chart", &[]),
        chart(2, "Tagged", &["古典"]),
    ]);

    ui.release_section_state();

    assert!(ui.inner.chart_rows.lock().is_empty());
    assert!(ui.inner.discovery_rows.lock().is_empty());
}

#[test]
fn discovery_token_invalidates_an_older_completion() {
    let ui = SearchUi::new();
    let first = ui.discover_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    let second = ui.discover_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;

    assert_ne!(first, second);
    assert_ne!(ui.discover_token.load(std::sync::atomic::Ordering::Acquire), first);
}

const SUGGESTIONS_POPUP: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/search/suggestions-popup.slint");
const SUGGEST_RS: &str = include_str!("../suggest.rs");
const QUERY_CALLBACKS_RS: &str = include_str!("../callbacks/query.rs");
const SUGGEST_CALLBACKS_RS: &str = include_str!("../callbacks/suggest.rs");
const LIFECYCLE_RS: &str = include_str!("../callbacks/lifecycle.rs");

#[test]
fn suggestion_token_invalidates_an_older_completion() {
    let ui = SearchUi::new();
    let first = ui.suggestion_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    let second = ui.suggestion_token.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;

    assert_ne!(first, second);
    assert_ne!(ui.suggestion_token.load(std::sync::atomic::Ordering::Acquire), first);
}

#[test]
fn the_global_carries_the_suggestion_model_and_the_commit_marker() {
    assert!(SEARCH_GLOBAL.contains("in-out property <[string]> suggestions;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <string> suggestions-query;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <string> committed-query;"));
    assert!(SEARCH_GLOBAL.contains("in-out property <bool> suggestions-pending: false;"));
    assert!(SEARCH_GLOBAL.contains("callback request-suggestions(string);"));
    assert!(SEARCH_GLOBAL.contains("callback suggestion-pick(string);"));
    assert!(SEARCH_GLOBAL.contains("callback dismiss-suggestions();"));
}

#[test]
fn the_view_asks_on_its_own_throttle_and_mounts_the_popup_under_the_bar() {
    let source = SEARCH_VIEW.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(source.contains("pending: Search.suggest-tick-pending;"));
    // Settled, not cadenced: each keystroke otherwise spends a request on a value nobody asked for.
    assert!(source.contains("trailing: true;"));
    assert!(source.contains(
        "if Search.query != Search.committed-query { Search.request-suggestions(Search.query); }"
    ));
    // The popup is over an uncommitted edit and only ever paints the text it answered.
    assert!(source.contains("Search.query != Search.committed-query"));
    assert!(source.contains("Search.query == Search.suggestions-query"));
    assert!(source.contains("SuggestionsPopup { x: (root.width - self.width) / 2;"));
    assert!(source.contains("picked(term) => { Search.suggestion-pick(term); }"));
    assert!(source.contains("Search.suggest-tick-pending += 1;"));
    // The commit waits for the exchange the pause produced, or the popup would be unmounted
    // before its answer ever paints.
    assert!(source.contains("if root.suggestion-live { Search.debounce-tick-pending += 1; } else"));
    assert!(source.contains("Search.dismiss-suggestions();"));
}

#[test]
fn the_suggestion_popup_sizes_off_its_model_and_uses_the_popup_chrome() {
    assert!(SUGGESTIONS_POPUP.contains("inherits PopupSurface"));
    assert!(
        SUGGESTIONS_POPUP
            .contains("height: root.padding-y * 2 + root.rows.length * root.row-height;")
    );
    assert!(SUGGESTIONS_POPUP.contains("overflow: elide;"));
    assert!(SUGGESTIONS_POPUP.contains("callback picked(string);"));
    assert!(SUGGESTIONS_POPUP.contains("clicked => { root.picked(label); }"));
}

#[test]
fn a_suggestion_fetch_is_gated_on_the_uncommitted_query_and_logs_a_failure() {
    assert!(SUGGEST_RS.contains("search_ui.state().last_query.lock().as_str() == needle"));
    assert!(SUGGEST_RS.contains("state.cloud.search_suggestions(&needle)"));
    assert!(SUGGEST_RS.contains("log::warn!(\"search::suggest: {}\", describe(&error))"));
    assert!(SUGGEST_RS.contains("search_ui.suggestion_token.load(Ordering::Acquire) != token"));
    assert!(SUGGEST_RS.contains("g.get_committed_query().trim() == reply.needle.as_str()"));
    // Every request, including an early bail and a failure, settles the flag the commit waits on.
    assert!(SUGGEST_RS.contains("g.set_suggestions_pending(false);"));
    assert!(SUGGEST_RS.contains("clear_on_ui(search_ui, token, weak);"));
}

#[test]
fn the_empty_box_and_a_section_leave_invalidate_a_pending_suggestion() {
    assert!(QUERY_CALLBACKS_RS.contains("su.suggestion_token.fetch_add(1, Ordering::AcqRel);"));
    assert!(QUERY_CALLBACKS_RS.contains("set_committed_query(SharedString::from(q.as_str()))"));
    assert!(LIFECYCLE_RS.contains("su.suggestion_token.fetch_add(1, Ordering::AcqRel);"));
    assert!(SUGGEST_CALLBACKS_RS.contains("g.set_committed_query(term);"));
    assert!(SUGGEST_CALLBACKS_RS.contains("g.on_dismiss_suggestions(move || {"));
    assert!(SUGGEST_CALLBACKS_RS.contains("ui.global::<Search>().set_suggestions_pending(true);"));
}
