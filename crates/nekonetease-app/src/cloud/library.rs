//! The signed-in account's library: recommendations, collections, and subscriptions.

use std::collections::HashSet;
use std::sync::Arc;

use nekonetease_core::entities::cloud::{
    Album, Artist, DAILY_PLAYLIST_ID, MusicLibraryCategory, Playlist, RoamingMode, Song,
};
use nekonetease_core::error::{AppError, AppResult};

use super::CloudService;
use super::validation::validate_id;

impl CloudService {
    pub async fn daily(&self) -> AppResult<Vec<Song>> {
        self.private_request(self.client.daily()).await
    }

    /// Daily recommendations use the same detail surface as ordinary remote playlists.
    pub async fn library_playlist(&self, id: i64) -> AppResult<(Playlist, Arc<[Song]>)> {
        if id != DAILY_PLAYLIST_ID {
            return self.saved_playlist(id).await;
        }
        let songs = self.daily().await?;
        let playlist = Playlist {
            id,
            name: "每日推荐".to_owned(),
            track_count: u64::try_from(songs.len()).unwrap_or(u64::MAX),
            cover_url: songs.first().and_then(|song| song.album.cover_url.clone()),
            ..Playlist::default()
        };
        Ok((playlist, songs.into()))
    }

    pub async fn recommendations(&self) -> AppResult<Vec<Playlist>> {
        self.private_request(self.client.recommendations()).await
    }

    /// Fetch a roaming batch using an actual `NetEase` song ID as its seed.
    pub async fn roaming_songs(
        &self,
        mode: RoamingMode,
        seed: Option<i64>,
    ) -> AppResult<Vec<Song>> {
        match mode {
            RoamingMode::Personal => self.fm().await,
            RoamingMode::Similar => {
                let id = seed.ok_or_else(|| {
                    AppError::Validation("请先播放一首歌曲，再开启相似歌曲".to_owned())
                })?;
                validate_id("歌曲", id)?;
                self.private_request(self.client.similar_songs(id)).await
            }
            RoamingMode::Heart => {
                let (account, cancellation) = self.account_snapshot()?;
                self.account_request_in_generation(
                    async {
                        let playlist_id = self.client.liked_playlist_id(account.id).await?;
                        let liked = self.client.liked_ids(account.id).await?;
                        let liked_seed =
                            seed.filter(|id| liked.contains(id)).or_else(|| liked.first().copied());
                        let Some(liked_seed) = liked_seed else {
                            return Ok(Vec::new());
                        };
                        self.client
                            .intelligence(liked_seed, playlist_id, seed.unwrap_or(liked_seed))
                            .await
                    },
                    cancellation,
                )
                .await
            }
        }
    }

    pub async fn fm(&self) -> AppResult<Vec<Song>> {
        self.private_request(self.client.fm()).await
    }

    pub async fn fm_trash(&self, song_id: i64) -> AppResult<()> {
        validate_id("歌曲", song_id)?;
        self.private_request(self.client.fm_trash(song_id)).await
    }

    pub async fn user_playlists(&self, offset: u64) -> AppResult<Vec<Playlist>> {
        let (account, cancellation) = self.account_snapshot()?;
        self.account_request_in_generation(
            self.client.user_playlists(account.id, offset),
            cancellation,
        )
        .await
    }

    /// Playlists the signed-in user has collected, excluding playlists they created.
    pub async fn saved_playlists(&self, offset: u64) -> AppResult<Vec<Playlist>> {
        self.user_playlists(offset).await
    }

    pub async fn saved_playlists_page(&self, offset: u64) -> AppResult<(Vec<Playlist>, bool)> {
        let (account, cancellation) = self.account_snapshot()?;
        self.account_request_in_generation(
            self.client.user_playlists_page(account.id, offset),
            cancellation,
        )
        .await
    }

    pub async fn library_playlists_page(
        &self,
        category: MusicLibraryCategory,
        offset: u64,
    ) -> AppResult<(Vec<Playlist>, bool)> {
        match category {
            MusicLibraryCategory::Collected => self.saved_playlists_page(offset).await,
            MusicLibraryCategory::Created => {
                let (account, cancellation) = self.account_snapshot()?;
                self.account_request_in_generation(
                    self.client.created_playlists_page(account.id, offset),
                    cancellation,
                )
                .await
            }
            MusicLibraryCategory::Roaming => Ok((Vec::new(), false)),
        }
    }

    pub async fn saved_albums(&self, offset: u64) -> AppResult<Vec<Album>> {
        self.private_request(self.client.saved_albums(offset)).await
    }

    /// Read the entire collection in server order under one account cancellation scope.
    pub async fn all_saved_albums(&self) -> AppResult<Vec<Album>> {
        self.private_request(async {
            let mut albums = Vec::new();
            let mut seen = HashSet::new();
            let mut offset = 0;
            loop {
                let (page, more) = self.client.saved_albums_page(offset).await?;
                let page_len = page.len();
                let before = albums.len();
                albums.extend(page.into_iter().filter(|album| seen.insert(album.id)));
                // A repeated page must not spin forever if the server ignores its offset.
                if !more || page_len == 0 || albums.len() == before {
                    break;
                }
                offset += u64::try_from(page_len).unwrap_or(u64::MAX);
            }
            Ok(albums)
        })
        .await
    }

    pub async fn saved_artists(&self, offset: u64) -> AppResult<Vec<Artist>> {
        self.private_request(self.client.saved_artists(offset)).await
    }

    pub async fn subscribe_playlist(&self, id: i64, subscribe: bool) -> AppResult<()> {
        validate_id("歌单", id)?;
        self.private_request(self.client.subscribe_playlist(id, subscribe)).await
    }

    pub async fn subscribe_album(&self, id: i64, subscribe: bool) -> AppResult<()> {
        validate_id("专辑", id)?;
        self.private_request(self.client.subscribe_album(id, subscribe)).await
    }

    pub async fn subscribe_artist(&self, id: i64, subscribe: bool) -> AppResult<()> {
        validate_id("歌手", id)?;
        self.private_request(self.client.subscribe_artist(id, subscribe)).await
    }
}
