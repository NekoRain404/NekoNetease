//! `NetEase` search requests, exact-ID projections, local page sorting and history updates.

use std::sync::atomic::Ordering;
use std::time::Duration;

use slint::{ComponentHandle, SharedString, Weak};

use crate::ui::search::{SearchUi, apply};
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::{Album, Artist, Playlist, SearchKind, SearchResult, Song};
use nekonetease_core::error::{AppResult, describe};
use nekonetease_ui::{AppWindow, SearchEntityRow, TrackListRow as UiTrackListRow};

pub(super) const SEARCH_PAGE_SIZE: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchRequest {
    pub(super) kind: SearchKind,
    pub(super) offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CloudTopResult {
    pub(super) kind: &'static str,
    pub(super) id: i64,
    pub(super) remote_id: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) artwork_path: String,
}

pub(super) fn compute_cloud_top_result(
    result: &SearchResult,
    query: &str,
) -> Option<CloudTopResult> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let mut albums = result
        .albums
        .iter()
        .map(|a| CloudTopResult {
            kind: "album",
            id: a.id,
            remote_id: a.id.to_string(),
            title: a.name.clone(),
            subtitle: a.artists.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(" / "),
            artwork_path: a.cover_url.clone().unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let mut artists = result
        .artists
        .iter()
        .map(|a| CloudTopResult {
            kind: "artist",
            id: a.id,
            remote_id: a.id.to_string(),
            title: a.name.clone(),
            subtitle: String::new(),
            artwork_path: a.image_url().unwrap_or_default().to_owned(),
        })
        .collect::<Vec<_>>();
    if albums.is_empty() {
        albums.extend(result.songs.iter().map(|song| CloudTopResult {
            kind: "album",
            id: song.album.id,
            remote_id: song.album.id.to_string(),
            title: song.album.name.clone(),
            subtitle: song.artist_names(),
            artwork_path: song.album.cover_url.clone().unwrap_or_default(),
        }));
    }
    if artists.is_empty() {
        artists.extend(result.songs.iter().flat_map(|song| {
            song.artists.iter().map(|artist| CloudTopResult {
                kind: "artist",
                id: artist.id,
                remote_id: artist.id.to_string(),
                title: artist.name.clone(),
                subtitle: String::new(),
                artwork_path: artist.image_url().unwrap_or_default().to_owned(),
            })
        }));
    }
    let all = albums.iter().chain(artists.iter());
    all.clone()
        .find(|x| x.title.to_lowercase() == needle)
        .or_else(|| all.clone().find(|x| x.title.to_lowercase().starts_with(&needle)))
        .cloned()
        .or_else(|| albums.drain(..).next())
        .or_else(|| artists.drain(..).next())
}

impl SearchRequest {
    pub(super) const fn new(kind: SearchKind, offset: u64) -> Self {
        Self { kind, offset }
    }
}

/// The UI summary and navigation state for one successful page response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SearchPageState {
    pub total: u64,
    pub has_previous: bool,
    pub has_next: bool,
    pub summary: String,
}

impl SearchPageState {
    pub(super) fn new(offset: u64, total: u64, page_count: usize) -> Self {
        let page_count = u64::try_from(page_count).unwrap_or(u64::MAX);
        let start = if page_count == 0 { 0 } else { offset.saturating_add(1) };
        let end = offset.saturating_add(page_count).min(total);
        let summary =
            if page_count == 0 { String::new() } else { format!("{start}–{end} / {total}") };
        Self {
            total,
            has_previous: offset > 0,
            has_next: offset.saturating_add(SEARCH_PAGE_SIZE) < total,
            summary,
        }
    }
}

pub(super) const fn search_kind_key(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Songs => "songs",
        SearchKind::Albums => "albums",
        SearchKind::Artists => "artists",
        SearchKind::Playlists => "playlists",
    }
}

pub(super) fn search_kind_from_key(key: &str) -> Option<SearchKind> {
    match key {
        "songs" => Some(SearchKind::Songs),
        "albums" => Some(SearchKind::Albums),
        "artists" => Some(SearchKind::Artists),
        "playlists" => Some(SearchKind::Playlists),
        _ => None,
    }
}

