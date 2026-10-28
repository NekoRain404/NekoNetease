//! The Search page: a query-driven list of `NetEase` songs over one cloud search per debounced
//! keystroke.
//!
//! **Songs** is a cloud-aware `TrackList` showing a compact prefix by default; "Show all" swaps to
//! the rest of the current cloud page from its cache, with no second network request. Recent
//! searches show only on an empty box, and a committed query joins them only after a pause.
//!
//! Results stay query-driven rather than refreshing under the user. A cloud favorite change patches
//! the cached row and the one on screen so sorting or expanding the result doesn't restore stale
//! state.

mod apply;
mod callbacks;
mod discover;
mod fetch;
mod selection;
mod state;
mod suggest;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use crate::ui::albums::AlbumsUi;
use crate::ui::artists::ArtistsUi;
use crate::ui::util::{clamp_i64_to_i32, opt_shared};
use crate::ui::view_ctx::ViewCtx;
use nekonetease_artwork::media::image::cover_thumbs::CoverThumbs;
use nekonetease_core::entities::album::AlbumStats;
use nekonetease_core::entities::artist::ArtistStats;
use nekonetease_ui::{
    AppWindow, EntityStripRow as UiEntityStripRow, Search, SearchEntityRow,
    TrackListRow as UiTrackListRow,
};

use state::{ALBUM_STRIP_THUMB_SIZE, ARTIST_STRIP_THUMB_SIZE, STRIP_THUMB_CAP, SearchUiState};

pub(super) use apply::apply_row_favorite;
pub(super) use selection::{clear_selection, handle_select_row, select_all};

/// Install the Search models, build the handle, and wire every `Search.*`
/// callback to it.
///
/// The two peers are the cross-tab hand-offs, taken as parameters so the
/// ordering is a compile error rather than a comment. There is no initial
/// fetch — the page paints empty until the user types.
///
/// The returned handle is not a keepalive; see [`crate::ui::albums::install`].
pub fn install(
    cx: ViewCtx<'_>,
    albums_ui: &Arc<AlbumsUi>,
    artists_ui: &Arc<ArtistsUi>,
) -> Arc<SearchUi> {
    install_models(cx.app);
    let search_ui = Arc::new(SearchUi::new());
    callbacks::wire(cx.app, cx.state, cx.view_state, &search_ui, albums_ui, artists_ui);
    search_ui
}

/// Rust-side state for the Search view, shared between the UI callbacks and the
/// async fetchers behind an `Arc`.
pub struct SearchUi {
    inner: SearchUiState,
    /// The two strip tiers, each also serving Top Result when it is of that
    /// kind — so clicking a strip card after the top card hits a warm LRU.
    pub(super) album_strip_thumbs: Arc<CoverThumbs>,
    pub(super) artist_strip_thumbs: Arc<CoverThumbs>,
    /// Section-visible shadow. `release_section_state` bails on it, so a quickly
    /// re-entered section doesn't have its caches cleared mid-refresh.
    section_active: AtomicBool,
    /// Bumped by every `commit-search`; the in-flight `kick_search` re-reads it
    /// once `search_all` resolves and drops its UI write if a newer keystroke
    /// superseded it. `BrowseUi::fetch_token`'s shape.
    pub(super) fetch_token: AtomicU64,
    /// Bumped by every keystroke; the delayed `schedule_history_add` bails if it
    /// moved, a newer keystroke meaning the user is still typing.
    pub(super) history_token: AtomicU64,
    pub(super) recent_token: AtomicU64,
    /// Bumped on section leave and by each discovery fetch; a response that resolves after the
    /// page left drops its repaint rather than writing into a section the user no longer sees.
    pub(super) discover_token: AtomicU64,
    /// Bumped by each suggestion request, by an empty box, a section leave, a pick and a
    /// dismissal; a response that no longer owns the token drops rather than painting over a
    /// newer query.
    pub(super) suggestion_token: AtomicU64,
}

