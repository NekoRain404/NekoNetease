//! Account-scoped `NetEase` artist subscriptions and their paged card projection.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_compat::Compat;
use parking_lot::Mutex;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use super::{FavoritesTab, FavoritesUi};
use crate::ui::row_match::Needle;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Artist;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, CloudAlbumGridRow, CloudAlbumRow, Favorites};

const PAGE_SIZE: usize = 30;

#[derive(Default)]
struct SavedArtistCache {
    generation: u64,
    artists: Vec<Artist>,
    next_offset: usize,
    has_more: bool,
}

#[derive(Default)]
pub(crate) struct SavedArtistsState {
    cache: Mutex<Option<SavedArtistCache>>,
    revision: AtomicU64,
    pending: Mutex<HashSet<i64>>,
    loading: AtomicBool,
    last_failed_offset: Mutex<Option<usize>>,
}

impl SavedArtistsState {
    fn clear(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.cache.lock().take();
        self.pending.lock().clear();
        self.loading.store(false, Ordering::Release);
        self.last_failed_offset.lock().take();
    }

    pub(super) fn invalidate_for_leave(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.loading.store(false, Ordering::Release);
        self.last_failed_offset.lock().take();
    }

    fn record_failed_offset(&self, offset: usize) {
        *self.last_failed_offset.lock() = Some(offset);
    }

    fn clear_failed_offset(&self) {
        self.last_failed_offset.lock().take();
    }

    fn retry_offset(&self, generation: u64) -> usize {
        if let Some(offset) = *self.last_failed_offset.lock() {
            return offset;
        }
        self.cache
            .lock()
            .as_ref()
            .filter(|cache| cache.generation == generation)
            .map_or(0, |cache| cache.next_offset)
    }

    pub(crate) fn cached_count(&self) -> Option<usize> {
        self.cache.lock().as_ref().filter(|cache| !cache.has_more).map(|cache| cache.artists.len())
    }

    fn has_cached_generation(&self, generation: u64) -> bool {
        self.cache.lock().as_ref().is_some_and(|cache| cache.generation == generation)
    }
}

pub(super) fn wire(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    wire_covers(ui, state);
    wire_load(ui, state, fav_ui);
    wire_membership(ui, state, fav_ui);
    wire_account_updates(ui, state, fav_ui);
}

fn wire_covers(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    let covers = Arc::new(crate::ui::remote_covers::RemoteCoverThumbs::new_for_cards(
        state.http_client().clone(),
        state.runtime.clone(),
        move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                let g = ui.global::<Favorites>();
                g.set_cloud_artist_covers_generation(
                    g.get_cloud_artist_covers_generation().wrapping_add(1),
                );
            });
        },
    ));
    ui.global::<Favorites>()
        .on_request_cloud_artist_cover(move |source, _| covers.get_or_schedule(source.as_str()));
}

pub(super) fn ensure_loaded(state: &AppState, fav_ui: &Arc<FavoritesUi>, weak: &Weak<AppWindow>) {
    let missing =
        !fav_ui.state().cloud_artists.has_cached_generation(state.cloud.status().generation);
    if missing {
        fav_ui.mark_cloud_artists_dirty();
    }
    if missing || fav_ui.take_cloud_artists_dirty() {
        spawn_page(state, fav_ui, weak, 0);
    }
}

fn wire_load(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_retry_cloud_artists(move || {
        let offset = fu.state().cloud_artists.retry_offset(s.cloud.status().generation);
        spawn_page(&s, &fu, &weak, offset);
    });

    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_load_more_cloud_artists(move || {
        let offset = {
            let cache = fu.state().cloud_artists.cache.lock();
            cache
                .as_ref()
                .filter(|cache| cache.has_more && cache.generation == s.cloud.status().generation)
                .map(|cache| cache.next_offset)
        };
        if let Some(offset) = offset {
            spawn_page(&s, &fu, &weak, offset);
        }
    });
}

