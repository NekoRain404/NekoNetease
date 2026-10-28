use super::{
    PAGE_SIZE, SavedArtistCache, SavedArtistsState, artist_row, merge_page, page_has_more,
    page_should_continue,
};
use nekonetease_core::entities::cloud::Artist;
use std::sync::atomic::Ordering;

const CLOUD_ARTISTS_TAB: &str = include_str!(
    "../../../../../nekonetease-ui/ui/views/favorites/cloud-favorites-artists-tab.slint"
);

#[test]
fn cloud_artist_page_cursor_tracks_server_rows_not_unique_projection_rows() {
    let mut cached = vec![Artist { id: 4_000_000_001, name: "One".into(), ..Artist::default() }];
    let page = vec![
        Artist { id: 4_000_000_001, name: "One".into(), ..Artist::default() },
        Artist { id: 4_000_000_002, name: "Two".into(), ..Artist::default() },
    ];
    let page_len = merge_page(&mut cached, page);

    assert_eq!(page_len, 2);
    assert_eq!(cached.len(), 2);
}

#[test]
fn only_full_net_ease_artist_pages_advertise_another_page() {
    assert!(page_has_more(PAGE_SIZE));
    assert!(!page_has_more(PAGE_SIZE - 1));
    assert!(!page_has_more(0));
}

#[test]
fn repeated_full_artist_pages_stop_pagination_after_deduplication() {
    assert!(page_should_continue(PAGE_SIZE, PAGE_SIZE));
    assert!(!page_should_continue(PAGE_SIZE, 0));
    assert!(!page_should_continue(PAGE_SIZE - 1, PAGE_SIZE - 1));
}

#[test]
fn account_invalidation_clears_cached_artists_and_revisions_inflight_work() {
    let state = SavedArtistsState::default();
    *state.cache.lock() = Some(SavedArtistCache {
        generation: 17,
        artists: vec![Artist { id: 4_000_000_001, ..Artist::default() }],
        next_offset: 30,
        has_more: true,
    });
    let request = state.revision.load(Ordering::Relaxed);

    state.clear();

    assert!(state.cache.lock().is_none());
    assert_ne!(state.revision.load(Ordering::Relaxed), request);
    assert!(!state.loading.load(Ordering::Acquire));
}

#[test]
fn leaving_cloud_artists_invalidates_inflight_work_and_reopens_the_loading_gate() {
    let state = SavedArtistsState::default();
    *state.cache.lock() = Some(SavedArtistCache {
        generation: 17,
        artists: vec![Artist { id: 4_000_000_001, ..Artist::default() }],
        next_offset: 30,
        has_more: true,
    });
    state.loading.store(true, Ordering::Release);
    let request = state.revision.load(Ordering::Relaxed);

    state.invalidate_for_leave();

    assert!(state.cache.lock().is_some(), "leave keeps cache for immediate repaint");
    assert_ne!(state.revision.load(Ordering::Relaxed), request);
    assert!(!state.loading.load(Ordering::Acquire));
}

#[test]
fn retry_reuses_the_offset_of_the_failed_request() {
    let state = SavedArtistsState::default();
    *state.cache.lock() = Some(SavedArtistCache {
        generation: 17,
        artists: vec![Artist { id: 4_000_000_001, ..Artist::default() }],
        next_offset: 60,
        has_more: true,
    });

    state.record_failed_offset(0);

    assert_eq!(state.retry_offset(17), 0);
}

#[test]
fn filtered_empty_state_waits_until_all_artist_pages_are_loaded() {
    assert!(CLOUD_ARTISTS_TAB.contains("Favorites.filtered-cloud-artist-count == 0"));
    assert!(CLOUD_ARTISTS_TAB.contains("!Favorites.cloud-artists-has-more"));
}

#[test]
fn cloud_artist_rows_keep_exact_remote_id_and_remote_artwork_only() {
    let artist = Artist {
        id: 4_000_000_001,
        name: "Artist".into(),
        cover_url: Some("https://p1.music.126.net/artist.jpg".into()),
        ..Artist::default()
    };
    let row = artist_row(&artist, true);
    assert_eq!(row.remote_id.as_str(), "4000000001");
    assert_eq!(row.title.as_str(), "Artist");
    assert_eq!(row.artwork_path.as_str(), "https://p1.music.126.net/artist.jpg");
    assert!(row.pending);

    let local = Artist { cover_url: Some("file:///tmp/artist.jpg".into()), ..artist };
    assert!(artist_row(&local, false).artwork_path.is_empty());
}
