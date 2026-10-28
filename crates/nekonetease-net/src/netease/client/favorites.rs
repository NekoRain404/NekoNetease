use std::collections::BTreeMap;

use serde_json::{Value, json};

use nekonetease_core::entities::cloud::{Album, Artist, LikedSong, Playlist, RecentSong, Song};

use crate::netease::endpoints::{self as ep};
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, field};

impl Client {
    pub async fn liked_playlist_id(&self, uid: i64) -> Result<i64> {
        let body = self
            .request(
                ep::USER_PLAYLISTS,
                json!({"uid":uid,"offset":0,"limit":30,"includeVideo":false}),
            )
            .await?;
        decode_liked_playlist_id(&body)
    }

    pub async fn created_playlists_page(
        &self,
        uid: i64,
        offset: u64,
    ) -> Result<(Vec<Playlist>, bool)> {
        let body = self
            .request(
                ep::USER_PLAYLISTS,
                json!({"uid":uid,"offset":offset,"limit":30,"includeVideo":false}),
            )
            .await?;
        decode_created_playlists(&body, uid)
    }
    pub async fn fm(&self) -> Result<Vec<Song>> {
        field(self.request(ep::FM, json!({})).await?, "data")
    }
    pub async fn fm_trash(&self, id: i64) -> Result<()> {
        self.request(ep::FM_TRASH, json!({"songId":id,"alg":"RT","time":25})).await?;
        Ok(())
    }
    pub async fn user_playlists(&self, uid: i64, offset: u64) -> Result<Vec<Playlist>> {
        self.user_playlists_page(uid, offset).await.map(|(playlists, _)| playlists)
    }

    pub async fn user_playlists_page(
        &self,
        uid: i64,
        offset: u64,
    ) -> Result<(Vec<Playlist>, bool)> {
        let body = self
            .request(
                ep::USER_PLAYLISTS,
                json!({"uid":uid,"offset":offset,"limit":30,"includeVideo":false}),
            )
            .await?;
        decode_saved_playlists(&body, uid)
    }
    pub async fn liked_ids(&self, uid: i64) -> Result<Vec<i64>> {
        field(self.request(ep::LIKED, json!({"uid":uid})).await?, "ids")
    }

    /// Return the server-ordered recent-song history for the current account. The wrapper route
    /// `/record/recent/song` maps to `NetEase`'s actual WEAPI endpoint
    /// `/api/play-record/song/list`.
    pub async fn recent_songs(&self) -> Result<Vec<RecentSong>> {
        let body = self.request(ep::RECENT_SONGS, json!({"limit":100})).await?;
        decode_recent_songs(&body)
    }

    /// Fetch liked songs in the order `NetEase` returns for the account's special liked playlist.
    /// Song details are fetched separately, so explicitly rejoin them by ID without allowing the
    /// detail endpoint's response order to change the server-provided favorite sequence.
    pub async fn liked_songs(&self, uid: i64) -> Result<Vec<LikedSong>> {
        let playlists = self
            .request(
                ep::USER_PLAYLISTS,
                json!({"uid":uid,"offset":0,"limit":30,"includeVideo":false}),
            )
            .await?;
        let playlist_id = decode_liked_playlist_id(&playlists)?;
        let body = self.request(ep::PLAYLIST, json!({"id":playlist_id,"n":100_000,"s":0})).await?;
        let entries = decode_liked_playlist_entries(&body)?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen = std::collections::HashSet::new();
        let ids = entries
            .iter()
            .filter_map(|entry| seen.insert(entry.id).then_some(entry.id))
            .collect::<Vec<_>>();
        let songs = self.songs(&ids).await?;
        Ok(assemble_liked_songs(entries, songs))
    }

    pub async fn like(&self, id: i64, like: bool) -> Result<()> {
        self.request(ep::LIKE, json!({"alg":"itembased","trackId":id,"like":like,"time":"3"}))
            .await?;
        Ok(())
    }
    pub async fn saved_albums(&self, offset: u64) -> Result<Vec<Album>> {
        self.saved_albums_page(offset).await.map(|(albums, _)| albums)
    }

