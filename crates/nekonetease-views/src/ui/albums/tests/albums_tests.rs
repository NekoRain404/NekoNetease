use super::*;
use nekonetease_core::entities::cloud::Comment;
use slint::Model as _;

/// Minimal `AlbumStats` builder — only the fields the grid filter / sort
/// read matter.
fn album(id: i64, name: &str, artist: &str, year: Option<i32>) -> AlbumStats {
    AlbumStats {
        id,
        name: name.to_string(),
        sort_name: None,
        artist_id: 1,
        artist_name: artist.to_string(),
        year,
        disc_count: None,
        is_compilation: false,
        musicbrainz_release_group_id: None,
        label: None,
        catalog_number: None,
        barcode: None,
        media: None,
        release_type: None,
        release_country: None,
        musicbrainz_id: None,
        artwork_path: None,
        track_count: 1,
        total_duration_ms: 0,
    }
}

fn names(data: &GridData, indices: &[usize]) -> Vec<String> {
    indices.iter().map(|&i| data.albums[i].name.clone()).collect()
}

#[test]
fn grid_data_precomputes_lowercased_sort_keys() {
    let data = GridData::new(vec![album(1, "AbBey Road", "The BEATLES", None)]);
    assert_eq!(data.keys.len(), 1);
    assert_eq!(data.keys[0].name_lc, "abbey road");
    assert_eq!(data.keys[0].artist_lc, "the beatles");
}

#[test]
fn compute_indices_with_empty_filter_keeps_all_albums() {
    let data = GridData::new(vec![album(1, "Alpha", "X", None), album(2, "Bravo", "Y", None)]);
    let idx = compute_indices(&data, "name", "asc", "");
    assert_eq!(names(&data, &idx), ["Alpha", "Bravo"]);
}

#[test]
fn compute_indices_filter_matches_name_or_artist_case_insensitively() {
    let data = GridData::new(vec![
        album(1, "Kind of Blue", "Miles Davis", None),
        album(2, "Blue Train", "John Coltrane", None),
        album(3, "Giant Steps", "John Coltrane", None),
    ]);
    // Album-name substring, case-insensitive.
    let by_name = compute_indices(&data, "name", "asc", "BLUE");
    assert_eq!(names(&data, &by_name), ["Blue Train", "Kind of Blue"]);
    // Artist-name substring.
    let by_artist = compute_indices(&data, "name", "asc", "coltrane");
    assert_eq!(names(&data, &by_artist), ["Blue Train", "Giant Steps"]);
}

#[test]
fn compute_indices_filter_matches_year_and_its_decade() {
    // `AlbumStats` carries the year, so a query the Search view answers
    // with a decade narrows this grid too.
    let data = GridData::new(vec![
        album(1, "Loveless", "My Bloody Valentine", Some(1991)),
        album(2, "Nevermind", "Nirvana", Some(1991)),
        album(3, "Kid A", "Radiohead", Some(2000)),
    ]);
    let by_year = compute_indices(&data, "name", "asc", "1991");
    assert_eq!(names(&data, &by_year), ["Loveless", "Nevermind"]);
    let by_decade = compute_indices(&data, "name", "asc", "199");
    assert_eq!(names(&data, &by_decade), ["Loveless", "Nevermind"]);
}

#[test]
fn compute_indices_filter_ignores_accents_the_way_the_search_view_does() {
    let data = GridData::new(vec![
        album(1, "Ágætis byrjun", "Sigur Rós", None),
        album(2, "Kid A", "Radiohead", None),
    ]);
    let by_name = compute_indices(&data, "name", "asc", "agaetis");
    assert!(by_name.is_empty(), "æ is a letter, not an accented a");
    let by_title = compute_indices(&data, "name", "asc", "byrjun");
    assert_eq!(names(&data, &by_title), ["Ágætis byrjun"]);
    let by_artist = compute_indices(&data, "name", "asc", "sigur ros");
    assert_eq!(names(&data, &by_artist), ["Ágætis byrjun"]);
}

#[test]
fn compute_indices_year_sort_breaks_ties_by_name_and_honours_dir() {
    let data = GridData::new(vec![
        album(1, "Later", "X", Some(2001)),
        album(2, "Earlier", "X", Some(1999)),
        album(3, "AlsoEarlier", "X", Some(1999)),
    ]);
    let asc = compute_indices(&data, "year", "asc", "");
    assert_eq!(names(&data, &asc), ["AlsoEarlier", "Earlier", "Later"]);
    let desc = compute_indices(&data, "year", "desc", "");
    assert_eq!(names(&data, &desc), ["Later", "Earlier", "AlsoEarlier"]);
}