/// Convert a remote song into the shared track-list shape. `id` is intentionally zero: callers
/// must use `remote_id` for every cloud action and must not feed a 64-bit `NetEase` ID into the
/// local-library `int` path.
pub(super) fn to_cloud_song_row(song: &Song, is_favorite: bool) -> UiTrackListRow {
    let artist = song.artist_names();
    let artwork_path = song
        .album
        .cover_url
        .as_deref()
        .filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default();
    let artist_remote_id = song
        .artists
        .first()
        .filter(|artist| artist.id > 0)
        .map(|artist| artist.id.to_string())
        .unwrap_or_default();
    let album_remote_id = if song.album.id > 0 { song.album.id.to_string() } else { String::new() };
    let duration_ms = i32::try_from(song.duration_ms).unwrap_or(i32::MAX);

    UiTrackListRow {
        id: 0,
        remote_id: SharedString::from(song.id.to_string()),
        title: SharedString::from(song.name.as_str()),
        artist: SharedString::from(artist.as_str()),
        album: SharedString::from(song.album.name.as_str()),
        genre: SharedString::default(),
        year: 0,
        track_number: 0,
        duration_ms,
        is_favorite,
        rating: 0,
        artwork_path: SharedString::from(artwork_path),
        display_duration: SharedString::from(crate::ui::tracks::format_duration_ms(
            i64::try_from(song.duration_ms).unwrap_or(i64::MAX),
        )),
        selected: false,
        enabled: true,
        album_id: 0,
        artist_id: 0,
        genre_id: 0,
        album_remote_id: SharedString::from(album_remote_id),
        artist_remote_id: SharedString::from(artist_remote_id),
        mv_remote_id: song
            .mv_id()
            .map_or_else(SharedString::default, |id| SharedString::from(id.to_string())),
        local_match: SharedString::default(),
    }
}

/// Project only the requested entity collection, keeping its remote ID as decimal text.
pub(super) fn to_search_entity_rows(
    result: &SearchResult,
    kind: SearchKind,
) -> Vec<SearchEntityRow> {
    match kind {
        SearchKind::Songs => Vec::new(),
        SearchKind::Albums => result.albums.iter().map(album_entity_row).collect(),
        SearchKind::Artists => result.artists.iter().map(artist_entity_row).collect(),
        SearchKind::Playlists => result.playlists.iter().map(playlist_entity_row).collect(),
    }
}

fn album_entity_row(album: &Album) -> SearchEntityRow {
    let subtitle =
        album.artists.iter().map(|artist| artist.name.as_str()).collect::<Vec<_>>().join(" / ");
    entity_row(album.id, &album.name, &subtitle, album.cover_url.as_deref())
}

fn artist_entity_row(artist: &Artist) -> SearchEntityRow {
    entity_row(artist.id, &artist.name, "", artist.image_url())
}

pub(super) fn playlist_entity_row(playlist: &Playlist) -> SearchEntityRow {
    entity_row(
        playlist.id,
        &playlist.name,
        playlist.description.as_deref().unwrap_or_default(),
        playlist.cover_url.as_deref(),
    )
}

fn entity_row(id: i64, title: &str, subtitle: &str, artwork: Option<&str>) -> SearchEntityRow {
    let artwork = artwork
        .filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default();
    SearchEntityRow {
        remote_id: SharedString::from(id.to_string()),
        title: SharedString::from(title),
        subtitle: SharedString::from(subtitle),
        artwork_path: SharedString::from(artwork),
    }
}

/// Run the current query and atomically discard any response superseded by another query, kind or
/// page request. The latest successful response is the only one that can update the visible page.
pub(super) async fn kick_search(
    state: &AppState,
    search_ui: &std::sync::Arc<SearchUi>,
    weak: &Weak<AppWindow>,
    query: String,
) -> AppResult<()> {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Ok(());
    }

    let request = SearchRequest::new(
        *search_ui.state().cloud_search_kind.lock(),
        *search_ui.state().cloud_search_offset.lock(),
    );
    kick_search_request(state, search_ui, weak, query, request).await
}

