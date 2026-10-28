//! The three-way diff's policy table, then the store round trip that proves a reviewed plan is the
//! only thing that can reach a local playlist.

use nekonetease_core::entities::cloud::{Artist, Song};
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries;
use nekonetease_store::database::queries::fixtures::insert_test_track;

use super::*;

fn cloud<'a>(name: &'a str, ids: &'a [i64]) -> CloudSide<'a> {
    CloudSide { playlist_id: 900, name, track_ids: ids, unmatched: 0 }
}

fn local<'a>(name: &'a str, ids: &'a [i64]) -> LocalSide<'a> {
    LocalSide { id: 7, name, track_ids: ids }
}

fn snapshot<'a>(cloud_name: &'a str, local_name: &'a str, ids: &'a [i64]) -> Snapshot<'a> {
    Snapshot { cloud_name, local_name, track_ids: ids }
}

#[test]
fn a_first_sync_adds_the_cloud_membership_and_keeps_local_extras() {
    let cloud = cloud("Cloud", &[1, 2]);
    let local = local("Local", &[2, 3]);

    let plan = plan_sync(&cloud, Some(&local), None, SyncPolicy::LocalWins);

    assert_eq!(plan.added(), &[1]);
    assert_eq!(plan.removed().len(), 0);
    assert_eq!(plan.conflicts().len(), 0);
    assert!(!plan.is_empty());
}

#[test]
fn a_first_link_is_a_write_even_when_the_membership_already_matches() {
    let cloud = cloud("Cloud", &[1]);
    let local = local("Local", &[1]);

    let plan = plan_sync(&cloud, Some(&local), None, SyncPolicy::LocalWins);

    assert_eq!(plan.added().len(), 0);
    assert_eq!(plan.removed().len(), 0);
    assert!(!plan.is_empty());
}