#[test]
fn grid_index_cache_matches_only_identical_filter_and_sort() {
    let c = GridIndexCache {
        filter: "x".into(),
        sort_field: "name".into(),
        sort_dir: "asc".into(),
        indices: vec![],
    };
    assert!(c.matches("x", "name", "asc"));
    assert!(!c.matches("y", "name", "asc"));
    assert!(!c.matches("x", "year", "asc"));
    assert!(!c.matches("x", "name", "desc"));
}

#[test]
fn comment_preview_caps_the_band_at_four() {
    let comments: Vec<Comment> = (0..6)
        .map(|i| Comment {
            id: i64::from(i) + 1,
            author: format!("author-{i}"),
            content: format!("content-{i}"),
            time_ms: i64::from(i),
            ..Comment::default()
        })
        .collect();
    let model = crate::ui::detail_comments::comment_preview_model(&comments);
    assert_eq!(model.row_count(), 4);
}

#[test]
fn comment_preview_projects_the_like_state_the_heart_draws() {
    let comments = vec![
        Comment {
            id: 7,
            author: "author".into(),
            content: "content".into(),
            time_ms: 1,
            liked: true,
            liked_count: 12,
        },
        Comment {
            id: 8,
            author: "other".into(),
            content: "other".into(),
            time_ms: 2,
            liked: false,
            liked_count: 0,
        },
    ];
    let model = crate::ui::detail_comments::comment_preview_model(&comments);
    let liked = model.row_data(0);
    let plain = model.row_data(1);
    assert!(matches!(&liked, Some(row) if row.id == 7 && row.liked && row.liked_count == 12));
    assert!(matches!(&plain, Some(row) if row.id == 8 && !row.liked && row.liked_count == 0));
    assert_eq!(liked.as_ref().map(|row| row.remote_id.as_str()), Some("7"));
    assert_eq!(plain.as_ref().map(|row| row.remote_id.as_str()), Some("8"));
}

#[test]
fn comment_wire_id_keeps_the_exact_id_past_the_slint_int_boundary() {
    let model = crate::ui::detail_comments::comment_preview_model(&[Comment {
        id: 4_294_967_301,
        author: "author".into(),
        content: "content".into(),
        ..Comment::default()
    }]);
    let row = model.row_data(0);
    assert_eq!(row.as_ref().map(|row| row.id), Some(i32::MAX));
    assert_eq!(row.as_ref().map(|row| row.remote_id.as_str()), Some("4294967301"));
    assert_eq!(
        crate::ui::detail_comments::comment_wire_id(&model, 0, i32::MAX),
        Some(4_294_967_301)
    );
    assert!(crate::ui::detail_comments::comment_wire_id(&model, 0, 0).is_none());
    assert!(crate::ui::detail_comments::comment_wire_id(&model, 1, i32::MAX).is_none());
}

#[test]
fn comment_like_toggle_rolls_back_only_while_the_row_still_holds_the_optimistic_state() {
    let model = crate::ui::detail_comments::comment_preview_model(&[Comment {
        id: 42,
        author: "author".into(),
        content: "content".into(),
        liked: false,
        liked_count: 3,
        ..Comment::default()
    }]);
    let first = crate::ui::detail_comments::apply_like_toggle(&model, 0, 42, true);
    assert!(first.is_some(), "the matching row should be patched");
    let Some(first) = first else { return };
    let applied = model.row_data(0);
    assert!(applied.is_some());
    let Some(applied) = applied else { return };
    assert!(applied.liked);
    assert_eq!(applied.liked_count, 4);

    let second = crate::ui::detail_comments::apply_like_toggle(&model, 0, 42, false);
    assert!(second.is_some());
    let Some(second) = second else { return };
    crate::ui::detail_comments::rollback_like_toggle(&model, &first);
    let after_stale = model.row_data(0);
    assert!(after_stale.is_some());
    let Some(after_stale) = after_stale else { return };
    assert!(!after_stale.liked, "the later click owns the row, not the earlier failure");
    assert_eq!(after_stale.liked_count, 3);

    crate::ui::detail_comments::rollback_like_toggle(&model, &second);
    let restored = model.row_data(0);
    assert!(restored.is_some());
    let Some(restored) = restored else { return };
    assert!(restored.liked, "the second toggle rolls back onto the first one's result");
    assert_eq!(restored.liked_count, 4);
}

#[test]
fn comment_like_toggle_refuses_a_row_whose_id_moved() {
    let model = crate::ui::detail_comments::comment_preview_model(&[Comment {
        id: 42,
        author: "author".into(),
        content: "content".into(),
        ..Comment::default()
    }]);
    assert!(crate::ui::detail_comments::apply_like_toggle(&model, 0, 41, true).is_none());
    assert!(
        crate::ui::detail_comments::apply_like_toggle(&model, 1, 42, true).is_none(),
        "an index past the preview is stale"
    );
}
