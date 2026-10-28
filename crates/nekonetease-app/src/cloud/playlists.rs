//! Liked/recent snapshots and every playlist write the account surface exposes.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use nekonetease_core::entities::cloud::{LikedSong, Playlist, RecentSong, Song};
use nekonetease_core::error::{AppError, AppResult};
use nekonetease_net::services::net::nos_upload::{self, MAX_COVER_BYTES};

use super::CloudService;
use super::cache::{CachedPlaylist, PLAYLIST_CACHE_BYTES, playlist_bytes, playlist_cache_is_fresh};
use super::validation::{
    validate_id, validate_ids, validate_playlist_description, validate_playlist_name,
    validate_playlist_tags, validate_track_batch,
};

impl CloudService {
    /// Load a signed-in user's saved playlist and its tracks under one account generation.
    ///
    /// A playlist is the one cloud response big enough that a re-open is worth avoiding: its
    /// track list costs a round trip plus one per 300 songs, so flipping back to a list you just
    /// looked at pays that again. The cache is keyed by account *and* generation, and cleared on
    /// every transition, so a stale entry can't outlive the session that fetched it.
    pub async fn saved_playlist(&self, id: i64) -> AppResult<(Playlist, Arc<[Song]>)> {
        validate_id("歌单", id)?;
        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        if let Some(cached) = self.cached_playlist(account.id, generation, id) {
            return Ok(cached);
        }
        let (playlist, songs) =
            self.account_request_in_generation(self.client.playlist(id), cancellation).await?;
        let loaded = (playlist, songs.into());
        self.store_playlist(account.id, generation, id, &loaded);
        Ok(loaded)
    }

    pub(super) fn cached_playlist(
        &self,
        account_id: i64,
        generation: u64,
        playlist_id: i64,
    ) -> Option<(Playlist, Arc<[Song]>)> {
        let key = (account_id, generation, playlist_id);
        let mut cache = self.playlist_cache.lock();
        match cache.peek(&key).map(|entry| entry.fetched_at) {
            Some(at) if playlist_cache_is_fresh(at, Instant::now()) => {
                cache.get(&key).map(|entry| (entry.playlist.clone(), Arc::clone(&entry.songs)))
            }
            Some(_) => {
                cache.pop(&key);
                None
            }
            None => None,
        }
    }

    /// Stored under the generation the *fetch* ran in, never the one current at this instant: a
    /// sign-in that lands between the two would otherwise file an old session's track list under
    /// the new one, and the next read would serve it.
    pub(super) fn store_playlist(
        &self,
        account_id: i64,
        generation: u64,
        playlist_id: i64,
        loaded: &(Playlist, Arc<[Song]>),
    ) {
        let key = (account_id, generation, playlist_id);
        let bytes = playlist_bytes(&loaded.0, &loaded.1);
        let mut cache = self.playlist_cache.lock();
        cache.pop(&key);
        if bytes > PLAYLIST_CACHE_BYTES {
            return;
        }
        let mut retained: usize = cache.iter().map(|(_, entry)| entry.bytes).sum();
        while retained + bytes > PLAYLIST_CACHE_BYTES {
            let Some((_, evicted)) = cache.pop_lru() else { break };
            retained -= evicted.bytes;
        }
        cache.put(
            key,
            CachedPlaylist {
                fetched_at: Instant::now(),
                playlist: loaded.0.clone(),
                songs: Arc::clone(&loaded.1),
                bytes,
            },
        );
    }

    pub async fn liked_ids(&self) -> AppResult<Vec<i64>> {
        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        let revision = self.liked_ids_revision.load(Ordering::Acquire);
        let ids = self
            .account_request_in_generation(self.client.liked_ids(account.id), cancellation)
            .await?;
        if self.status().generation == generation
            && self.liked_ids_revision.load(Ordering::Acquire) == revision
        {
            *self.liked_ids_cache.write() =
                Some((account.id, generation, ids.iter().copied().collect()));
        }
        Ok(ids)
    }

