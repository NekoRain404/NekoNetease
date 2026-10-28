use std::collections::BTreeMap;

use futures_util::{StreamExt as _, TryStreamExt as _};
use serde_json::{Value, json};

use nekonetease_core::entities::cloud::{Album, Artist, Playlist, Song};

use crate::netease::endpoints::{self as ep};
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, field};

/// Song ids per `song/detail` request. The route takes a much longer list, but a batch this size
/// keeps one response small enough to parse without a noticeable pause on the UI-facing hop.
const SONG_BATCH: usize = 300;
/// How many `song/detail` batches may be in flight at once. The route publishes no rate limit and
/// a playlist open is the only caller that asks for more than one or two.
const SONG_BATCH_CONCURRENCY: usize = 3;

impl Client {
    pub async fn songs(&self, ids: &[i64]) -> Result<Vec<Song>> {
        // Owned batches rather than borrowed ones: an async block over a `&[i64]` makes this
        // future's type depend on one slice lifetime, which callers spawning it can't satisfy.
        let batches = ids.chunks(SONG_BATCH).map(<[i64]>::to_vec).collect::<Vec<_>>();
        let mut fetched = futures_util::stream::iter(batches)
            .map(|batch| self.song_batch(batch))
            // A playlist open is the one caller that pays for more than a batch or two, and the
            // batches are independent: sequentially, a 600-track playlist waits out two round
            // trips for no ordering benefit. `buffered` keeps them in playlist order however they
            // interleave.
            .buffered(SONG_BATCH_CONCURRENCY);
        let mut songs = Vec::with_capacity(ids.len());
        while let Some(batch) = fetched.try_next().await? {
            songs.extend(batch);
        }
        Ok(songs)
    }

    async fn song_batch(&self, ids: Vec<i64>) -> Result<Vec<Song>> {
        let c = ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>();
        let body = self
            .request(
                ep::SONGS,
                json!({"c":serde_json::to_string(&c).map_err(|_|Error::InvalidResponse)?}),
            )
            .await?;
        let mut songs: Vec<Song> = decode_ref(&body["songs"])?;
        let privileges: Vec<nekonetease_core::entities::cloud::Privilege> =
            field(body, "privileges").unwrap_or_default();
        for song in &mut songs {
            song.privilege = privileges.iter().find(|p| p.id == song.id).cloned();
        }
        // Detail responses may reorder IDs; restore the caller's order and preserve repeated
        // entries (playlists can contain the same song more than once).
        Ok(reorder_songs(&ids, songs))
    }

    pub async fn playlist(&self, id: i64) -> Result<(Playlist, Vec<Song>)> {
        // `n` is how many tracks this response inlines. Nothing here reads them — the rows come
        // from `songs` below — and a non-zero `n` makes the header carry a second copy of the
        // whole playlist, its payload growing with the track count.
        let body = self.request(ep::PLAYLIST, json!({"id":id,"n":0,"s":0})).await?;
        let playlist = decode_playlist_header(&body, id)?;
        let ids = body["playlist"]["trackIds"]
            .as_array()
            .ok_or(Error::InvalidResponse)?
            .iter()
            .filter_map(|v| v["id"].as_i64())
            .collect::<Vec<_>>();
        // Detail can include a full track array as well as IDs. Release that response before
        // fetching the batches, rather than keeping both representations for the entire load.
        drop(body);
        let songs = self.songs(&ids).await?;
        Ok((playlist, songs))
    }

    pub async fn album(&self, id: i64) -> Result<(Album, Vec<Song>)> {
        let path = format!("/api/v1/album/{id}");
        let body = self.request_path(&path, json!({})).await?;
        let album: Album = decode_ref(&body["album"])?;
        let mut songs: Vec<Song> = field(body, "songs")?;
        fill_album_covers(&album, &mut songs);
        Ok((album, songs))
    }

    pub async fn artist(&self, id: i64) -> Result<(Artist, Vec<Song>)> {
        let path = format!("/api/v1/artist/{id}");
        let body = self.request_path(&path, json!({})).await?;
        Ok((decode_ref(&body["artist"])?, field(body, "hotSongs")?))
    }
}

pub(super) fn fill_album_covers(album: &Album, songs: &mut [Song]) {
    for song in songs {
        if song.album.id == album.id && song.album.cover_url.as_deref().is_none_or(str::is_empty) {
            song.album.cover_url.clone_from(&album.cover_url);
        }
    }
}

pub(super) fn decode_playlist_header(body: &Value, requested_id: i64) -> Result<Playlist> {
    let playlist: Playlist = decode_ref(body.get("playlist").ok_or(Error::InvalidResponse)?)?;
    if playlist.id != requested_id {
        return Err(Error::InvalidResponse);
    }
    Ok(playlist)
}

pub(super) fn reorder_songs(ids: &[i64], songs: Vec<Song>) -> Vec<Song> {
    let map = songs.into_iter().map(|song| (song.id, song)).collect::<BTreeMap<_, _>>();
    ids.iter().filter_map(|id| map.get(id).cloned()).collect()
}
