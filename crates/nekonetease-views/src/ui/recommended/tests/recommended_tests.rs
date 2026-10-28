use slint::Model;

use super::{chunk, playlist_rows};
use nekonetease_core::entities::cloud::Playlist;

fn playlist(id: i64, name: &str, tags: &[&str], cover: Option<&str>) -> Playlist {
    Playlist {
        id,
        name: name.to_owned(),
        cover_url: cover.map(str::to_owned),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        ..Playlist::default()
    }
}

#[test]
fn a_card_carries_the_exact_remote_id_as_text() {
    let id = i64::from(i32::MAX) + 7;
    let rows = playlist_rows(&[playlist(id, "Late night", &[], None)]);

    assert_eq!(rows.len(), 1);
    let expected = id.to_string();
    assert_eq!(rows[0].remote_id.as_str(), expected.as_str());
    assert_eq!(rows[0].id, 0, "the card's int id is the row index, not the 64-bit wire id");
}

#[test]
fn cards_stay_in_the_service_order() {
    let rows = playlist_rows(&[
        playlist(10, "One", &[], None),
        playlist(20, "Two", &[], None),
        playlist(30, "Three", &[], None),
    ]);

    let titles: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(titles, ["One", "Two", "Three"]);
}

#[test]
fn a_card_carries_the_track_count_and_no_duration() {
    let mut source = playlist(1, "One", &[], None);
    source.track_count = 42;
    let rows = playlist_rows(&[source]);

    assert_eq!(rows[0].track_count, 42);
    // A cloud suggestion reports no duration; the grid drops the separator rather than
    // trailing a `0:00`.
    assert_eq!(rows[0].total_duration_ms, 0);
}

#[test]
fn a_card_keeps_only_a_remote_cover() {
    let rows = playlist_rows(&[playlist(1, "One", &[], Some("https://cdn.example/a.jpg"))]);
    assert_eq!(rows[0].artwork_path.as_str(), "https://cdn.example/a.jpg");

    let rows = playlist_rows(&[playlist(1, "One", &[], Some("/music/a.jpg"))]);
    assert_eq!(rows[0].artwork_path.as_str(), "");
}

#[test]
fn rows_chunk_into_the_page_column_count() {
    let rows = playlist_rows(&[
        playlist(1, "One", &[], None),
        playlist(2, "Two", &[], None),
        playlist(3, "Three", &[], None),
        playlist(4, "Four", &[], None),
        playlist(5, "Five", &[], None),
    ]);

    let grid = chunk(&rows, 2);
    let widths: Vec<usize> = grid.iter().map(|row| row.playlists.row_count()).collect();
    assert_eq!(widths, [2, 2, 1]);
}

/// The page is its own: it borrows the `Playlists.open-special` door to reach the two special
/// playlists, but mounts none of My Library's surfaces or its band.
#[test]
fn the_page_mounts_no_my_library_surface() {
    const SOURCES: [(&str, &str); 3] = [
        ("recommended.rs", include_str!("../mod.rs")),
        (
            "recommended-view.slint",
            include_str!(
                "../../../../../nekonetease-ui/ui/views/recommended/recommended-view.slint"
            ),
        ),
        (
            "recommended.slint",
            include_str!("../../../../../nekonetease-ui/ui/globals/recommended.slint"),
        ),
    ];
    for (name, source) in SOURCES {
        for needle in ["MyLibrary.", "MyLibraryPlaylistsTab", "RoamingTab", "LibraryTabBand"] {
            assert!(!source.contains(needle), "{name} borrows {needle}");
        }
    }
}
