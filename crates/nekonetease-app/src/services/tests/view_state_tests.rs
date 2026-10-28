use std::collections::HashMap;

use tempfile::TempDir;

use crate::services::settings::SortDir;
use nekonetease_core::error::AppError;

use super::*;

fn json_err(e: &serde_json::Error) -> AppError {
    AppError::Validation(format!("json error: {e}"))
}

#[test]
fn test_view_state_default() {
    let vs = ViewStateData::default();
    assert_eq!(vs.last_nav_index, 9);
    assert!(vs.view_columns.is_empty());
    assert!(vs.view_column_widths.is_empty());
    assert!(vs.view_sort.is_empty());
    assert!(vs.browse_path.is_none());
    assert_eq!(vs.browse_view_mode, 0);
    assert!(vs.last_detail_ids.is_empty());
    assert!(!vs.artist_albums_collapsed);
    assert_eq!(vs.settings_tab, 3);
    assert_eq!(vs.favorites_tab, 0);
    assert_eq!(vs.recently_played_tab, 0);
    assert_eq!(vs.my_library_tab, 0);
}

#[test]
fn test_view_state_missing_fields_default() -> Result<(), AppError> {
    // An empty object must deserialize to all-defaults — mirrors a fresh
    // `views.json` or a partial file written by an older client.
    let vs: ViewStateData = serde_json::from_str("{}").map_err(|e| json_err(&e))?;
    assert_eq!(vs.last_nav_index, 9);
    assert!(vs.view_sort.is_empty());
    // A missing Settings tab defaults to Services; a missing Browse presentation
    // lands on the list rather than failing the whole file.
    assert_eq!(vs.browse_view_mode, 0);
    assert_eq!(vs.settings_tab, 3);
    assert_eq!(vs.favorites_tab, 0);
    assert_eq!(vs.recently_played_tab, 0);
    assert_eq!(vs.my_library_tab, 0);
    // Zero is what makes this an *old* file to `read_view_state`, so a legacy category still
    // folds. A fresh-file default here would silently skip the migration.
    assert_eq!(vs.view_state_version, 0);
    Ok(())
}

/// A `views.json` from before the Favorites page was tabbed still carries the
/// two collapse flags the strips used. Serde ignores unknown keys by default,
/// so an installed client must upgrade in place and land on the first tab
/// rather than refusing to load its whole view state.
#[test]
fn test_view_state_ignores_the_retired_collapse_flags() -> Result<(), AppError> {
    let legacy = r#"{
        "last_nav_index": 2,
        "favorites_artists_collapsed": true,
        "favorites_most_played_collapsed": true
    }"#;
    let vs: ViewStateData = serde_json::from_str(legacy).map_err(|e| json_err(&e))?;
    assert_eq!(vs.last_nav_index, 2);
    assert_eq!(vs.favorites_tab, 0);
    Ok(())
}

#[test]
fn test_view_state_roundtrip() -> Result<(), AppError> {
    let vs = ViewStateData {
        browse_path: Some("/music/rock".to_owned()),
        browse_view_mode: 1,
        last_nav_index: 5,
        artist_albums_collapsed: true,
        last_detail_ids: HashMap::from([("album_detail".to_owned(), 42)]),
        settings_tab: 3,
        recently_played_tab: 1,
        my_library_tab: 2,
        ..ViewStateData::default()
    };
    let json = serde_json::to_string(&vs).map_err(|e| json_err(&e))?;
    let back: ViewStateData = serde_json::from_str(&json).map_err(|e| json_err(&e))?;
    assert_eq!(back.browse_path.as_deref(), Some("/music/rock"));
    assert_eq!(back.browse_view_mode, 1);
    assert_eq!(back.last_nav_index, 5);
    assert!(back.artist_albums_collapsed);
    assert_eq!(back.last_detail_ids.get("album_detail").copied(), Some(42));
    assert_eq!(back.settings_tab, 3);
    assert_eq!(back.recently_played_tab, 1);
    assert_eq!(back.my_library_tab, 2);
    Ok(())
}

#[test]
fn test_view_sort_in_view_state() -> Result<(), AppError> {
    let json = r#"{"view_sort": {"tracks": {"field": "title", "dir": "desc"}, "albums": {"field": "year", "dir": "asc"}}}"#;
    let vs: ViewStateData = serde_json::from_str(json).map_err(|e| json_err(&e))?;
    assert_eq!(vs.view_sort.len(), 2);
    let tracks_sort = vs
        .view_sort
        .get("tracks")
        .ok_or_else(|| AppError::Validation("missing tracks sort".into()))?;
    assert_eq!(tracks_sort.field, "title");
    assert!(matches!(tracks_sort.dir, SortDir::Desc));
    let albums_sort = vs
        .view_sort
        .get("albums")
        .ok_or_else(|| AppError::Validation("missing albums sort".into()))?;
    assert_eq!(albums_sort.field, "year");
    assert!(matches!(albums_sort.dir, SortDir::Asc));
    Ok(())
}

/// The last supported section index survives a `views.json` round trip.
#[test]
fn the_top_of_the_range_survives_a_views_json_round_trip() -> Result<(), AppError> {
    let vs = ViewStateData { last_nav_index: MAX_NAV_INDEX, ..ViewStateData::default() };
    let json = serde_json::to_string(&vs).map_err(|e| json_err(&e))?;
    let back: ViewStateData = serde_json::from_str(&json).map_err(|e| json_err(&e))?;
    assert_eq!(back.last_nav_index, MAX_NAV_INDEX);
    Ok(())
}

/// Recommendations sat at 2 and Roaming at 3; the new scheme drops Recommendations and moves
/// Roaming down to 2. The fold is a one-time read-side event, so the mapping is the contract.
#[test]
fn the_retired_category_fold_maps_old_indices_onto_the_new_scheme() {
    for index in 0..=4 {
        let expected = match index {
            2 => 0,
            3 => 2,
            other => other,
        };
        assert_eq!(fold_retired_music_library_category(index), expected);
    }
}

/// The fold must not run twice: under the new scheme 2 is Roaming, so a stamped file keeps it.
/// Without the stamp a Roaming pick would land on Collected one launch later.
#[test]
fn an_old_views_file_is_folded_once_and_stamped() -> Result<(), AppError> {
    let tmp = TempDir::new().map_err(AppError::io_source)?;
    let paths = Paths::rooted_at(tmp.path().to_path_buf());
    std::fs::write(&paths.view_state_path, r#"{"music_library_category": 3}"#)
        .map_err(AppError::io_source)?;

    let first = read_view_state(&paths)?;
    assert_eq!(first.music_library_category, 2);
    assert_eq!(first.view_state_version, VIEW_STATE_VERSION);

    let second = read_view_state(&paths)?;
    assert_eq!(second.music_library_category, 2);
    Ok(())
}
