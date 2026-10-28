//! Internal data structures for the Search view. Mirrors `src/ui/favorites/state.rs`
//! in shape, concentrated here so the public [`crate::ui::search::SearchUi`] surface
//! stays small.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;

use parking_lot::Mutex;

use nekonetease_app::services::settings::{SortDir, ViewSort};
use nekonetease_core::entities::cloud::{SearchKind, SearchResult};
use nekonetease_core::entities::netease_discovery::HotSearchItem;
use nekonetease_ui::{SearchEntityRow, TrackListRow as UiTrackListRow};

/// Per-view caches for the current `NetEase` search page and local search history.
pub(crate) struct SearchUiState {
    /// The query whose request most recently committed. Mirrors `Search.query` at commit time,
    /// not at every keystroke, so request handlers never observe a half-typed value.
    pub last_query: Mutex<String>,
    /// Raw cloud response for the current page, used to play every song in that page even while
    /// the compact Songs presentation only paints its first few rows.
    pub cloud_results: Mutex<Option<SearchResult>>,
    /// Search-view rows projected directly from the cloud response. Remote entity identity stays
    /// string-valued in Slint and these rows never pass through the local SQLite projection.
    pub cloud_song_rows: Mutex<Vec<UiTrackListRow>>,
    pub cloud_entity_rows: Mutex<Vec<SearchEntityRow>>,
    /// Request cursor for the current cloud search. The service owns the page size; views only
    /// ask for a direction and cannot construct an out-of-range offset.
    pub cloud_search_kind: Mutex<SearchKind>,
    pub cloud_search_offset: Mutex<u64>,
    /// Favorite toggles made from the visible page, kept until the next fetch so a cached sort or
    /// compact/full re-apply cannot repaint the stale server snapshot.
    pub cloud_favorite_overrides: Mutex<HashMap<i64, bool>>,
    /// Mirror of the persisted history file, pushed into `Search.recent-rows` after
    /// every add / remove / clear — the sort handler runs off the UI thread and
    /// shouldn't reach into the Slint global.
    pub recent: Mutex<Vec<String>>,
    /// The trending queries the empty-query state paints as chips. Kept as the boundary type so
    /// the score and icon stay available to a later surface; the global takes only the words.
    pub hot_searches: Mutex<Vec<HotSearchItem>>,
    /// Recommended playlists, already projected to the shared entity row the discovery strip
    /// draws. Remote IDs stay decimal text, as every cloud row does.
    pub discovery_rows: Mutex<Vec<SearchEntityRow>>,
    /// The chart catalogue, projected to the strip's row shape with a heading row (an empty
    /// remote id) where a server tag starts a category. A separate cell so a failed charts
    /// request cannot blank the recommendations beside it.
    pub chart_rows: Mutex<Vec<SearchEntityRow>>,
    /// Whether this section-visit's one discovery fetch has been claimed. Released with the rest
    /// of the section state, so a re-entry re-asks rather than painting an old directory forever.
    pub discovery_loaded: Mutex<bool>,
    /// Active sort over the Songs section. `NetEase` returns relevance order by default; users can
    /// choose a standard track-list field after the fact, persisted to `views.json`.
    pub sort: Mutex<ViewSort>,
    /// `TrackListRow.id`s currently `selected: true` on the Slint model. The same
    /// diff-then-write Favorites uses.
    pub applied_selection: Mutex<HashSet<i32>>,
}

impl SearchUiState {
    pub(super) fn new() -> Self {
        Self {
            last_query: Mutex::new(String::new()),
            cloud_results: Mutex::new(None),
            cloud_song_rows: Mutex::new(Vec::new()),
            cloud_entity_rows: Mutex::new(Vec::new()),
            cloud_search_kind: Mutex::new(SearchKind::Songs),
            cloud_search_offset: Mutex::new(0),
            cloud_favorite_overrides: Mutex::new(HashMap::new()),
            recent: Mutex::new(Vec::new()),
            hot_searches: Mutex::new(Vec::new()),
            discovery_rows: Mutex::new(Vec::new()),
            chart_rows: Mutex::new(Vec::new()),
            discovery_loaded: Mutex::new(false),
            // `"rank"` is synthetic: NetEase's response already orders by relevance, so the
            // first apply is a no-op. Other fields sort the current page in memory.
            sort: Mutex::new(ViewSort { field: "rank".to_owned(), dir: SortDir::Asc }),
            applied_selection: Mutex::new(HashSet::new()),
        }
    }
}

/// Albums-strip tile size (px). The strip renders 160 px square cards and `FemtoVG`
/// minifies with plain bilinear, so staying near the on-screen size keeps
/// `image-fit: cover` clean. Sized against *these* cards rather than another view's
/// tier: the grids draw flex-filled cards that grow with the window, so a strip
/// following them would decode tiles several times larger than any card it can draw.
pub(super) const ALBUM_STRIP_THUMB_SIZE: u32 = 180;

/// Artists-strip tile size (px). Larger than the albums tier because the circular
/// avatar reads softer when downscaled less aggressively.
pub(super) const ARTIST_STRIP_THUMB_SIZE: u32 = 200;

/// LRU capacity per strip. `search_all` clamps album and artist results to 20 each, so
/// 24 covers a fresh swap without the next set's decode evicting the first row before
/// it is painted.
pub(super) const STRIP_THUMB_CAP: NonZeroUsize = match NonZeroUsize::new(24) {
    Some(n) => n,
    None => panic!("STRIP_THUMB_CAP > 0"),
};

/// Songs rows shown when `show-all-tracks` is off. The toggle expands to whatever
/// `search_all` returned — its own `LIMIT` is the only cap, so don't restate that
/// number here.
pub(super) const COMPACT_TRACK_LIMIT: usize = 5;
