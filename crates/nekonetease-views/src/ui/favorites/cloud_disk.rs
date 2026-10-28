//! The account's uploaded cloud-disk songs and their paged track-list projection.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_compat::Compat;
use parking_lot::Mutex;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use super::{FavoritesTab, FavoritesUi};
use crate::ui::row_match::Needle;
use nekonetease_app::library::cloud_match::{self, NamedMatch};
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Song;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Favorites};

/// The task's page size. The endpoint reports the whole library's count, so the cache keeps
/// asking while it holds fewer.
const PAGE_SIZE: u64 = 50;

#[derive(Default)]
struct CloudDiskCache {
    generation: u64,
    songs: Vec<Song>,
    total: u64,
}

/// A completed mapping of the cached page. `songs` is the page size it covered, so a page loaded
/// afterwards is never counted as a miss.
#[derive(Clone, Default)]
struct DiskMatches {
    labels: HashMap<i64, String>,
    songs: usize,
}

#[derive(Default)]
pub(crate) struct CloudDiskState {
    cache: Mutex<Option<CloudDiskCache>>,
    matches: Mutex<Option<DiskMatches>>,
    revision: AtomicU64,
    match_request: AtomicU64,
    loading: AtomicBool,
    matching: AtomicBool,
    last_failed_offset: Mutex<Option<u64>>,
}

impl CloudDiskState {
    fn clear(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.match_request.fetch_add(1, Ordering::Relaxed);
        self.cache.lock().take();
        self.matches.lock().take();
        self.loading.store(false, Ordering::Release);
        self.matching.store(false, Ordering::Release);
        self.last_failed_offset.lock().take();
    }

    pub(super) fn invalidate_for_leave(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        // The cache and its mapping survive for the immediate repaint; only a run still in flight
        // must not leave the re-entered button disabled.
        self.matching.store(false, Ordering::Release);
        self.loading.store(false, Ordering::Release);
        self.last_failed_offset.lock().take();
    }

    fn record_failed_offset(&self, offset: u64) {
        *self.last_failed_offset.lock() = Some(offset);
    }

    fn clear_failed_offset(&self) {
        self.last_failed_offset.lock().take();
    }

    fn retry_offset(&self, generation: u64) -> u64 {
        if let Some(offset) = *self.last_failed_offset.lock() {
            return offset;
        }
        self.cache
            .lock()
            .as_ref()
            .filter(|cache| cache.generation == generation)
            .map_or(0, |cache| u64::try_from(cache.songs.len()).unwrap_or(u64::MAX))
    }

    pub(crate) fn cached_total(&self) -> u64 {
        self.cache.lock().as_ref().map_or(0, |cache| cache.total)
    }

    fn has_cached_generation(&self, generation: u64) -> bool {
        self.cache.lock().as_ref().is_some_and(|cache| cache.generation == generation)
    }

    pub(super) fn song_ids_matching(&self, needle: &Needle) -> Vec<i64> {
        self.cache.lock().as_ref().map_or_else(Vec::new, |cache| {
            cache
                .songs
                .iter()
                .filter(|song| song_matches(song, needle))
                .map(|song| song.id)
                .collect()
        })
    }
}

pub(super) fn wire(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    wire_load(ui, state, fav_ui);
    wire_match(ui, state, fav_ui);
    wire_account_updates(ui, state, fav_ui);
}

/// Returns whether a page was spawned, so a tab pick can rewind the count its cached paint
/// wrote only when a fetch is actually on the way.
pub(super) fn ensure_loaded(
    state: &AppState,
    fav_ui: &Arc<FavoritesUi>,
    weak: &Weak<AppWindow>,
) -> bool {
    let missing = !fav_ui.state().cloud_disk.has_cached_generation(state.cloud.status().generation);
    if missing {
        fav_ui.mark_cloud_disk_dirty();
    }
    let asked = missing || fav_ui.take_cloud_disk_dirty();
    if asked {
        spawn_page(state, fav_ui, weak, 0);
    }
    asked
}

fn wire_load(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_retry_cloud_disk(move || {
        let offset = fu.state().cloud_disk.retry_offset(s.cloud.status().generation);
        spawn_page(&s, &fu, &weak, offset);
    });

    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_load_more_cloud_disk(move || {
        let offset = {
            let cache = fu.state().cloud_disk.cache.lock();
            cache
                .as_ref()
                .filter(|cache| {
                    cache.generation == s.cloud.status().generation
                        && u64::try_from(cache.songs.len()).unwrap_or(u64::MAX) < cache.total
                })
                .map(|cache| u64::try_from(cache.songs.len()).unwrap_or(u64::MAX))
        };
        if let Some(offset) = offset {
            spawn_page(&s, &fu, &weak, offset);
        }
    });
}

