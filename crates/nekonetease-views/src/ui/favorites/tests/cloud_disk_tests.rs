use super::{
    CloudDiskCache, CloudDiskState, DiskMatches, PAGE_SIZE, label_for, matched_summary,
    merge_songs, song_matches,
};
use crate::ui::row_match::fold_needle;
use nekonetease_app::library::cloud_match::NamedMatch;
use nekonetease_core::entities::cloud::{Album, Artist, Song};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

const DISK_TAB: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/favorites/cloud-favorites-disk-tab.slint");

fn song(id: i64, name: &str, artist: &str, album: &str) -> Song {
    Song {
        id,
        name: name.into(),
        artists: vec![Artist { id: 1, name: artist.into(), ..Artist::default() }],
        album: Album { id: 2, name: album.into(), ..Album::default() },
        ..Song::default()
    }
}

fn seeded(generation: u64, songs: Vec<Song>, total: u64) -> CloudDiskState {
    let state = CloudDiskState::default();
    *state.cache.lock() = Some(CloudDiskCache { generation, songs, total });
    state
}

#[test]
fn a_page_is_requested_fifty_songs_at_a_time() {
    assert_eq!(PAGE_SIZE, 50);
}

#[test]
fn merging_a_page_keeps_the_first_copy_of_a_repeated_song() {
    let mut cached = vec![song(1, "One", "A", "X")];
    let page = vec![song(1, "One again", "A", "X"), song(2, "Two", "B", "Y")];

    let added = merge_songs(&mut cached, page);

    assert_eq!(added, 1);
    assert_eq!(cached.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(cached.first().map(|s| s.name.as_str()), Some("One"));
}

#[test]
fn merging_a_page_preserves_the_servers_order() {
    let mut cached = Vec::new();
    let page = vec![song(7, "First", "A", "X"), song(3, "Second", "B", "Y")];

    let added = merge_songs(&mut cached, page);

    assert_eq!(added, 2);
    assert_eq!(cached.iter().map(|s| s.id).collect::<Vec<_>>(), vec![7, 3]);
}

#[test]
fn a_song_matches_on_its_title_artist_or_album() {
    let entry = song(1, "Hyperballad", "Björk", "Post");

    assert!(song_matches(&entry, &fold_needle("hyper")));
    assert!(song_matches(&entry, &fold_needle("bjork")));
    assert!(song_matches(&entry, &fold_needle("POST")));
    assert!(!song_matches(&entry, &fold_needle("homogenic")));
}

#[test]
fn an_unfiltered_list_matches_every_song() {
    let entry = song(1, "Hyperballad", "Björk", "Post");

    assert!(song_matches(&entry, &fold_needle("")));
}

#[test]
fn filtered_ids_follow_the_order_the_server_returned() {
    let state = seeded(
        4,
        vec![
            song(9, "Post", "Björk", "Post"),
            song(2, "Army of Me", "Björk", "Post"),
            song(5, "Other", "Someone", "Elsewhere"),
        ],
        120,
    );

    let ids = state.song_ids_matching(&fold_needle("björk"));

    assert_eq!(ids, vec![9, 2]);
    assert_eq!(state.cached_total(), 120);
}

#[test]
fn a_cache_only_serves_the_account_generation_that_filled_it() {
    let state = seeded(4, Vec::new(), 0);

    assert!(state.has_cached_generation(4));
    assert!(!state.has_cached_generation(5));
}

#[test]
fn retry_asks_for_the_offset_that_failed_before_anything_else() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);

    state.record_failed_offset(50);

    assert_eq!(state.retry_offset(4), 50);
}

#[test]
fn retry_falls_back_to_the_cached_length_then_to_the_first_page() {
    let state = seeded(4, vec![song(1, "One", "A", "X"), song(2, "Two", "B", "Y")], 90);

    assert_eq!(state.retry_offset(4), 2);
    assert_eq!(state.retry_offset(5), 0);
}