impl SearchUi {
    fn new() -> Self {
        Self {
            inner: SearchUiState::new(),
            album_strip_thumbs: Arc::new(CoverThumbs::with_config(
                ALBUM_STRIP_THUMB_SIZE,
                STRIP_THUMB_CAP,
            )),
            artist_strip_thumbs: Arc::new(CoverThumbs::with_config(
                ARTIST_STRIP_THUMB_SIZE,
                STRIP_THUMB_CAP,
            )),
            section_active: AtomicBool::new(false),
            fetch_token: AtomicU64::new(0),
            history_token: AtomicU64::new(0),
            recent_token: AtomicU64::new(0),
            discover_token: AtomicU64::new(0),
            suggestion_token: AtomicU64::new(0),
        }
    }

    pub fn set_section_active(&self, active: bool) {
        self.section_active.store(active, Ordering::Relaxed);
    }

    pub fn section_active(&self) -> bool {
        self.section_active.load(Ordering::Relaxed)
    }

    /// Drop every section-local buffer and reset the cached results, so a hidden
    /// page holds nothing. Runs off the UI thread on section leave, bailing when
    /// the section is already active again — a user who flipped tabs and back
    /// before this ran. Release order is
    /// `FavoritesUi::release_section_state`'s.
    pub fn release_section_state(&self) {
        if self.section_active() {
            return;
        }
        self.fetch_token.fetch_add(1, Ordering::AcqRel);
        self.discover_token.fetch_add(1, Ordering::AcqRel);
        self.suggestion_token.fetch_add(1, Ordering::AcqRel);
        self.album_strip_thumbs.clear();
        self.artist_strip_thumbs.clear();
        *self.inner.cloud_results.lock() = None;
        self.inner.cloud_song_rows.lock().clear();
        self.inner.cloud_entity_rows.lock().clear();
        self.inner.cloud_search_offset.lock().clone_from(&0);
        self.inner.cloud_favorite_overrides.lock().clear();
        self.inner.last_query.lock().clear();
        self.inner.applied_selection.lock().clear();
        // The discovery strip is per-visit: its one fetch is cheap and the directory moves, so a
        // re-entry asks again instead of painting whatever was recommended an hour ago.
        self.inner.hot_searches.lock().clear();
        self.inner.discovery_rows.lock().clear();
        self.inner.chart_rows.lock().clear();
        *self.inner.discovery_loaded.lock() = false;
        nekonetease_platform::services::platform::allocator::trim();
    }

    /// [`Self::release_section_state`]'s release, run while the user is still on
    /// Search — when the box goes empty. No `section_active()` bail: an explicit
    /// clear says "start fresh" rather than "mid-edit", and the empty state's
    /// Recent chips touch no strip LRU, so the freed pages stay freed until the
    /// next query commits.
    ///
    /// The trade is that clearing and retyping the same query re-decodes a
    /// strip's worth of covers. Worth it against keeping thumbnails warm with
    /// nothing on screen to display them.
    pub fn release_for_empty_query(&self) {
        self.fetch_token.fetch_add(1, Ordering::AcqRel);
        self.suggestion_token.fetch_add(1, Ordering::AcqRel);
        self.album_strip_thumbs.clear();
        self.artist_strip_thumbs.clear();
        *self.inner.cloud_results.lock() = None;
        self.inner.cloud_song_rows.lock().clear();
        self.inner.cloud_entity_rows.lock().clear();
        self.inner.cloud_search_offset.lock().clone_from(&0);
        self.inner.cloud_favorite_overrides.lock().clear();
        self.inner.last_query.lock().clear();
        self.inner.applied_selection.lock().clear();
        nekonetease_platform::services::platform::allocator::trim();
    }

    /// Flip the cached `NetEase` song's favorite state so a local re-apply does not restore the old
    /// value. Remote IDs stay `i64` here and use strings only at the Slint boundary.
    pub fn flip_favorite(&self, id: i64, fav: bool) {
        self.inner.cloud_favorite_overrides.lock().insert(id, fav);
        for row in self.inner.cloud_song_rows.lock().iter_mut() {
            if row.remote_id.as_str().parse::<i64>().ok() == Some(id) {
                row.is_favorite = fav;
            }
        }
    }

