//! The columns each `tracks` projection selects, one list per narrow `FromRow` struct in
//! `nekonetease_core::entities::track`. Listed in the struct's field order for legibility; `FromRow`
//! matches by name, so the order carries nothing.

use std::sync::{Mutex, OnceLock, PoisonError};

/// A projection's column names, joined once for a bare `SELECT` and once per table alias.
pub(crate) struct Columns {
    names: &'static [&'static str],
    joined: OnceLock<String>,
    /// `(alias, "alias.a, alias.b, …")`. The aliases are literals and few, so leaking each
    /// prefixed list once is bounded.
    prefixed: Mutex<Vec<(&'static str, &'static str)>>,
}

impl Columns {
    const fn new(names: &'static [&'static str]) -> Self {
        Self { names, joined: OnceLock::new(), prefixed: Mutex::new(Vec::new()) }
    }

    pub(crate) fn joined(&self) -> &str {
        self.joined.get_or_init(|| self.names.join(", "))
    }

    /// Project these columns while resolving a missing track cover from its album.
    ///
    /// Track files can carry their own artwork, but many releases store the cover only once on
    /// the album row. Track lists need a usable path for both shapes, with track artwork taking
    /// precedence when it exists. `alias` is the track table's alias where the surrounding query
    /// has one; the album lookup stays a primary-key subquery so callers need no extra join.
    pub(crate) fn with_album_artwork_fallback(&self, alias: Option<&str>) -> String {
        let qualify = |column: &str| match alias {
            Some(alias) => format!("{alias}.{column}"),
            None => column.to_owned(),
        };

        self.names
            .iter()
            .map(|column| {
                if *column == "artwork_path" {
                    format!(
                        "COALESCE(NULLIF({}, ''), (SELECT NULLIF(albums.artwork_path, '') \
                         FROM albums WHERE albums.id = {})) AS artwork_path",
                        qualify("artwork_path"),
                        qualify("album_id")
                    )
                } else {
                    qualify(column)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The list with every column qualified by `alias`, for a query that joins `tracks`.
    pub(crate) fn prefixed(&self, alias: &'static str) -> &'static str {
        let mut cache = self.prefixed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&(_, hit)) = cache.iter().find(|(seen, _)| *seen == alias) {
            return hit;
        }
        let qualified: &'static str = Box::leak(
            self.names
                .iter()
                .map(|column| format!("{alias}.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
                .into_boxed_str(),
        );
        cache.push((alias, qualified));
        qualified
    }
}

/// `TrackSummary`: what the queue, the Now Playing bar and the playback path read.
pub(crate) static TRACK_SUMMARY: Columns = Columns::new(&[
    "id",
    "file_path",
    "file_name",
    "title",
    "artist",
    "album",
    "duration_ms",
    "artwork_path",
    "track_number",
    "disc_number",
    "last_position",
    "is_favorite",
    "rating",
    "replaygain_track_gain",
    "replaygain_track_peak",
    "replaygain_album_gain",
    "replaygain_album_peak",
]);

/// `TrackListRow`: every track list.
pub(crate) static TRACK_LIST: Columns = Columns::new(&[
    "id",
    "file_path",
    "file_name",
    "title",
    "artist",
    "album_artist",
    "album",
    "genre",
    "track_number",
    "disc_number",
    "year",
    "duration_ms",
    "artwork_path",
    "is_favorite",
    "rating",
    "album_id",
    "artist_id",
    "genre_id",
    "date_added",
    "sort_key",
]);

/// `PlaylistExportRow`: one Extended-M3U8 line.
pub(crate) static PLAYLIST_EXPORT: Columns =
    Columns::new(&["file_path", "file_hash", "title", "artist", "duration_ms"]);

/// `TrackMeta`: the Now Playing view's technical chips.
pub(crate) static TRACK_META: Columns = Columns::new(&[
    "id",
    "codec",
    "bitrate",
    "sample_rate",
    "bit_depth",
    "channels",
    "year",
    "genre",
]);

/// `TagEditRow`: the Edit Track Information dialog.
pub(crate) static TRACK_TAG_EDIT: Columns = Columns::new(&[
    "id",
    "file_path",
    "title",
    "artist",
    "album_artist",
    "album",
    "year",
    "original_year",
    "track_number",
    "track_total",
    "disc_number",
    "disc_total",
    "disc_subtitle",
    "subtitle",
    "comment",
    "bpm",
    "initial_key",
    "mood",
    "grouping",
    "work",
    "movement",
    "movement_number",
    "movement_total",
    "language",
    "copyright",
    "isrc",
    "artwork_path",
    "codec",
    "bitrate",
    "sample_rate",
    "bit_depth",
    "channels",
    "file_size",
    "date_modified",
    "duration_ms",
    "file_hash",
]);

/// `TrackLinks`: the three ids a "Go to …" entry navigates by.
pub(crate) static TRACK_LINKS: Columns = Columns::new(&["id", "album_id", "artist_id", "genre_id"]);

/// `MostPlayedFavorite`: the Most Played cards.
pub(crate) static MOST_PLAYED: Columns = Columns::new(&[
    "id",
    "title",
    "artist",
    "album_artist",
    "album",
    "genre",
    "year",
    "artwork_path",
    "play_count",
    "duration_ms",
    "is_favorite",
]);
