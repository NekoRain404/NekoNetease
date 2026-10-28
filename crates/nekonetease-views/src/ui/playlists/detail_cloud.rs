//! Cloud playlist projects: the remote `NetEase` rows the detail band and track list render,
//! and the comment preview that rides above them.
//!
//! Kept beside `detail` rather than in it so the open/refresh orchestration stays readable; the
//! projections are pure, and the comment load is the one async piece.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, SharedString, Weak};

use super::PlaylistsUi;
use crate::ui::detail_comments::comment_preview_model;
use crate::ui::util::len_as_i32;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::{Account, Playlist as CloudPlaylist, Song};
use nekonetease_core::entities::playlist::PlaylistStats;
use nekonetease_core::entities::track::TrackListRow as RsTrackListRow;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, PlaylistDetail};

/// Load recent comments for the open playlist without letting a comment failure fail the open.
///
/// `request` and `account_generation` are the caller's guards: a retry, a newer open or an
/// account switch must not paint an older answer over the current playlist.
pub(crate) async fn load_playlist_comments(
    state: &AppState,
    playlists_ui: &Arc<PlaylistsUi>,
    weak: &Weak<AppWindow>,
    playlist_id: i64,
    request: u64,
    account_generation: u64,
) {
    let write_loading = {
        let playlists_ui = playlists_ui.clone();
        let cloud = state.cloud.clone();
        move |ui: AppWindow| {
            if cloud.status().generation != account_generation
                || playlists_ui.detail_request.load(Ordering::Acquire) != request
            {
                return;
            }
            let g = ui.global::<PlaylistDetail>();
            g.set_comments_loading(true);
            g.set_comments_error(SharedString::default());
        }
    };
    let _ = weak.upgrade_in_event_loop(write_loading);

    // The model is built inside the UI closure: a `ModelRc` is an `Rc`, and the closure handed to
    // `upgrade_in_event_loop` must cross threads. The plain comments are `Send`; the model is not.
    let (comments, error) = match state.cloud.playlist_comments(playlist_id, 0).await {
        Ok((comments, _more)) => (comments, String::new()),
        Err(error) => (Vec::new(), describe(&error)),
    };

    let publish = {
        let playlists_ui = playlists_ui.clone();
        let cloud = state.cloud.clone();
        move |ui: AppWindow| {
            if cloud.status().generation != account_generation
                || playlists_ui.detail_request.load(Ordering::Acquire) != request
            {
                return;
            }
            let g = ui.global::<PlaylistDetail>();
            g.set_comments(comment_preview_model(&comments));
            g.set_comments_error(SharedString::from(error.as_str()));
            g.set_comments_loading(false);
        }
    };
    let _ = weak.upgrade_in_event_loop(publish);
}

#[cfg(test)]
#[path = "tests/detail_cloud_tests.rs"]
mod tests;

/// Whether the account may edit this playlist. `NetEase` marks a collected playlist `subscribed`;
/// its `creator` is authoritative when present, and the fallback covers responses that omit it.
pub(super) fn cloud_playlist_owned(playlist: &CloudPlaylist, account: Option<&Account>) -> bool {
    match &playlist.creator {
        Some(creator) => account.is_some_and(|account| creator.id == account.id),
        None => !playlist.subscribed,
    }
}

pub(super) fn cloud_playlist_stats(
    playlist: CloudPlaylist,
    thumbnail_path: Option<String>,
    songs: &[Song],
) -> PlaylistStats {
    PlaylistStats {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        thumbnail_path,
        is_smart: false,
        smart_criteria: None,
        created_at: String::new(),
        updated_at: String::new(),
        custom_thumbnail: false,
        track_count: len_as_i32(songs.len()),
        total_duration_ms: songs.iter().fold(0_i64, |sum, song| {
            sum.saturating_add(i64::try_from(song.duration_ms).unwrap_or(i64::MAX))
        }),
    }
}

pub(super) fn cloud_song_row(
    song: &Song,
    is_favorite: bool,
    artwork_path: Option<String>,
) -> RsTrackListRow {
    let artist = song.artist_names();
    let artist_id = song.artists.first().map(|artist| artist.id);
    RsTrackListRow {
        id: song.id,
        file_path: String::new(),
        file_name: song.name.clone(),
        title: song.name.clone(),
        artist: (!artist.is_empty()).then_some(artist),
        album_artist: None,
        album: (!song.album.name.is_empty()).then(|| song.album.name.clone()),
        genre: None,
        track_number: None,
        disc_number: None,
        year: None,
        duration_ms: i64::try_from(song.duration_ms).unwrap_or(i64::MAX),
        artwork_path,
        is_favorite,
        rating: 0,
        album_id: (song.album.id > 0).then_some(song.album.id),
        artist_id,
        genre_id: None,
        date_added: String::new(),
        sort_key: None,
        mv_id: song.mv_id(),
    }
}
