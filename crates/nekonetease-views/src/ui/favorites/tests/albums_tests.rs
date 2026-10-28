use super::{AlbumsState, album_matches, album_row, artwork_paths};
use crate::ui::row_match::fold_needle;
use nekonetease_core::entities::cloud::{Album, Artist};
use std::sync::atomic::Ordering;

#[test]
fn album_filter_matches_title_and_each_artist_with_case_and_accent_folding() {
    let album = Album {
        id: 4_000_000_000,
        name: "Début".into(),
        artists: vec![
            Artist { name: "Björk".into(), ..Artist::default() },
            Artist { name: "Guest Artist".into(), ..Artist::default() },
        ],
        ..Album::default()
    };
    for query in ["", "DEBUT", "bjork", "GUEST"] {
        assert!(album_matches(&album, &fold_needle(query)), "query: {query}");
    }
    assert!(!album_matches(&album, &fold_needle("unrelated")));
}

#[test]
fn clearing_account_data_invalidates_inflight_fetch_and_pending_membership() {
    let state = AlbumsState::default();
    *state.cache.lock() = Some((7, vec![Album { id: 4_000_000_000, ..Album::default() }]));
    state.pending.lock().insert(4_000_000_000);
    let request = state.revision.load(Ordering::Relaxed);
    state.clear();
    assert!(state.cache.lock().is_none());
    assert!(state.pending.lock().is_empty());
    assert_ne!(state.revision.load(Ordering::Relaxed), request);
}

#[test]
fn album_cards_keep_exact_ids_and_only_remote_artwork() {
    let mut album = Album {
        id: 4_000_000_001,
        name: "Album".into(),
        cover_url: Some("https://p1.music.126.net/cover.jpg".into()),
        artists: vec![Artist { name: "Artist".into(), ..Artist::default() }],
    };
    let row = album_row(&album, true);
    assert_eq!(row.remote_id.as_str(), "4000000001");
    assert_eq!(row.artist.as_str(), "Artist");
    assert_eq!(row.artwork_path.as_str(), "https://p1.music.126.net/cover.jpg");
    assert!(row.pending);
    album.cover_url = Some("file:///private/cover.jpg".into());
    assert!(album_row(&album, false).artwork_path.is_empty());
}

#[test]
fn album_banner_uses_four_distinct_remote_covers_in_collection_order() {
    let albums = ["a", "a", "b", "c", "d", "e"].map(|name| Album {
        cover_url: Some(format!("https://p1.music.126.net/{name}.jpg")),
        ..Album::default()
    });
    assert_eq!(
        artwork_paths(&albums),
        ["a", "b", "c", "d"].map(|name| format!("https://p1.music.126.net/{name}.jpg"))
    );
}