    pub async fn saved_albums_page(&self, offset: u64) -> Result<(Vec<Album>, bool)> {
        let body =
            self.request(ep::ALBUMS, json!({"offset":offset,"limit":30,"total":true})).await?;
        decode_saved_albums_page(&body)
    }
    pub async fn saved_artists(&self, offset: u64) -> Result<Vec<Artist>> {
        field(
            self.request(ep::ARTISTS, json!({"offset":offset,"limit":30,"total":true})).await?,
            "data",
        )
    }
}

pub(super) fn decode_saved_playlists(body: &Value, uid: i64) -> Result<(Vec<Playlist>, bool)> {
    decode_playlist_page(body, |playlist| {
        playlist["creator"]["userId"].as_i64().is_none_or(|owner| owner != uid)
    })
}

pub(super) fn decode_created_playlists(body: &Value, uid: i64) -> Result<(Vec<Playlist>, bool)> {
    decode_playlist_page(body, |playlist| playlist["creator"]["userId"].as_i64() == Some(uid))
}

pub(super) fn decode_saved_albums_page(body: &Value) -> Result<(Vec<Album>, bool)> {
    let albums: Vec<Album> = decode_ref(&body["data"])?;
    let more =
        body["hasMore"].as_bool().or_else(|| body["more"].as_bool()).unwrap_or(albums.len() >= 30);
    Ok((albums, more))
}

fn decode_playlist_page(
    body: &Value,
    include: impl Fn(&Value) -> bool,
) -> Result<(Vec<Playlist>, bool)> {
    let raw = body["playlist"].as_array().ok_or(Error::InvalidResponse)?;
    let page_len = raw.len();
    let playlists =
        raw.iter().filter(|playlist| include(playlist)).map(decode_ref).collect::<Result<_>>()?;
    let more = body["more"].as_bool().unwrap_or(page_len >= 30);
    Ok((playlists, more))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LikedPlaylistEntry {
    id: i64,
    added_at_ms: Option<i64>,
}

pub(super) fn decode_liked_playlist_id(body: &Value) -> Result<i64> {
    body["playlist"]
        .as_array()
        .and_then(|playlists| {
            playlists.iter().find(|playlist| playlist["specialType"].as_i64() == Some(5))
        })
        .and_then(|playlist| playlist["id"].as_i64())
        .filter(|id| *id > 0)
        .ok_or(Error::InvalidResponse)
}

pub(super) fn decode_liked_playlist_entries(body: &Value) -> Result<Vec<LikedPlaylistEntry>> {
    let entries = body["playlist"]["trackIds"].as_array().ok_or(Error::InvalidResponse)?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let id = entry["id"].as_i64().filter(|id| *id > 0)?;
            let added_at_ms = entry["at"]
                .as_i64()
                .or_else(|| entry["at"].as_str().and_then(|value| value.parse().ok()));
            Some(LikedPlaylistEntry { id, added_at_ms })
        })
        .collect())
}

pub(super) fn assemble_liked_songs(
    entries: Vec<LikedPlaylistEntry>,
    songs: Vec<Song>,
) -> Vec<LikedSong> {
    let by_id = songs.into_iter().map(|song| (song.id, song)).collect::<BTreeMap<_, _>>();
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            by_id.get(&entry.id).cloned().map(|song| LikedSong {
                song,
                favorite_order: i32::try_from(index.saturating_add(1)).unwrap_or(i32::MAX),
                added_at_ms: entry.added_at_ms,
            })
        })
        .collect()
}

pub(super) fn decode_recent_songs(body: &Value) -> Result<Vec<RecentSong>> {
    let list = body["data"]["list"].as_array().ok_or(Error::InvalidResponse)?;
    let mut songs = Vec::with_capacity(list.len());
    for entry in list {
        if entry["resourceType"].as_str().is_some_and(|kind| kind != "SONG") {
            continue;
        }
        let Some(played_at_ms) = entry["playTime"].as_i64() else { continue };
        let song: Song = decode_ref(&entry["data"])?;
        if song.id > 0 {
            songs.push(RecentSong { song, played_at_ms });
        }
    }
    Ok(songs)
}
