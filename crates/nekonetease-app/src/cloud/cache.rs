//! Per-account in-process caches, keyed by account and generation so no entry outlives its session.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use lru::LruCache;
use nekonetease_core::entities::cloud::{Artist, Playlist, RecentSong, Song};

use super::CloudService;

/// How long a fetched playlist may be reopened without another round trip. Long enough to make
/// flipping between two playlists free, short enough that a list edited on another device shows up
/// on its own.
pub(super) const PLAYLIST_CACHE_TTL: Duration = Duration::from_mins(5);
/// An entry is a whole track list — roughly a megabyte for a thousand songs — so a few, not a stack.
pub(super) const PLAYLIST_CACHE_CAP: NonZeroUsize = match NonZeroUsize::new(4) {
    Some(cap) => cap,
    None => panic!("PLAYLIST_CACHE_CAP > 0"),
};
// Entry count alone cannot bound a cache whose playlists can contain tens of thousands of songs.
pub(super) const PLAYLIST_CACHE_BYTES: usize = 8 * 1024 * 1024;

pub(super) type LikedIdsCache = Arc<parking_lot::RwLock<Option<(i64, u64, HashSet<i64>)>>>;
pub(super) type RecentSongsCache = Arc<parking_lot::RwLock<Option<(i64, u64, Vec<RecentSong>)>>>;
/// `(account, generation, playlist id)` → when it was fetched, and the response itself.
pub(super) type PlaylistCache = Arc<parking_lot::Mutex<LruCache<(i64, u64, i64), CachedPlaylist>>>;

pub(super) struct CachedPlaylist {
    pub(super) fetched_at: Instant,
    pub(super) playlist: Playlist,
    pub(super) songs: Arc<[Song]>,
    pub(super) bytes: usize,
}

pub(super) fn playlist_bytes(playlist: &Playlist, songs: &[Song]) -> usize {
    let optional = |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
    let artists = |artists: &Vec<Artist>| {
        artists.capacity() * std::mem::size_of::<Artist>()
            + artists
                .iter()
                .map(|artist| {
                    artist.name.capacity()
                        + optional(&artist.cover_url)
                        + optional(&artist.alternate_cover_url)
                })
                .sum::<usize>()
    };
    std::mem::size_of_val(songs)
        + std::mem::size_of::<CachedPlaylist>()
        + playlist.name.capacity()
        + optional(&playlist.cover_url)
        + optional(&playlist.description)
        + songs
            .iter()
            .map(|song| {
                song.name.capacity()
                    + song.album.name.capacity()
                    + optional(&song.album.cover_url)
                    + artists(&song.artists)
                    + artists(&song.album.artists)
            })
            .sum::<usize>()
}

/// Whether a cached playlist is still worth serving. Pure, so both sides of the TTL boundary are
/// table-testable without a clock to steer.
pub(super) fn playlist_cache_is_fresh(fetched_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(fetched_at) < PLAYLIST_CACHE_TTL
}

impl CloudService {
    /// A recent-history snapshot for the visible account. It is only an in-process presentation
    /// cache; opening the page still refreshes it from `NetEase`.
    pub fn cached_recent_songs(&self) -> Option<Vec<RecentSong>> {
        let status = self.status();
        let account_id = status.account?.id;
        let cache = self.recent_songs_cache.read();
        let (cached_account, generation, songs) = cache.as_ref()?;
        (*cached_account == account_id && *generation == status.generation).then(|| songs.clone())
    }

    /// Cached membership for player controls. `None` means this account's liked set has not yet
    /// been fetched in this process; callers can still write the toggle and refresh it afterward.
    pub fn cached_is_liked(&self, song_id: i64) -> Option<bool> {
        let status = self.status();
        let account_id = status.account?.id;
        let cache = self.liked_ids_cache.read();
        let (cached_account, generation, ids) = cache.as_ref()?;
        (*cached_account == account_id && *generation == status.generation)
            .then(|| ids.contains(&song_id))
    }

    /// The whole liked set this process already holds for the visible account, or `None` when it
    /// was never fetched (or belongs to a previous account or generation). A view flagging every
    /// row of a list is the caller that needs the set rather than one membership answer, and a
    /// list open is not worth a second whole-list request when the snapshot is already there.
    pub fn cached_liked_ids(&self) -> Option<HashSet<i64>> {
        let status = self.status();
        let account_id = status.account?.id;
        let cache = self.liked_ids_cache.read();
        let (cached_account, generation, ids) = cache.as_ref()?;
        (*cached_account == account_id && *generation == status.generation).then(|| ids.clone())
    }

    /// Version of the liked-song snapshot. Views use it to discard a read that raced a successful
    /// like/unlike write, so an older response cannot paint stale membership after the button.
    pub fn liked_ids_revision(&self) -> u64 {
        self.liked_ids_revision.load(Ordering::Acquire)
    }
}