#[test]
fn a_first_sync_keeps_local_extras_even_when_the_cloud_wins() {
    let cloud = cloud("Cloud", &[1]);
    let local = local("Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), None, SyncPolicy::CloudWins);

    assert_eq!(plan.removed().len(), 0);
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn a_local_addition_is_not_read_as_a_cloud_removal() {
    let cloud = cloud("Cloud", &[1, 2]);
    let local = local("Local", &[1, 2, 3]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert!(plan.is_empty());
}

#[test]
fn a_cloud_addition_is_added() {
    let cloud = cloud("Cloud", &[1, 2, 3]);
    let local = local("Local", &[1, 2]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.added(), &[3]);
}

#[test]
fn local_wins_keeps_a_track_the_cloud_dropped() {
    let cloud = cloud("Cloud", &[1]);
    let local = local("Local", &[1, 2]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.removed().len(), 0);
    assert_eq!(plan.conflicts(), &[SyncConflict::CloudRemovalKept(2)]);
}

#[test]
fn cloud_wins_removes_a_track_the_cloud_dropped() {
    let cloud = cloud("Cloud", &[1]);
    let local = local("Local", &[1, 2]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::CloudWins);

    assert_eq!(plan.removed(), &[2]);
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn local_wins_does_not_re_add_a_local_removal() {
    let cloud = cloud("Cloud", &[1, 2]);
    let local = local("Local", &[1]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.added().len(), 0);
    assert_eq!(plan.conflicts(), &[SyncConflict::LocalRemovalKept(2)]);
}

#[test]
fn cloud_wins_re_adds_a_local_removal() {
    let cloud = cloud("Cloud", &[1, 2]);
    let local = local("Local", &[1]);
    let snapshot = snapshot("Cloud", "Local", &[1, 2]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::CloudWins);

    assert_eq!(plan.added(), &[2]);
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn a_cloud_rename_is_applied_when_the_local_name_stayed() {
    let cloud = cloud("New", &[1]);
    let local = local("Local", &[1]);
    let snapshot = snapshot("Old", "Local", &[1]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.rename_to(), Some("New"));
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn local_wins_keeps_a_name_that_moved_on_both_sides() {
    let cloud = cloud("Cloud New", &[1]);
    let local = local("Local New", &[1]);
    let snapshot = snapshot("Cloud Old", "Local Old", &[1]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.rename_to(), None);
    assert_eq!(plan.conflicts(), &[SyncConflict::NameKept]);
}

#[test]
fn cloud_wins_takes_the_cloud_name_when_both_moved() {
    let cloud = cloud("Cloud New", &[1]);
    let local = local("Local New", &[1]);
    let snapshot = snapshot("Cloud Old", "Local Old", &[1]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::CloudWins);

    assert_eq!(plan.rename_to(), Some("Cloud New"));
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn a_playlist_linked_under_its_own_name_is_not_locally_renamed() {
    let cloud = cloud("Cloud", &[1]);
    let local = local("Mine", &[1]);
    let snapshot = snapshot("Cloud", "Mine", &[1]);

    let plan = plan_sync(&cloud, Some(&local), Some(&snapshot), SyncPolicy::LocalWins);

    assert_eq!(plan.rename_to(), None);
    assert_eq!(plan.conflicts().len(), 0);
}

#[test]
fn a_first_sync_with_nothing_chosen_creates_the_playlist() {
    let cloud = cloud("Cloud", &[2, 1]);

    let plan = plan_sync(&cloud, None, None, SyncPolicy::LocalWins);

    assert!(plan.creates_playlist());
    assert!(!plan.is_empty());
    assert_eq!(plan.cloud_track_ids(), &[2, 1]);
}

fn song(id: i64, title: &str) -> Song {
    Song {
        id,
        name: title.to_owned(),
        artists: vec![Artist { name: "Artist".to_owned(), ..Artist::default() }],
        duration_ms: 180_000,
        ..Song::default()
    }
}

async fn seeded_playlist(db: &DbPool, name: &str, track_ids: &[i64]) -> Result<i64, AppError> {
    queries::playlist::create_playlist_with_tracks(db, name, None, track_ids).await
}

#[tokio::test]
async fn a_reviewed_plan_links_an_existing_playlist_and_its_snapshot_stops_reattribution()
-> Result<(), AppError> {
    let db = DbPool::test_pool().await?;
    let ids = [
        insert_test_track(&db, "/music/one.mp3", "One", "Artist", "Album", "Rock").await?,
        insert_test_track(&db, "/music/two.mp3", "Two", "Artist", "Album", "Rock").await?,
        insert_test_track(&db, "/music/three.mp3", "Three", "Artist", "Album", "Rock").await?,
    ];
    let local_id = seeded_playlist(&db, "Mine", &ids[..2]).await?;
    let songs = [song(1, "One"), song(2, "Two"), song(3, "Three")];
    let request = SyncRequest {
        cloud_playlist_id: 900,
        cloud_name: "Cloud",
        cloud_songs: &songs,
        chosen_local_playlist_id: Some(local_id),
        policy: SyncPolicy::LocalWins,
    };

    let reviewed = plan(&db, &request).await?;
    assert_eq!(reviewed.added(), &[ids[2]]);
    let report = apply(&db, &reviewed).await?;
    assert_eq!(report.local_playlist_id, local_id);
    assert_eq!(report.added, 1);

    assert_eq!(queries::cloud::playlist_track_ids(&db, local_id).await?, ids.to_vec());
    let link = queries::cloud::playlist_link(&db, 900)
        .await?
        .ok_or_else(|| AppError::not_found("playlist link", 900))?;
    assert_eq!(link.local_playlist_id, local_id);
    assert_eq!(link.local_name, "Mine");

    let again = plan(&db, &request).await?;
    assert!(again.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_plan_with_nothing_chosen_creates_a_new_playlist_on_apply() -> Result<(), AppError> {
    let db = DbPool::test_pool().await?;
    let track =
        insert_test_track(&db, "/music/solo.mp3", "Solo", "Artist", "Album", "Rock").await?;
    let songs = [song(1, "Solo")];
    let request = SyncRequest {
        cloud_playlist_id: 901,
        cloud_name: "Cloud",
        cloud_songs: &songs,
        chosen_local_playlist_id: None,
        policy: SyncPolicy::LocalWins,
    };

    let reviewed = plan(&db, &request).await?;
    assert!(reviewed.creates_playlist());
    let report = apply(&db, &reviewed).await?;

    let playlist = queries::playlist::get_playlist_by_id(&db, report.local_playlist_id).await?;
    assert_eq!(playlist.name, "Cloud");
    assert_eq!(
        queries::cloud::playlist_track_ids(&db, report.local_playlist_id).await?,
        vec![track]
    );
    assert!(queries::cloud::playlist_link(&db, 901).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_smart_playlist_is_refused_before_any_diff_is_shown() -> Result<(), AppError> {
    let db = DbPool::test_pool().await?;
    let smart = queries::playlist::create_smart_playlist(&db, "Smart", None, "{}").await?;
    let songs = [song(1, "Solo")];
    let request = SyncRequest {
        cloud_playlist_id: 902,
        cloud_name: "Cloud",
        cloud_songs: &songs,
        chosen_local_playlist_id: Some(smart.id),
        policy: SyncPolicy::LocalWins,
    };

    assert!(matches!(plan(&db, &request).await, Err(AppError::Validation(_))));
    Ok(())
}

#[tokio::test]
async fn a_cloud_playlist_already_paired_refuses_a_second_local_choice() -> Result<(), AppError> {
    let db = DbPool::test_pool().await?;
    let first = insert_test_track(&db, "/music/a.mp3", "A", "Artist", "Album", "Rock").await?;
    let second = insert_test_track(&db, "/music/b.mp3", "B", "Artist", "Album", "Rock").await?;
    let paired = seeded_playlist(&db, "Paired", &[first]).await?;
    let other = seeded_playlist(&db, "Other", &[second]).await?;
    let songs = [song(1, "A")];
    let request = SyncRequest {
        cloud_playlist_id: 903,
        cloud_name: "Cloud",
        cloud_songs: &songs,
        chosen_local_playlist_id: Some(paired),
        policy: SyncPolicy::LocalWins,
    };
    let reviewed = plan(&db, &request).await?;
    apply(&db, &reviewed).await?;

    let request = SyncRequest { chosen_local_playlist_id: Some(other), ..request };
    assert!(matches!(plan(&db, &request).await, Err(AppError::Validation(_))));
    Ok(())
}