fn wire_match(ui: &AppWindow, state: &AppState, fav_ui: &Arc<FavoritesUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let fu = fav_ui.clone();
    ui.global::<Favorites>().on_match_cloud_disk(move || {
        spawn_match(&s, &fu, &weak);
    });
}

/// Maps the cached page against the local library. Read-only: the answer is kept beside the cache
/// and only patches what is already on screen.
fn spawn_match(state: &AppState, fav_ui: &Arc<FavoritesUi>, weak: &Weak<AppWindow>) {
    if !fav_ui.section_active() || fav_ui.active_tab() != FavoritesTab::CloudDisk {
        return;
    }
    let generation = state.cloud.status().generation;
    let songs = {
        let cache = fav_ui.state().cloud_disk.cache.lock();
        let Some(cache) = cache.as_ref().filter(|cache| cache.generation == generation) else {
            return;
        };
        cache.songs.clone()
    };
    let Some(ui) = weak.upgrade() else { return };
    let cloud = &fav_ui.state().cloud_disk;
    if cloud.matching.swap(true, Ordering::AcqRel) {
        return;
    }
    let request = cloud.match_request.fetch_add(1, Ordering::Relaxed) + 1;
    let covered = songs.len();
    ui.global::<Favorites>().set_cloud_disk_matching(true);
    ui.global::<Favorites>().set_cloud_disk_match_error(SharedString::default());

    let state = state.clone();
    let fav_ui = fav_ui.clone();
    let weak = weak.clone();
    state.runtime.clone().spawn(async move {
        let result = cloud_match::match_cloud_disk_named(&state.db, &songs).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let cloud = &fav_ui.state().cloud_disk;
            // A newer run owns the flag, and an account switch already cleared both.
            if cloud.match_request.load(Ordering::Relaxed) != request {
                return;
            }
            cloud.matching.store(false, Ordering::Release);
            let g = ui.global::<Favorites>();
            g.set_cloud_disk_matching(false);
            match result {
                Ok(found) => {
                    *cloud.matches.lock() = Some(DiskMatches {
                        labels: found
                            .iter()
                            .map(|found| (found.cloud_song_id, label_for(found)))
                            .collect(),
                        songs: covered,
                    });
                    g.set_cloud_disk_match_error(SharedString::default());
                    apply_now(&ui, &state, &fav_ui);
                }
                Err(error) => {
                    g.set_cloud_disk_match_error(describe(&error).into());
                }
            }
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
            fu.state().cloud_disk.clear();
            fu.mark_cloud_disk_dirty();
            let g = ui.global::<Favorites>();
            g.set_cloud_disk_tracks(ModelRc::default());
            g.set_cloud_disk_count(-1);
            g.set_cloud_disk_loading(false);
            g.set_cloud_disk_loading_more(false);
            g.set_cloud_disk_has_more(false);
            g.set_cloud_disk_error(SharedString::default());
            g.set_cloud_disk_matching(false);
            g.set_cloud_disk_match_error(SharedString::default());
            g.set_cloud_disk_matched_count(-1);
            g.set_cloud_disk_match_total(0);
            if fu.section_active() && fu.active_tab() == FavoritesTab::CloudDisk {
                spawn_page(&s, &fu, &weak, 0);
            }
        }
    }));
}

pub(super) fn spawn_page(
    state: &AppState,
    fav_ui: &Arc<FavoritesUi>,
    weak: &Weak<AppWindow>,
    offset: u64,
) {
    if !fav_ui.section_active() || fav_ui.active_tab() != FavoritesTab::CloudDisk {
        return;
    }
    if fav_ui.state().cloud_disk.loading.swap(true, Ordering::AcqRel) {
        return;
    }
    fav_ui.take_cloud_disk_dirty();

    let generation = state.cloud.status().generation;
    let request = fav_ui.state().cloud_disk.revision.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<Favorites>();
        if offset == 0 {
            g.set_cloud_disk_loading(true);
        } else {
            g.set_cloud_disk_loading_more(true);
        }
        g.set_cloud_disk_error(SharedString::default());
    });

    let state = state.clone();
    let fav_ui = fav_ui.clone();
    let weak = weak.clone();
    state.runtime.clone().spawn(async move {
        let result = state.cloud.cloud_disk_page(offset, PAGE_SIZE).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let cloud = &fav_ui.state().cloud_disk;
            if cloud.revision.load(Ordering::Relaxed) != request {
                return;
            }
            if state.cloud.status().generation != generation {
                cloud.loading.store(false, Ordering::Release);
                fav_ui.mark_cloud_disk_dirty();
                return;
            }
            cloud.loading.store(false, Ordering::Release);
            let g = ui.global::<Favorites>();
            g.set_cloud_disk_loading(false);
            g.set_cloud_disk_loading_more(false);
            match result {
                Ok(page) => {
                    cloud.clear_failed_offset();
                    if offset == 0 {
                        let mut songs = Vec::with_capacity(page.songs.len());
                        merge_songs(&mut songs, page.songs);
                        *cloud.cache.lock() =
                            Some(CloudDiskCache { generation, songs, total: page.total });
                        // A fresh page replaces the one the mapping covered, so its summary no
                        // longer describes what is on screen; the user asks again to map it.
                        *cloud.matches.lock() = None;
                    } else {
                        let mut cache = cloud.cache.lock();
                        let Some(cache) =
                            cache.as_mut().filter(|cache| cache.generation == generation)
                        else {
                            g.set_cloud_disk_error(
                                "Cloud disk list expired. Retry to load again.".into(),
                            );
                            return;
                        };
                        let added = merge_songs(&mut cache.songs, page.songs);
                        if page.total > 0 {
                            cache.total = page.total;
                        }
                        // A page that added nothing is the end of what the server will hand over,
                        // whatever its count says; otherwise the pill re-asks an offset that never
                        // moves.
                        if added == 0 {
                            cache.total = u64::try_from(cache.songs.len()).unwrap_or(u64::MAX);
                        }
                    }
                    fav_ui.take_cloud_disk_dirty();
                    g.set_cloud_disk_error(SharedString::default());
                    apply_now(&ui, &state, &fav_ui);
                }
                Err(error) => {
                    cloud.record_failed_offset(offset);
                    fav_ui.mark_cloud_disk_dirty();
                    g.set_cloud_disk_error(describe(&error).into());
                }
            }
        });
    });
}