#[test]
fn clearing_a_failed_offset_lets_retry_resume_from_the_cache() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);
    state.record_failed_offset(50);

    state.clear_failed_offset();

    assert_eq!(state.retry_offset(4), 1);
}

#[test]
fn an_account_clear_drops_the_cache_and_revisions_inflight_work() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);
    state.record_failed_offset(50);
    state.loading.store(true, Ordering::Release);
    let request = state.revision.load(Ordering::Relaxed);

    state.clear();

    assert!(state.cache.lock().is_none());
    assert_ne!(state.revision.load(Ordering::Relaxed), request);
    assert!(!state.loading.load(Ordering::Acquire));
    assert_eq!(state.retry_offset(4), 0);
}

#[test]
fn leaving_keeps_the_cache_but_revisions_inflight_work() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);
    state.loading.store(true, Ordering::Release);
    let request = state.revision.load(Ordering::Relaxed);

    state.invalidate_for_leave();

    assert!(state.cache.lock().is_some(), "leave keeps the cache for an immediate repaint");
    assert_ne!(state.revision.load(Ordering::Relaxed), request);
    assert!(!state.loading.load(Ordering::Acquire));
}

fn matched_state(labels: HashMap<i64, String>, songs: usize) -> DiskMatches {
    DiskMatches { labels, songs }
}

fn named(artist: Option<String>) -> NamedMatch {
    NamedMatch { cloud_song_id: 1, local_track_id: 2, title: "Hyperballad".to_owned(), artist }
}

#[test]
fn a_matched_copy_is_named_by_its_title_and_artist() {
    assert_eq!(label_for(&named(Some("Björk".to_owned()))), "Hyperballad · Björk");
}

#[test]
fn a_match_with_no_artist_tag_is_named_by_its_title_alone() {
    assert_eq!(label_for(&named(None)), "Hyperballad");
    assert_eq!(label_for(&named(Some(String::new()))), "Hyperballad");
}

#[test]
fn a_mapped_page_counts_its_matches_against_the_page_it_covered() {
    let matches = matched_state(HashMap::from([(1, "One".to_owned()), (2, "Two".to_owned())]), 3);

    assert_eq!(matched_summary(Some(&matches)), Some((2, 3)));
}

#[test]
fn an_unasked_page_states_no_summary() {
    assert_eq!(matched_summary(None), None);
}

#[test]
fn an_account_clear_drops_the_matched_page() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);
    *state.matches.lock() = Some(matched_state(HashMap::from([(1, "One".to_owned())]), 1));
    state.matching.store(true, Ordering::Release);
    let request = state.match_request.load(Ordering::Relaxed);

    state.clear();

    assert!(state.matches.lock().is_none());
    assert!(!state.matching.load(Ordering::Acquire));
    assert_ne!(state.match_request.load(Ordering::Relaxed), request);
}

#[test]
fn leaving_keeps_the_matched_page_but_clears_a_running_match() {
    let state = seeded(4, vec![song(1, "One", "A", "X")], 90);
    *state.matches.lock() = Some(matched_state(HashMap::from([(1, "One".to_owned())]), 1));
    state.matching.store(true, Ordering::Release);

    state.invalidate_for_leave();

    assert!(
        state.matches.lock().is_some(),
        "the kept cache still has a mapping, so re-entering repaints the chips without a second run"
    );
    assert!(!state.matching.load(Ordering::Acquire));
}

#[test]
fn the_disk_tab_offers_the_mapping_and_states_its_result() {
    assert!(DISK_TAB.contains("clicked => { Favorites.match-cloud-disk(); }"));
    assert!(DISK_TAB.contains("enabled: !Favorites.cloud-disk-matching"));
    assert!(DISK_TAB.contains(
        "@tr(\"{} of {} matched\", Favorites.cloud-disk-matched-count, Favorites.cloud-disk-match-total)"
    ));
}
