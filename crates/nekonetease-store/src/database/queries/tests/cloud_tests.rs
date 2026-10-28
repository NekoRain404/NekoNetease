//! The two cloud bridges at the SQL boundary: the match projection, and a sync
//! apply that has to leave the playlist, its thumbnail and the snapshot coherent
//! in one transaction.

use crate::database::queries;
use crate::database::queries::cloud::CloudPlaylistSync;
use crate::database::queries::fixtures::setup_seeded_db;
use nekonetease_core::error::AppError;

fn sync<'a>(cloud: i64, local: i64) -> CloudPlaylistSync<'a> {
    CloudPlaylistSync {
        cloud_playlist_id: cloud,
        local_playlist_id: local,
        rename_to: None,
        remove_track_ids: &[],
        add_track_ids: &[],
        cloud_name: "Shared",
        local_name: "Shared",
        cloud_track_ids: &[],
    }
}

#[tokio::test]
async fn match_candidates_returns_only_the_columns_a_matcher_compares() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;

    let mut rows = queries::cloud::match_candidates(&db).await?;
    rows.sort_by_key(|row| row.id);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].title, "Alpha Song");
    assert_eq!(rows[0].artist.as_deref(), Some("Artist A"));
    assert_eq!(rows[0].duration_ms, 180_000);
    Ok(())
}

#[tokio::test]
async fn playlist_track_ids_follow_playlist_order_rather_than_insert_order() -> Result<(), AppError>
{
    let db = setup_seeded_db().await?;
    let playlist =
        queries::playlist::create_playlist_with_tracks(&db, "Ordered", None, &[3, 1, 2]).await?;

    let ids = queries::cloud::playlist_track_ids(&db, playlist).await?;

    assert_eq!(ids, vec![3, 1, 2]);
    Ok(())
}

#[tokio::test]
async fn apply_renames_removes_and_adds_in_one_pass() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;
    let local =
        queries::playlist::create_playlist_with_tracks(&db, "Shared", None, &[1, 2]).await?;

    queries::cloud::apply_cloud_playlist_sync(
        &db,
        &CloudPlaylistSync {
            rename_to: Some("Cloud Name"),
            remove_track_ids: &[2],
            add_track_ids: &[3],
            cloud_name: "Cloud Name",
            local_name: "Cloud Name",
            cloud_track_ids: &[1, 3],
            ..sync(9_001, local)
        },
    )
    .await?;

    let playlist = queries::playlist::get_playlist_by_id(&db, local).await?;
    assert_eq!(playlist.name, "Cloud Name");
    assert_eq!(queries::cloud::playlist_track_ids(&db, local).await?, vec![1, 3]);
    let link = queries::cloud::playlist_link(&db, 9_001)
        .await?
        .ok_or_else(|| AppError::not_found("playlist link", 9_001))?;
    assert_eq!(link.local_playlist_id, local);
    assert_eq!(link.cloud_name, "Cloud Name");
    assert_eq!(link.local_name, "Cloud Name");
    assert_eq!(queries::cloud::playlist_link_track_ids(&db, 9_001).await?, vec![1, 3]);
    Ok(())
}

#[tokio::test]
async fn an_empty_apply_still_records_the_snapshot() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;
    let local = queries::playlist::create_playlist(&db, "Shared", None).await?;

    let mut request = sync(9_002, local.id);
    request.cloud_track_ids = &[1, 3];
    queries::cloud::apply_cloud_playlist_sync(&db, &request).await?;

    assert_eq!(queries::cloud::playlist_link_track_ids(&db, 9_002).await?, vec![1, 3]);
    Ok(())
}

#[tokio::test]
async fn a_smart_playlist_refuses_a_membership_write() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;
    let smart = queries::playlist::create_smart_playlist(&db, "Smart", None, "{}").await?;

    let mut request = sync(9_003, smart.id);
    request.add_track_ids = &[1];
    request.cloud_track_ids = &[1];
    let refused = queries::cloud::apply_cloud_playlist_sync(&db, &request).await;

    assert!(matches!(refused, Err(AppError::Validation(_))));
    assert_eq!(queries::cloud::playlist_track_ids(&db, smart.id).await?, Vec::<i64>::new());
    Ok(())
}

#[tokio::test]
async fn a_playlist_paired_with_one_cloud_side_refuses_a_second() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;
    let local = queries::playlist::create_playlist(&db, "Shared", None).await?;
    queries::cloud::apply_cloud_playlist_sync(&db, &sync(9_004, local.id)).await?;

    let mut request = sync(9_005, local.id);
    request.cloud_name = "Other";
    request.add_track_ids = &[1];
    request.cloud_track_ids = &[1];
    let refused = queries::cloud::apply_cloud_playlist_sync(&db, &request).await;

    assert!(matches!(refused, Err(AppError::Validation(_))));
    Ok(())
}

#[tokio::test]
async fn an_apply_into_a_missing_local_playlist_is_not_found() -> Result<(), AppError> {
    let db = setup_seeded_db().await?;

    let refused = queries::cloud::apply_cloud_playlist_sync(&db, &sync(9_006, 404)).await;

    assert!(matches!(refused, Err(AppError::NotFound(_))));
    Ok(())
}