/// Appends the songs not already cached, in server order, and answers how many were new.
fn merge_songs(songs: &mut Vec<Song>, page: Vec<Song>) -> usize {
    let before = songs.len();
    let mut seen: HashSet<i64> = songs.iter().map(|song| song.id).collect();
    songs.extend(page.into_iter().filter(|song| seen.insert(song.id)));
    songs.len() - before
}

pub(super) fn paint_on_enter(weak: &Weak<AppWindow>, state: &AppState, fav_ui: &FavoritesUi) {
    if fav_ui.active_tab() == FavoritesTab::CloudDisk
        && let Some(ui) = weak.upgrade()
    {
        apply_now(&ui, state, fav_ui);
    }
}

fn song_matches(song: &Song, needle: &Needle) -> bool {
    needle.contains(&song.name)
        || needle.contains(&song.artist_names())
        || needle.contains(&song.album.name)
}

pub(super) fn apply_now(ui: &AppWindow, state: &AppState, fav_ui: &FavoritesUi) {
    if !fav_ui.section_active() || fav_ui.active_tab() != FavoritesTab::CloudDisk {
        return;
    }
    let cloud = &fav_ui.state().cloud_disk;
    let (songs, total, has_more) = {
        let cache = cloud.cache.lock();
        let Some(cache) = cache.as_ref() else {
            return;
        };
        let needle = fav_ui.state().filter.lock().clone();
        let songs = cache
            .songs
            .iter()
            .filter(|song| song_matches(song, &needle))
            .cloned()
            .collect::<Vec<_>>();
        let total = cache.total;
        let cached = u64::try_from(cache.songs.len()).unwrap_or(u64::MAX);
        (songs, total, cached < total)
    };
    let matched = cloud.matches.lock().clone();
    let g = ui.global::<Favorites>();
    g.set_cloud_disk_count(i32::try_from(total).unwrap_or(i32::MAX));
    g.set_cloud_disk_has_more(has_more);
    let mut rows = crate::ui::callbacks::cloud_tracks::cloud_song_rows(songs, state);
    if let Some(matches) = &matched {
        for row in &mut rows {
            if let Some(label) = crate::ui::util::parse_cloud_id(row.remote_id.as_str())
                .and_then(|id| matches.labels.get(&id))
            {
                row.local_match = label.as_str().into();
            }
        }
    }
    let (matched_count, match_total) = matched_summary(matched.as_ref()).unwrap_or((-1, 0));
    g.set_cloud_disk_matched_count(matched_count);
    g.set_cloud_disk_match_total(match_total);
    g.set_cloud_disk_tracks(ModelRc::from(Rc::new(VecModel::from(rows))));
    crate::ui::hero_chips::publish_favorites(ui, fav_ui);
}

/// The toolbar's "N of M matched", or `None` until the user has asked. M is the page the run
/// covered, not the rows on screen now, which a filter or a later page would misstate.
fn matched_summary(matches: Option<&DiskMatches>) -> Option<(i32, i32)> {
    matches.map(|matches| (count(matches.labels.len()), count(matches.songs)))
}

/// The row's tooltip: the local track's own title, plus its artist when the tag carries one.
fn label_for(found: &NamedMatch) -> String {
    match found.artist.as_deref().filter(|artist| !artist.is_empty()) {
        Some(artist) => format!("{} · {}", found.title, artist),
        None => found.title.clone(),
    }
}

fn count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
#[path = "tests/cloud_disk_tests.rs"]
mod tests;