fn wire_membership(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_set_cloud_artist_favorite(move |remote_id, saved| {
        let Some(id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else { return };
        let Some(ui) = weak.upgrade() else { return };
        if !fu.state().cloud_artists.pending.lock().insert(id) {
            return;
        }
        let generation = s.cloud.status().generation;
        fu.state().cloud_artists.revision.fetch_add(1, Ordering::Relaxed);
        fu.state().cloud_artists.loading.store(false, Ordering::Release);
        ui.global::<Favorites>().set_cloud_artists_loading(false);
        ui.global::<Favorites>().set_cloud_artists_loading_more(false);
        ui.global::<Favorites>().set_cloud_artist_action_error(SharedString::default());
        apply_now(&ui, &fu);

        let s = s.clone();
        let fu = fu.clone();
        let w = weak.clone();
        let refresh_base_weak = w.clone();
        s.runtime.clone().spawn(async move {
            let result = s.cloud.subscribe_artist(id, saved).await;
            let _ = w.upgrade_in_event_loop(move |ui| {
                if s.cloud.status().generation != generation {
                    return;
                }
                let cloud = &fu.state().cloud_artists;
                cloud.pending.lock().remove(&id);
                cloud.revision.fetch_add(1, Ordering::Relaxed);
                match result {
                    Ok(()) => {
                        if let Some(cache) = cloud.cache.lock().as_mut()
                            && cache.generation == generation
                        {
                            if saved {
                                // Saving from another surface changes the first page order; reload
                                // it instead of guessing where NetEase inserted the artist.
                                cache.artists.retain(|artist| artist.id != id);
                                cache.next_offset = 0;
                                cache.has_more = true;
                            } else {
                                cache.artists.retain(|artist| artist.id != id);
                                cache.next_offset = cache.next_offset.saturating_sub(1);
                                // The NetEase list shifted left, so the next page offset must be
                                // the current row count; this refills the short page without skips.
                            }
                        }
                        let g = ui.global::<Favorites>();
                        g.set_cloud_artist_action_error(SharedString::default());
                        apply_now(&ui, &fu);
                        if saved {
                            fu.mark_cloud_artists_dirty();
                            let refresh_weak = refresh_base_weak.clone();
                            spawn_page(&s, &fu, &refresh_weak, 0);
                        } else if cloud.cache.lock().as_ref().is_some_and(|cache| cache.has_more) {
                            if fu.section_active() && fu.active_tab() == FavoritesTab::CloudArtists
                            {
                                let offset = cloud
                                    .cache
                                    .lock()
                                    .as_ref()
                                    .map_or(0, |cache| cache.next_offset);
                                fu.mark_cloud_artists_dirty();
                                let refresh_weak = refresh_base_weak.clone();
                                spawn_page(&s, &fu, &refresh_weak, offset);
                            } else {
                                fu.mark_cloud_artists_dirty();
                            }
                        }
                    }
                    Err(error) => {
                        // Writes are never retried automatically: the server may have accepted one.
                        let message = describe(&error);
                        ui.global::<Favorites>().set_cloud_artist_action_error(message.into());
                        apply_now(&ui, &fu);
                    }
                }
            });
        });
    });
}

fn wire_account_updates(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let mut account = state.cloud.subscribe();
    let mut generation = state.cloud.status().generation;
    let s = state.clone();
    let fu = fav_ui.clone();
    let weak = ui.as_weak();
    let _ = slint::spawn_local(Compat::new(async move {
        while account.changed().await.is_ok() {
            let current = s.cloud.status().generation;
            if current == generation {
                continue;
            }
            generation = current;
            let Some(ui) = weak.upgrade() else { break };
            fu.state().cloud_artists.clear();
            fu.mark_cloud_artists_dirty();
            let g = ui.global::<Favorites>();
            g.set_cloud_artist_rows(ModelRc::default());
            g.set_cloud_artist_count(-1);
            g.set_filtered_cloud_artist_count(0);
            g.set_cloud_artists_loading(false);
            g.set_cloud_artists_loading_more(false);
            g.set_cloud_artists_has_more(false);
            g.set_cloud_artists_error(SharedString::default());
            g.set_cloud_artist_action_error(SharedString::default());
            if fu.section_active() && fu.active_tab() == FavoritesTab::CloudArtists {
                spawn_page(&s, &fu, &weak, 0);
            }
        }
    }));
}

pub(super) fn spawn_page(
    state: &AppState,
    fav_ui: &Arc<FavoritesUi>,
    weak: &Weak<AppWindow>,
    offset: usize,
) {
    if !fav_ui.section_active() || fav_ui.active_tab() != FavoritesTab::CloudArtists {
        return;
    }
    if fav_ui.state().cloud_artists.loading.swap(true, Ordering::AcqRel) {
        return;
    }
    fav_ui.take_cloud_artists_dirty();

    let generation = state.cloud.status().generation;
    let request = fav_ui.state().cloud_artists.revision.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<Favorites>();
        if offset == 0 {
            g.set_cloud_artists_loading(true);
        } else {
            g.set_cloud_artists_loading_more(true);
        }
        g.set_cloud_artists_error(SharedString::default());
    });

    let state = state.clone();
    let fav_ui = fav_ui.clone();
    let weak = weak.clone();
    state.runtime.clone().spawn(async move {
        let result = state.cloud.saved_artists(u64::try_from(offset).unwrap_or(u64::MAX)).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let cloud = &fav_ui.state().cloud_artists;
            if cloud.revision.load(Ordering::Relaxed) != request {
                return;
            }
            if state.cloud.status().generation != generation {
                cloud.loading.store(false, Ordering::Release);
                fav_ui.mark_cloud_artists_dirty();
                return;
            }
            cloud.loading.store(false, Ordering::Release);
            let g = ui.global::<Favorites>();
            g.set_cloud_artists_loading(false);
            g.set_cloud_artists_loading_more(false);
            match result {
                Ok(page) => {
                    cloud.clear_failed_offset();
                    let cache = if offset == 0 {
                        let mut cache =
                            SavedArtistCache { generation, ..SavedArtistCache::default() };
                        let page_len = merge_page(&mut cache.artists, page);
                        cache.next_offset = page_len;
                        cache.has_more = page_should_continue(page_len, cache.artists.len());
                        cache
                    } else {
                        let mut cache = cloud.cache.lock();
                        let Some(cache) =
                            cache.as_mut().filter(|cache| cache.generation == generation)
                        else {
                            g.set_cloud_artists_error(
                                "Artist list expired. Retry to load again.".into(),
                            );
                            return;
                        };
                        let before = cache.artists.len();
                        let page_len = merge_page(&mut cache.artists, page);
                        let added = cache.artists.len().saturating_sub(before);
                        cache.next_offset = offset.saturating_add(page_len);
                        cache.has_more = page_should_continue(page_len, added);
                        fav_ui.take_cloud_artists_dirty();
                        g.set_cloud_artists_error(SharedString::default());
                        apply_now(&ui, &fav_ui);
                        return;
                    };
                    *cloud.cache.lock() = Some(cache);
                    fav_ui.take_cloud_artists_dirty();
                    g.set_cloud_artists_error(SharedString::default());
                    apply_now(&ui, &fav_ui);
                }
                Err(error) => {
                    cloud.record_failed_offset(offset);
                    fav_ui.mark_cloud_artists_dirty();
                    g.set_cloud_artists_error(describe(&error).into());
                }
            }
        });
    });
}

