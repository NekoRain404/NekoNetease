use std::collections::HashMap;

use super::{conflict_rows, count, policy_of, policy_token};
use nekonetease_app::library::cloud_playlist_sync::{
    CloudSide, LocalSide, Snapshot, SyncPlan, SyncPolicy, plan_sync,
};
use nekonetease_ui::SyncConflictKind;

const TAB_PILLS: &str =
    include_str!("../../../../../../nekonetease-ui/ui/views/my-library/tab-pills.slint");
const DETAIL_VIEW: &str =
    include_str!("../../../../../../nekonetease-ui/ui/views/my-library/playlist-detail.slint");
const REVIEW_PANEL: &str =
    include_str!("../../../../../../nekonetease-ui/ui/views/my-library/sync-review-panel.slint");
const PLAYLISTS_GLOBAL: &str =
    include_str!("../../../../../../nekonetease-ui/ui/globals/playlists.slint");
const TRACK_ROW: &str =
    include_str!("../../../../../../nekonetease-ui/ui/components/track-list/track-list-row.slint");

/// Both sides moved away from the last applied snapshot, and the cloud dropped 13 while the
/// local playlist dropped 14 — so a local-wins plan reports one conflict per direction plus the
/// rename.
fn diverged(policy: SyncPolicy) -> SyncPlan {
    let cloud = CloudSide { playlist_id: 7, name: "Cloud", track_ids: &[11, 13], unmatched: 2 };
    let local = LocalSide { id: 5, name: "Local", track_ids: &[11, 14] };
    let snapshot =
        Snapshot { cloud_name: "OldCloud", local_name: "OldLocal", track_ids: &[13, 14] };
    plan_sync(&cloud, Some(&local), Some(&snapshot), policy)
}

#[test]
fn conflict_rows_keep_the_plans_order_and_name_the_local_track() {
    let plan = diverged(SyncPolicy::LocalWins);
    let titles = HashMap::from([(13, "Thirteen".to_owned()), (14, "Fourteen".to_owned())]);

    let rows = conflict_rows(&plan, &titles);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].kind, SyncConflictKind::LocalRemovalKept);
    assert_eq!(rows[0].label.as_str(), "Thirteen");
    assert_eq!(rows[1].kind, SyncConflictKind::CloudRemovalKept);
    assert_eq!(rows[1].label.as_str(), "Fourteen");
    assert_eq!(rows[2].kind, SyncConflictKind::NameKept);
    assert_eq!(rows[2].label.as_str(), "");
}

#[test]
fn a_conflict_the_local_library_cannot_name_prints_no_label() {
    let plan = diverged(SyncPolicy::LocalWins);

    let rows = conflict_rows(&plan, &HashMap::new());

    assert_eq!(rows[0].label.as_str(), "");
    assert_eq!(rows[1].label.as_str(), "");
}

#[test]
fn cloud_wins_settles_both_directions_and_takes_the_cloud_name() {
    let plan = diverged(SyncPolicy::CloudWins);

    assert_eq!(plan.added(), &[13]);
    assert_eq!(plan.removed(), &[14]);
    assert_eq!(plan.rename_to(), Some("Cloud"));
    assert_eq!(plan.conflicts(), []);
}

#[test]
fn the_policy_tokens_round_trip_and_refuse_anything_else() {
    assert_eq!(policy_token(SyncPolicy::LocalWins), "local");
    assert_eq!(policy_token(SyncPolicy::CloudWins), "cloud");
    assert_eq!(policy_of("local"), Some(SyncPolicy::LocalWins));
    assert_eq!(policy_of("cloud"), Some(SyncPolicy::CloudWins));
    assert_eq!(policy_of(""), None);
    assert_eq!(policy_of("Cloud"), None);
}

#[test]
fn a_count_never_wraps_an_int() {
    assert_eq!(count(0), 0);
    assert_eq!(count(3), 3);
    assert_eq!(count(usize::MAX), i32::MAX);
}

#[test]
fn the_owned_cloud_pill_opens_the_review_and_nothing_else_does() {
    assert!(
        TAB_PILLS.contains(
            "if PlaylistDetail.cloud-collection && PlaylistDetail.cloud-owned: PillButton"
        )
    );
    assert!(TAB_PILLS.contains("clicked => { PlaylistDetail.request-sync-review(); }"));
}

#[test]
fn the_detail_mounts_the_panel_on_its_own_flag() {
    assert!(DETAIL_VIEW.contains("if PlaylistDetail.sync-open"));
    assert!(DETAIL_VIEW.contains("SyncReviewPanel"));
}

#[test]
fn the_panel_applies_the_plan_it_reviewed() {
    assert!(REVIEW_PANEL.contains("enabled: PlaylistDetail.sync-can-apply"));
    assert!(REVIEW_PANEL.contains("clicked => { PlaylistDetail.apply-sync(); }"));
    assert!(REVIEW_PANEL.contains("PlaylistDetail.set-sync-policy(\"local\")"));
    assert!(REVIEW_PANEL.contains("PlaylistDetail.set-sync-policy(\"cloud\")"));
}

#[test]
fn the_global_declares_the_whole_review_surface() {
    for name in [
        "sync-open",
        "sync-loading",
        "sync-applying",
        "sync-error",
        "sync-policy",
        "sync-can-apply",
        "sync-no-changes",
        "sync-creates-playlist",
        "sync-cloud-tracks",
        "sync-added",
        "sync-removed",
        "sync-unmatched",
        "sync-rename-to",
        "sync-conflicts",
        "request-sync-review",
        "set-sync-policy",
        "apply-sync",
        "close-sync",
    ] {
        assert!(PLAYLISTS_GLOBAL.contains(name), "{name} is missing from the global");
    }
}

#[test]
fn a_cloud_disk_row_names_the_local_copy_it_matched() {
    assert!(TRACK_ROW.contains("@tr(\"Local copy: {}\", root.row-data.local_match)"));
}