    /// Load the liked playlist's server order without allowing an account transition during fetch.
    pub async fn liked_songs(&self) -> AppResult<Vec<LikedSong>> {
        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        let revision = self.liked_ids_revision.load(Ordering::Acquire);
        let songs = self
            .account_request_in_generation(self.client.liked_songs(account.id), cancellation)
            .await?;
        if self.status().generation == generation
            && self.liked_ids_revision.load(Ordering::Acquire) == revision
        {
            *self.liked_ids_cache.write() =
                Some((account.id, generation, songs.iter().map(|entry| entry.song.id).collect()));
        }
        Ok(songs)
    }

    /// Fetch the signed-in account's `NetEase` play history, preserving the order returned by the
    /// server. The snapshot is retained per account so the page can paint it immediately while a
    /// fresh request runs each time the user opens the history view.
    pub async fn recent_songs(&self) -> AppResult<Vec<RecentSong>> {
        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        let songs =
            self.account_request_in_generation(self.client.recent_songs(), cancellation).await?;
        if self.status().generation == generation {
            *self.recent_songs_cache.write() = Some((account.id, generation, songs.clone()));
        }
        Ok(songs)
    }

    /// Report a completed listening session only to the account that actually heard it.
    /// Reporting failures stay in the background and never change the login or player state.
    pub async fn report_listen(
        &self,
        song_id: i64,
        seconds: u64,
        generation: u64,
    ) -> AppResult<()> {
        let (_account, cancellation) = self.account_snapshot()?;
        if self.status().generation != generation {
            return Ok(());
        }
        validate_id("歌曲", song_id)?;
        self.account_request_in_generation(
            self.client.report_listen(song_id, seconds),
            cancellation,
        )
        .await?;
        if self.status().generation == generation {
            *self.recent_songs_cache.write() = None;
        }
        Ok(())
    }

    pub async fn like(&self, song_id: i64, liked: bool) -> AppResult<()> {
        validate_id("歌曲", song_id)?;
        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        self.account_request_in_generation(self.client.like(song_id, liked), cancellation).await?;
        if self.status().generation == generation {
            self.liked_ids_revision.fetch_add(1, Ordering::AcqRel);
            if let Some((cached_account, cached_generation, ids)) =
                self.liked_ids_cache.write().as_mut()
                && *cached_account == account.id
                && *cached_generation == generation
            {
                if liked {
                    ids.insert(song_id);
                } else {
                    ids.remove(&song_id);
                }
            }
        }
        Ok(())
    }

    pub async fn create_playlist(&self, name: &str) -> AppResult<i64> {
        let name = validate_playlist_name(name)?;
        self.private_request(self.client.create_playlist(name)).await
    }