pub(super) async fn kick_search_request(
    state: &AppState,
    search_ui: &std::sync::Arc<SearchUi>,
    weak: &Weak<AppWindow>,
    query: String,
    request: SearchRequest,
) -> AppResult<()> {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Ok(());
    }
    let kind = request.kind;
    let offset = request.offset;
    let token = search_ui.fetch_token.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let is_new_query = search_ui.state().last_query.lock().as_str() != query;
    (*search_ui.state().last_query.lock()).clone_from(&query);
    if is_new_query {
        search_ui.state().cloud_favorite_overrides.lock().clear();
        *search_ui.state().cloud_results.lock() = None;
        search_ui.state().cloud_song_rows.lock().clear();
        search_ui.state().cloud_entity_rows.lock().clear();
    }
    apply::set_loading_on_ui(search_ui, token, weak, true);

    match state.cloud.search(&query, kind, offset).await {
        Ok(result) => {
            if search_ui.fetch_token.load(Ordering::Acquire) != token {
                return Ok(());
            }

            let total = result.total(kind);
            let mut songs = if kind == SearchKind::Songs {
                result
                    .songs
                    .iter()
                    .map(|song| {
                        let liked = search_ui
                            .favorite_override(song.id)
                            .or_else(|| state.cloud.cached_is_liked(song.id))
                            .unwrap_or(false);
                        to_cloud_song_row(song, liked)
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let sort = search_ui.state().sort.lock().clone();
            sort_cloud_rows(&mut songs, &sort.field, sort.dir.as_str());
            let entities = to_search_entity_rows(&result, kind);
            let page_count =
                if kind == SearchKind::Songs { result.songs.len() } else { entities.len() };
            let top = compute_cloud_top_result(&result, &query);
            *search_ui.state().cloud_results.lock() = Some(result);
            (*search_ui.state().cloud_song_rows.lock()).clone_from(&songs);
            (*search_ui.state().cloud_entity_rows.lock()).clone_from(&entities);
            *search_ui.state().cloud_search_kind.lock() = kind;
            *search_ui.state().cloud_search_offset.lock() = offset;
            let page = SearchPageState::new(offset, total, page_count);
            let page_index = i32::try_from(offset / SEARCH_PAGE_SIZE).unwrap_or(i32::MAX);
            apply::apply_cloud_search_results(
                search_ui, token, weak, kind, songs, entities, page, page_index,
            );
            apply::apply_cloud_top_result(search_ui, token, weak, top);

            if is_new_query && offset == 0 {
                let history_token = search_ui.history_token.load(Ordering::Relaxed);
                schedule_history_add(state, search_ui, weak, query, history_token).await;
            }
            Ok(())
        }
        Err(error) => {
            if search_ui.fetch_token.load(Ordering::Acquire) == token {
                apply::set_error_on_ui(search_ui, token, weak, describe(&error));
            }
            Err(error)
        }
    }
}

/// Repaint the cached current page after a sort, compact/full toggle or favorite change.
pub(super) fn reapply_cached_results(search_ui: &std::sync::Arc<SearchUi>, weak: &Weak<AppWindow>) {
    let sort = search_ui.state().sort.lock().clone();
    let rows = {
        let mut rows = search_ui.state().cloud_song_rows.lock();
        sort_cloud_rows(&mut rows, &sort.field, sort.dir.as_str());
        rows.clone()
    };
    *search_ui.state().cloud_song_rows.lock() = rows;
    apply::apply_cached_cloud_rows(search_ui, weak);
}

fn sort_cloud_rows(rows: &mut [UiTrackListRow], field: &str, dir: &str) {
    if field == "rank" {
        return;
    }
    rows.sort_by(|left, right| {
        let order = match field {
            "artist" => left.artist.to_lowercase().cmp(&right.artist.to_lowercase()),
            "album" => left.album.to_lowercase().cmp(&right.album.to_lowercase()),
            "length" => left.duration_ms.cmp(&right.duration_ms),
            "favorite" => left.is_favorite.cmp(&right.is_favorite),
            "track_number" => left.track_number.cmp(&right.track_number),
            "year" => left.year.cmp(&right.year),
            _ => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
        };
        if dir == "desc" { order.reverse() } else { order }
    });
}

/// Add a committed query to the recent list only if the user has stopped editing it for two
/// seconds. Recent history remains local to the application and is independent of the account.
async fn schedule_history_add(
    state: &AppState,
    search_ui: &SearchUi,
    weak: &Weak<AppWindow>,
    query: String,
    token: u64,
) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    if search_ui.history_token.load(Ordering::Relaxed) != token
        || search_ui.state().last_query.lock().as_str() != query
    {
        return;
    }
    match library::search::add_search_history(state, query).await {
        Ok(rows) => {
            (*search_ui.state().recent.lock()).clone_from(&rows);
            push_recent_rows_to_slint(weak, rows);
        }
        Err(error) => log::warn!("search::add_history: {}", describe(&error)),
    }
}

/// Keep the global `recent-rows` model in sync with the persisted history file.
pub(super) fn push_recent_rows_to_slint(weak: &Weak<AppWindow>, rows: Vec<String>) {
    let weak = weak.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<crate::ui::search::Search>();
        g.set_recent_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
            rows.into_iter().map(SharedString::from).collect::<Vec<_>>(),
        ))));
    });
}
