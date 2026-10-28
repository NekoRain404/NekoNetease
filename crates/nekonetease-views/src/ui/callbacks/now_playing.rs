//! `Player.toggle-favorite` fans out into Tracks / Browse / the four detail views / Search rows.

use std::sync::Arc;

use slint::{ComponentHandle, Global as _};

use crate::ui::albums::AlbumsUi;
use crate::ui::artists::ArtistsUi;
use crate::ui::browse::{self as browse_ui_mod, BrowseUi};
use crate::ui::genres::GenresUi;
use crate::ui::playlists::PlaylistsUi;
use crate::ui::search::{self as search_ui_mod, SearchUi};
use crate::ui::track_detail::DetailRows;
use crate::ui::tracks::{self as tracks_ui_mod, TracksUi};
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::{lock_state, with_state_emit};
use nekonetease_ui::{AlbumDetail, AppWindow, ArtistDetail, GenreDetail, Player, PlaylistDetail};

/// The view-side surfaces holding per-row favorite state. Each patch no-ops where the id isn't
/// listed, so mirroring into all of them on every change is safe.
///
/// Search is here because the bar's heart is on screen while its results are, and it is the
/// one surface with no `library_changed` subscriber to correct a missed patch later.
#[derive(Clone)]
struct CurrentTrackMirrors {
    tracks: Arc<TracksUi>,
    browse: Arc<BrowseUi>,
    albums: DetailRows<AlbumsUi>,
    artists: DetailRows<ArtistsUi>,
    genres: DetailRows<GenresUi>,
    playlists: DetailRows<PlaylistsUi>,
    search: Arc<SearchUi>,
    weak: slint::Weak<AppWindow>,
}

impl CurrentTrackMirrors {
    fn mirror_favorite(&self, id: i64, fav: bool) {
        self.tracks.flip_favorite(id, fav);
        tracks_ui_mod::apply_row_favorite(&self.weak, id, fav);
        self.browse.flip_favorite(id, fav);
        browse_ui_mod::apply_row_favorite(&self.weak, id, fav);
        self.albums.mirror_favorite(id, fav);
        self.artists.mirror_favorite(id, fav);
        self.genres.mirror_favorite(id, fav);
        self.playlists.mirror_favorite(id, fav);
        self.search.flip_favorite(id, fav);
        search_ui_mod::apply_row_favorite(&self.weak, id, fav);
    }
}

fn mirrors(ui: &AppWindow, handles: RowFlagHandles<'_>) -> CurrentTrackMirrors {
    CurrentTrackMirrors {
        tracks: handles.tracks.clone(),
        browse: handles.browse.clone(),
        albums: DetailRows::new(handles.albums, ui.global::<AlbumDetail>().as_weak()),
        artists: DetailRows::new(handles.artists, ui.global::<ArtistDetail>().as_weak()),
        genres: DetailRows::new(handles.genres, ui.global::<GenreDetail>().as_weak()),
        playlists: DetailRows::new(handles.playlists, ui.global::<PlaylistDetail>().as_weak()),
        search: handles.search.clone(),
        weak: ui.as_weak(),
    }
}

/// The handles both fan-outs take, borrowed rather than cloned — a parameter list this long
/// reads as an ordering nobody can check at the call site.
#[derive(Clone, Copy)]
pub struct RowFlagHandles<'a> {
    pub tracks: &'a Arc<TracksUi>,
    pub browse: &'a Arc<BrowseUi>,
    pub albums: &'a Arc<AlbumsUi>,
    pub artists: &'a Arc<ArtistsUi>,
    pub genres: &'a Arc<GenresUi>,
    pub playlists: &'a Arc<PlaylistsUi>,
    pub search: &'a Arc<SearchUi>,
}

/// Wire `Player.toggle-favorite` (heart in the Now Playing view, the
/// now-playing-bar, and the queue/track overflow menu). Cloud tracks update
/// the `NetEase` liked-song list; local files keep their local-library flag.
/// Local row caches are mirrored after their database write succeeds.
///
/// Call once after every `install` whose handle [`RowFlagHandles`] carries.
pub fn wire_now_playing_favorite(ui: &AppWindow, state: &AppState, handles: RowFlagHandles<'_>) {
    let s = state.clone();
    let m = mirrors(ui, handles);
    ui.global::<Player>().on_toggle_favorite(move || {
        let s = s.clone();
        let m = m.clone();
        let Some(current) = lock_state(&s.player_state).source.clone() else {
            return;
        };
        let id = current.id;
        let liked = !current.is_favorite;
        let is_cloud = current.file_path.is_empty();
        s.runtime.clone().spawn(async move {
            if is_cloud {
                let generation = s.cloud.status().generation;
                if let Err(error) = s.cloud.like(id, liked).await {
                    log::warn!("netease_like({id}, {liked}): {}", describe(&error));
                    return;
                }
                if s.cloud.status().generation != generation {
                    return;
                }
                with_state_emit(&s.player_state, &s.sinks, |player| {
                    if let Some(track) = player.source.as_mut()
                        && track.id == id
                        && track.file_path.is_empty()
                    {
                        Arc::make_mut(track).is_favorite = liked;
                    }
                    for track in &mut player.queue.tracks {
                        if track.id == id && track.file_path.is_empty() {
                            Arc::make_mut(track).is_favorite = liked;
                        }
                    }
                });
                s.cloud_liked_changed.bump();
                return;
            }

            match library::favorites::toggle_current_favorite(&s).await {
                Ok(Some((id, fav))) => m.mirror_favorite(id, fav),
                Ok(None) => {}
                Err(e) => log::warn!("toggle_favorite: {}", describe(&e)),
            }
        });
    });
}
