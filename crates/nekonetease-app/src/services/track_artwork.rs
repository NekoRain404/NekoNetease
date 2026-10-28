//! Session-only lookup of remote album-art URLs for tracks without readable local artwork.
//!
//! The URL is resolved from the tagged artist + album and kept in a bounded in-memory LRU. Image
//! bytes remain with the caller's in-memory image tier; this service never writes artwork to disk.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, watch};

use nekonetease_core::error::describe;
use nekonetease_net::media::fetch::deezer;

const CACHE_CAP: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(cap) => cap,
    None => panic!("track artwork URL cache capacity must be non-zero"),
};
const FOUND_TTL: Duration = Duration::from_hours(24);
const NOT_FOUND_TTL: Duration = Duration::from_mins(10);
pub const TRANSIENT_RETRY_DELAY: Duration = Duration::from_secs(30);
const FAILED_TTL: Duration = TRANSIENT_RETRY_DELAY;
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(175);

#[derive(Clone)]
enum LookupUpdate {
    Pending,
    Settled,
}

enum LookupState {
    Idle,
    Pending,
    Ready { url: Option<String>, expires_at: Instant },
}

struct LookupEntry {
    state: Mutex<LookupState>,
    updates: watch::Sender<LookupUpdate>,
}

impl LookupEntry {
    fn new() -> Self {
        let (updates, _) = watch::channel(LookupUpdate::Pending);
        Self { state: Mutex::new(LookupState::Idle), updates }
    }
}

enum Claim {
    Cached(Option<String>),
    Start,
    Wait,
}

/// What a synchronous cover binding knows about a URL lookup.
pub enum CachedCoverUrl {
    NotCached,
    Pending,
    NotFound,
    Found(String),
}

/// Resolves remote album cover URLs without persisting either the lookup or image data.
pub struct TrackArtworkResolver {
    entries: Mutex<LruCache<String, Arc<LookupEntry>>>,
    /// Deezer shares a per-IP quota across its search endpoints. Stagger visible-row misses so a
    /// library with many untagged covers does not issue a burst of simultaneous searches.
    request_gate: AsyncMutex<()>,
    last_request: Mutex<Option<Instant>>,
}

impl Default for TrackArtworkResolver {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(CACHE_CAP)),
            request_gate: AsyncMutex::new(()),
            last_request: Mutex::new(None),
        }
    }
}

impl TrackArtworkResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a previously resolved URL, a settled no-match, or the current lookup state.
    pub fn cached_url(&self, artist: &str, album: &str) -> CachedCoverUrl {
        let Some(key) = lookup_key(artist, album) else { return CachedCoverUrl::NotFound };
        let Some(entry) = self.entries.lock().get(&key).cloned() else {
            return CachedCoverUrl::NotCached;
        };
        let mut state = entry.state.lock();
        match &*state {
            LookupState::Ready { url, expires_at } if *expires_at > Instant::now() => url
                .as_ref()
                .map_or(CachedCoverUrl::NotFound, |url| CachedCoverUrl::Found(url.clone())),
            LookupState::Ready { .. } => {
                *state = LookupState::Idle;
                entry.updates.send_replace(LookupUpdate::Pending);
                CachedCoverUrl::NotCached
            }
            LookupState::Idle => CachedCoverUrl::NotCached,
            LookupState::Pending => CachedCoverUrl::Pending,
        }
    }

    /// Claim one background search. A caller that gets `true` must call
    /// [`Self::lookup_claimed`] and invalidate the UI bindings when it finishes.
    pub fn claim_lookup(&self, artist: &str, album: &str) -> bool {
        let Some(key) = lookup_key(artist, album) else { return false };
        let entry = self.entry(&key);
        matches!(Self::claim(&entry), Claim::Start)
    }

    /// Resolve a URL for the active playback source, joining any list-row lookup already in flight.
    pub async fn resolve(
        &self,
        client: &reqwest::Client,
        artist: &str,
        album: &str,
    ) -> Option<String> {
        let key = lookup_key(artist, album)?;
        let entry = self.entry(&key);
        let artist = artist.trim().to_owned();
        let album = album.trim().to_owned();

        loop {
            // Subscribe before claiming so a just-finished request cannot be missed between the
            // state check and waiting for its watch update.
            let mut updates = entry.updates.subscribe();
            match Self::claim(&entry) {
                Claim::Cached(url) => return url,
                Claim::Start => {
                    return self.lookup_entry(client, &artist, &album, &entry).await;
                }
                Claim::Wait => {
                    if matches!(&*updates.borrow(), LookupUpdate::Settled) {
                        continue;
                    }
                    if updates.changed().await.is_err() {
                        return None;
                    }
                }
            }
        }
    }

    /// Complete a lookup previously claimed by a synchronous UI callback.
    pub async fn lookup_claimed(
        &self,
        client: &reqwest::Client,
        artist: &str,
        album: &str,
    ) -> Option<String> {
        let key = lookup_key(artist, album)?;
        let entry = self.entry(&key);
        if let Claim::Cached(url) = Self::claim(&entry) {
            return url;
        }
        self.lookup_entry(client, artist.trim(), album.trim(), &entry).await
    }

    fn entry(&self, key: &str) -> Arc<LookupEntry> {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get(key) {
            return Arc::clone(entry);
        }
        let entry = Arc::new(LookupEntry::new());
        entries.put(key.to_owned(), Arc::clone(&entry));
        entry
    }

    fn claim(entry: &LookupEntry) -> Claim {
        let mut state = entry.state.lock();
        match &*state {
            LookupState::Ready { url, expires_at } if *expires_at > Instant::now() => {
                Claim::Cached(url.clone())
            }
            LookupState::Pending => Claim::Wait,
            LookupState::Idle | LookupState::Ready { .. } => {
                *state = LookupState::Pending;
                entry.updates.send_replace(LookupUpdate::Pending);
                Claim::Start
            }
        }
    }

    async fn lookup_entry(
        &self,
        client: &reqwest::Client,
        artist: &str,
        album: &str,
        entry: &LookupEntry,
    ) -> Option<String> {
        self.pace_request().await;
        let (url, ttl) = match deezer::search_album_cover(client, artist, album).await {
            Ok(Some(url)) if is_trusted_cover_url(&url) => (Some(url), FOUND_TTL),
            Ok(Some(url)) => {
                log::warn!("Deezer returned an untrusted track cover URL: {url}");
                (None, NOT_FOUND_TTL)
            }
            Ok(None) => (None, NOT_FOUND_TTL),
            Err(error) => {
                log::debug!(
                    "track album cover lookup failed for {artist} / {album}: {}",
                    describe(&error)
                );
                (None, FAILED_TTL)
            }
        };

        *entry.state.lock() =
            LookupState::Ready { url: url.clone(), expires_at: Instant::now() + ttl };
        entry.updates.send_replace(LookupUpdate::Settled);
        url
    }

    async fn pace_request(&self) {
        let _gate = self.request_gate.lock().await;
        let now = Instant::now();
        let wait = self
            .last_request
            .lock()
            .map(|last| (last + MIN_REQUEST_INTERVAL).saturating_duration_since(now))
            .unwrap_or_default();
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *self.last_request.lock() = Some(Instant::now());
    }
}

fn lookup_key(artist: &str, album: &str) -> Option<String> {
    let artist = artist.trim();
    let album = album.trim();
    if artist.is_empty() || album.is_empty() {
        return None;
    }
    Some(format!("{}\0{}", artist.to_lowercase(), album.to_lowercase()))
}

fn is_trusted_cover_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else { return false };
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".deezer.com") || host.ends_with(".dzcdn.net"))
}