    pub(super) fn favorite_override(&self, id: i64) -> Option<bool> {
        self.inner.cloud_favorite_overrides.lock().get(&id).copied()
    }

    pub(crate) fn state(&self) -> &SearchUiState {
        &self.inner
    }

    /// Backs `Search.request-album-strip-cover`.
    pub fn album_strip_cover(&self, artwork_path: &str) -> Image {
        self.album_strip_thumbs.get_or_load_opt(Some(artwork_path).filter(|s| !s.is_empty()))
    }

    /// Backs `Search.request-artist-strip-cover`.
    pub fn artist_strip_cover(&self, artwork_path: &str) -> Image {
        self.artist_strip_thumbs.get_or_load_opt(Some(artwork_path).filter(|s| !s.is_empty()))
    }
}

/// Hand the global its empty `VecModel`s. Later updates find them by
/// downcasting back, on the UI thread.
fn install_models(ui: &AppWindow) {
    let g = ui.global::<Search>();

    let tracks: Rc<VecModel<UiTrackListRow>> = Rc::new(VecModel::default());
    g.set_tracks(ModelRc::from(tracks));

    let albums: Rc<VecModel<UiEntityStripRow>> = Rc::new(VecModel::default());
    g.set_album_rows(ModelRc::from(albums));

    let artists: Rc<VecModel<UiEntityStripRow>> = Rc::new(VecModel::default());
    g.set_artist_rows(ModelRc::from(artists));

    let recent: Rc<VecModel<SharedString>> = Rc::new(VecModel::default());
    g.set_recent_rows(ModelRc::from(recent));

    let hot: Rc<VecModel<SharedString>> = Rc::new(VecModel::default());
    g.set_hot_searches(ModelRc::from(hot));

    let discovery: Rc<VecModel<SearchEntityRow>> = Rc::new(VecModel::default());
    g.set_discovery_rows(ModelRc::from(discovery));

    let charts: Rc<VecModel<SearchEntityRow>> = Rc::new(VecModel::default());
    g.set_chart_rows(ModelRc::from(charts));

    let suggestions: Rc<VecModel<SharedString>> = Rc::new(VecModel::default());
    g.set_suggestions(ModelRc::from(suggestions));

    let sel: Rc<VecModel<i32>> = Rc::new(VecModel::default());
    g.set_selected_ids(ModelRc::from(sel));
}

/// An `AlbumStats` as its strip row, subtitled with the artist name for parity
/// with the Most Played cards. These cards surface no play count.
pub fn to_slint_album_strip_row(a: &AlbumStats) -> UiEntityStripRow {
    UiEntityStripRow {
        id: clamp_i64_to_i32(a.id),
        title: SharedString::from(a.name.as_str()),
        subtitle: SharedString::from(a.artist_name.as_str()),
        artwork_path: opt_shared(a.artwork_path.as_deref()),
        play_count: 0,
        is_favorite: false,
    }
}

/// An `ArtistStats` as its strip row, subtitled with the album count for parity
/// with the Favorite Artists grid. Localizing that label is the caller's — the
/// fetch runs on a worker with no locale context, so the apply path translates.
pub fn to_slint_artist_strip_row(a: &ArtistStats, subtitle: &str) -> UiEntityStripRow {
    UiEntityStripRow {
        id: clamp_i64_to_i32(a.id),
        title: SharedString::from(a.name.as_str()),
        subtitle: SharedString::from(subtitle),
        artwork_path: opt_shared(a.image_path.as_deref()),
        play_count: 0,
        is_favorite: false,
    }
}

// `const _` is type-checked but never dead-code-flagged, so no `#[allow]` is owed.
const _: fn() = || {
    fn check<T: Send + Sync>() {}
    check::<SearchUi>();
};

#[cfg(test)]
#[path = "tests/search_ui_tests.rs"]
mod tests;