    pub async fn delete_playlists(&self, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Err(AppError::Validation("请选择要删除的歌单".to_owned()));
        }
        validate_ids("歌单", ids)?;
        self.private_request(self.client.remove_playlists(ids)).await
    }

    pub async fn rename_playlist(&self, id: i64, name: &str) -> AppResult<()> {
        validate_id("歌单", id)?;
        let name = validate_playlist_name(name)?;
        self.private_request(self.client.update_playlist_name(id, name)).await
    }

    pub async fn update_playlist_description(&self, id: i64, desc: &str) -> AppResult<()> {
        validate_id("歌单", id)?;
        validate_playlist_description(desc)?;
        self.private_request(self.client.update_playlist_desc(id, desc)).await
    }

    pub async fn update_playlist_tags(&self, id: i64, tags: &[String]) -> AppResult<()> {
        validate_id("歌单", id)?;
        let tags = validate_playlist_tags(tags)?;
        self.private_request(self.client.update_playlist_tags(id, &tags)).await
    }

    pub async fn add_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> AppResult<()> {
        validate_id("歌单", id)?;
        validate_track_batch(track_ids)?;
        self.private_request(self.client.add_playlist_tracks(id, track_ids)).await
    }

    pub async fn remove_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> AppResult<()> {
        validate_id("歌单", id)?;
        validate_track_batch(track_ids)?;
        self.private_request(self.client.remove_playlist_tracks(id, track_ids)).await
    }

    pub async fn reorder_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> AppResult<()> {
        validate_id("歌单", id)?;
        validate_track_batch(track_ids)?;
        self.private_request(self.client.reorder_playlist_tracks(id, track_ids)).await
    }

    pub async fn unsubscribe_playlist(&self, id: i64) -> AppResult<()> {
        validate_id("歌单", id)?;
        self.private_request(self.client.unsubscribe_playlist(id)).await
    }

    /// Upload a cover for one of the account's own playlists.
    ///
    /// The image is checked and sent before the playlist is pointed at it, so a failed transfer
    /// leaves the old cover in place. The cache entry is dropped rather than patched: the detail
    /// re-fetches the playlist's new `cover_url`, and the grid turns that into the stored
    /// thumbnail the header paints.
    pub async fn upload_playlist_cover(&self, id: i64, path: &Path) -> AppResult<()> {
        validate_id("歌单", id)?;
        let format = cover_format(path)?;
        let size = tokio::fs::metadata(path)
            .await
            .map_err(|e| AppError::io(format!("封面文件无法读取: {}", path.display()), e))?
            .len();
        validate_cover_size(size)?;

        let (account, cancellation) = self.account_snapshot()?;
        let generation = self.status().generation;
        let token = self
            .account_request_in_generation(
                self.client
                    .alloc_cover_token(&cover_filename(path, format.extension), format.extension),
                cancellation.clone(),
            )
            .await?;
        tokio::select! {
            () = cancellation.cancelled() => {
                return Err(AppError::network_msg("网易云请求已取消"));
            }
            result = nos_upload::upload_cover_image(&token, path, format.content_type) => {
                result?;
            }
        }
        self.account_request_in_generation(
            self.client.set_playlist_cover(id, token.doc_id()),
            cancellation,
        )
        .await?;
        if self.status().generation == generation {
            self.playlist_cache.lock().pop(&(account.id, generation, id));
        }
        Ok(())
    }
}

struct CoverFormat {
    extension: &'static str,
    content_type: &'static str,
}

/// The upload's extension and MIME type both come from the pick's own suffix. The service accepts
/// a JPEG, PNG or WebP here; anything else is refused before a token is allocated for it.
fn cover_format(path: &Path) -> AppResult<CoverFormat> {
    let Some(extension) = path.extension().and_then(OsStr::to_str) else {
        return Err(AppError::Validation("封面仅支持 JPG、PNG 或 WebP 图片".to_owned()));
    };
    match extension.to_ascii_lowercase().as_str() {
        "jpg" => Ok(CoverFormat { extension: "jpg", content_type: "image/jpeg" }),
        "jpeg" => Ok(CoverFormat { extension: "jpeg", content_type: "image/jpeg" }),
        "png" => Ok(CoverFormat { extension: "png", content_type: "image/png" }),
        "webp" => Ok(CoverFormat { extension: "webp", content_type: "image/webp" }),
        _ => Err(AppError::Validation("封面仅支持 JPG、PNG 或 WebP 图片".to_owned())),
    }
}

fn validate_cover_size(size: u64) -> AppResult<()> {
    if size == 0 {
        return Err(AppError::Validation("封面图片为空".to_owned()));
    }
    if size > MAX_COVER_BYTES {
        return Err(AppError::Validation(format!(
            "封面图片不能超过 {} MB",
            MAX_COVER_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

fn cover_filename(path: &Path, extension: &str) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty())
        .map_or_else(|| format!("cover.{extension}"), str::to_owned)
}

#[cfg(test)]
#[path = "../tests/playlist_cover_tests.rs"]
mod tests;