fn page_has_more(page_len: usize) -> bool {
    page_len >= PAGE_SIZE
}

fn page_should_continue(page_len: usize, added: usize) -> bool {
    page_has_more(page_len) && added > 0
}

fn merge_page(artists: &mut Vec<Artist>, page: Vec<Artist>) -> usize {
    let page_len = page.len();
    let mut seen: HashSet<i64> = artists.iter().map(|artist| artist.id).collect();
    artists.extend(page.into_iter().filter(|artist| seen.insert(artist.id)));
    page_len
}

pub(super) fn paint_on_enter(weak: &Weak<AppWindow>, fav_ui: &FavoritesUi) {
    if fav_ui.active_tab() == FavoritesTab::CloudArtists
        && let Some(ui) = weak.upgrade()
    {
        apply_now(&ui, fav_ui);
    }
}

fn artist_matches(artist: &Artist, needle: &Needle) -> bool {
    needle.contains(&artist.name)
}

fn artist_row(artist: &Artist, pending: bool) -> CloudAlbumRow {
    let artwork = artist
        .image_url()
        .filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default();
    CloudAlbumRow {
        remote_id: artist.id.to_string().into(),
        title: artist.name.as_str().into(),
        artist: SharedString::default(),
        artwork_path: artwork.into(),
        pending,
    }
}

pub(super) fn apply_now(ui: &AppWindow, fav_ui: &FavoritesUi) {
    if !fav_ui.section_active() || fav_ui.active_tab() != FavoritesTab::CloudArtists {
        return;
    }
    let g = ui.global::<Favorites>();
    let cloud = &fav_ui.state().cloud_artists;
    let (rows, count, has_more) = {
        let cache = cloud.cache.lock();
        let Some(cache) = cache.as_ref() else {
            return;
        };
        let needle = fav_ui.state().filter.lock().clone();
        let pending = cloud.pending.lock();
        let rows = cache
            .artists
            .iter()
            .filter(|artist| artist_matches(artist, &needle))
            .map(|artist| artist_row(artist, pending.contains(&artist.id)))
            .collect::<Vec<_>>();
        (rows, cache.artists.len(), cache.has_more)
    };
    g.set_cloud_artist_count(crate::ui::util::len_as_i32(count));
    g.set_filtered_cloud_artist_count(crate::ui::util::len_as_i32(rows.len()));
    g.set_cloud_artists_has_more(has_more);
    let columns = usize::try_from(g.get_columns().max(1)).unwrap_or(1);
    let grid = rows
        .chunks(columns)
        .map(|row| CloudAlbumGridRow {
            albums: ModelRc::from(Rc::new(VecModel::from(row.to_vec()))),
        })
        .collect::<Vec<_>>();
    g.set_cloud_artist_rows(ModelRc::from(Rc::new(VecModel::from(grid))));
    crate::ui::hero_chips::publish_favorites(ui, fav_ui);
}

#[cfg(test)]
#[path = "tests/saved_artists_tests.rs"]
mod tests;
