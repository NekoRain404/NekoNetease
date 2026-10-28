//! Reconciles a cloud playlist into a local one, through a dry run the user sees first.
//!
//! **A local playlist is only ever written by applying a [`SyncPlan`]**, and a plan has private
//! fields and no public constructor: [`plan`] or [`plan_sync`] is the only way to obtain one.
//! That is what makes "never overwrite without showing the diff" a property of the types rather
//! than a rule a new call site has to remember — [`apply`] cannot be reached with a diff nobody
//! computed.
//!
//! The diff is three-way. The link table stores the cloud membership as of the last apply, which
//! is the base that separates a track the user added locally from one the cloud never carried; a
//! two-way comparison would read the first as a cloud removal and delete it. See the migration for
//! why the snapshot records the cloud side and why both names are kept.

use std::collections::{BTreeSet, HashMap};

use nekonetease_core::entities::cloud::Song;
use nekonetease_core::error::{AppError, AppResult};
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries;
use nekonetease_store::database::queries::cloud::CloudPlaylistSync;

use super::cloud_match;

/// Which side settles a disagreement. The default is the user's own edits: a local removal or
/// rename is a deliberate act on this machine, and a background sync overriding it would make the
/// playlist feel like it belongs to the account instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncPolicy {
    #[default]
    LocalWins,
    CloudWins,
}

/// One divergence the dry run reports but, under [`SyncPolicy::LocalWins`], does not change.
///
/// Informational rather than pending: re-reporting it on every pass is the honest answer, because
/// the cloud really does still carry the track the user removed. Applying the plan touches nothing
/// for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncConflict {
    /// The cloud still has a track the user removed locally; it stays removed.
    LocalRemovalKept(i64),
    /// The cloud dropped a track the user still has locally; it stays local.
    CloudRemovalKept(i64),
    /// Both sides renamed the playlist, differently; the local name stays.
    NameKept,
}

/// The reviewed diff. Constructed only by [`plan`] / [`plan_sync`], which is what [`apply`] relies
/// on for its "the user has seen this" contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPlan {
    cloud_playlist_id: i64,
    local_playlist_id: Option<i64>,
    create_name: Option<String>,
    rename_to: Option<String>,
    added: Vec<i64>,
    removed: Vec<i64>,
    conflicts: Vec<SyncConflict>,
    unmatched: usize,
    cloud_name: String,
    result_name: String,
    cloud_track_ids: Vec<i64>,
    /// True when applying writes link state the snapshot does not hold yet: a first link, or a
    /// name divergence this plan settles in the local playlist's favour. Without it `is_empty`
    /// would call such a plan a no-op and the link never land.
    records_link: bool,
}

impl SyncPlan {
    pub fn cloud_playlist_id(&self) -> i64 {
        self.cloud_playlist_id
    }

    /// `None` when applying this plan creates the local playlist.
    pub fn local_playlist_id(&self) -> Option<i64> {
        self.local_playlist_id
    }

    pub fn creates_playlist(&self) -> bool {
        self.local_playlist_id.is_none()
    }

    pub fn rename_to(&self) -> Option<&str> {
        self.rename_to.as_deref()
    }

    pub fn added(&self) -> &[i64] {
        &self.added
    }

    pub fn removed(&self) -> &[i64] {
        &self.removed
    }

    pub fn conflicts(&self) -> &[SyncConflict] {
        &self.conflicts
    }

    /// Cloud songs no local track matched. They cannot be synced, and the dry run owes the user
    /// that count rather than silently dropping them.
    pub fn unmatched(&self) -> usize {
        self.unmatched
    }

    /// The cloud membership applying this plan would record, in cloud order. A creating plan is
    /// exactly this list, so a preview can show what the new playlist will hold.
    pub fn cloud_track_ids(&self) -> &[i64] {
        &self.cloud_track_ids
    }

    /// True when applying would change nothing: an existing playlist, no membership moves, no
    /// rename, and a snapshot already holding both names. A creating plan is never empty.
    pub fn is_empty(&self) -> bool {
        !self.creates_playlist()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.rename_to.is_none()
            && !self.records_link
    }
}

/// What one apply wrote, for a toast's summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyReport {
    pub local_playlist_id: i64,
    pub added: usize,
    pub removed: usize,
    pub renamed: Option<String>,
}

/// What a caller asks the library to plan.
pub struct SyncRequest<'a> {
    pub cloud_playlist_id: i64,
    pub cloud_name: &'a str,
    pub cloud_songs: &'a [Song],
    /// The local playlist to reconcile into when this cloud playlist has never been applied. An
    /// already-linked cloud playlist ignores this, and a different id here is refused rather than
    /// silently re-pointing the link.
    pub chosen_local_playlist_id: Option<i64>,
    pub policy: SyncPolicy,
}

/// The cloud side of a diff, with its membership already resolved to local track ids.
pub struct CloudSide<'a> {
    pub playlist_id: i64,
    pub name: &'a str,
    pub track_ids: &'a [i64],
    pub unmatched: usize,
}

/// The local playlist as it stands now.
pub struct LocalSide<'a> {
    pub id: i64,
    pub name: &'a str,
    pub track_ids: &'a [i64],
}

/// The names and cloud membership recorded by the last apply.
pub struct Snapshot<'a> {
    pub cloud_name: &'a str,
    pub local_name: &'a str,
    pub track_ids: &'a [i64],
}

/// Reads both sides and the last snapshot, then diffs them.
pub async fn plan(db: &DbPool, request: &SyncRequest<'_>) -> AppResult<SyncPlan> {
    let matches = cloud_match::match_cloud_disk(db, request.cloud_songs).await?;
    let unmatched = request.cloud_songs.len().saturating_sub(matches.len());
    let mut seen = BTreeSet::new();
    let cloud_track_ids = matches
        .iter()
        .map(|found| found.local_track_id)
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    let cloud = CloudSide {
        playlist_id: request.cloud_playlist_id,
        name: request.cloud_name,
        track_ids: &cloud_track_ids,
        unmatched,
    };

    let link = queries::cloud::playlist_link(db, request.cloud_playlist_id).await?;
    let local_id = match (&link, request.chosen_local_playlist_id) {
        (Some(link), Some(chosen)) if chosen != link.local_playlist_id => {
            return Err(AppError::Validation(
                "this cloud playlist is already paired with another local playlist".to_owned(),
            ));
        }
        (Some(link), _) => Some(link.local_playlist_id),
        (None, chosen) => chosen,
    };
    let Some(local_id) = local_id else {
        return Ok(plan_sync(&cloud, None, None, request.policy));
    };

    let playlist = queries::playlist::get_playlist_by_id(db, local_id).await?;
    if playlist.is_smart {
        return Err(AppError::Validation(
            "a smart playlist resolves its own membership and cannot be synced".to_owned(),
        ));
    }
    let local_track_ids = queries::cloud::playlist_track_ids(db, local_id).await?;
    let local = LocalSide { id: local_id, name: &playlist.name, track_ids: &local_track_ids };

    let Some(link) = link else {
        return Ok(plan_sync(&cloud, Some(&local), None, request.policy));
    };
    let snapshot_track_ids =
        queries::cloud::playlist_link_track_ids(db, request.cloud_playlist_id).await?;
    let snapshot = Snapshot {
        cloud_name: &link.cloud_name,
        local_name: &link.local_name,
        track_ids: &snapshot_track_ids,
    };
    Ok(plan_sync(&cloud, Some(&local), Some(&snapshot), request.policy))
}

/// The diff itself, with no I/O, so every conflict rule is decided in one readable place.
pub fn plan_sync(
    cloud: &CloudSide<'_>,
    local: Option<&LocalSide<'_>>,
    snapshot: Option<&Snapshot<'_>>,
    policy: SyncPolicy,
) -> SyncPlan {
    let Some(local) = local else {
        // A playlist this link has never named: cloud order is the only order there is.
        return SyncPlan {
            cloud_playlist_id: cloud.playlist_id,
            local_playlist_id: None,
            create_name: Some(cloud.name.to_owned()),
            rename_to: None,
            added: Vec::new(),
            removed: Vec::new(),
            conflicts: Vec::new(),
            unmatched: cloud.unmatched,
            cloud_name: cloud.name.to_owned(),
            result_name: cloud.name.to_owned(),
            cloud_track_ids: cloud.track_ids.to_vec(),
            records_link: false,
        };
    };

    let cloud_ids = cloud.track_ids.iter().copied().collect::<BTreeSet<_>>();
    let local_ids = local.track_ids.iter().copied().collect::<BTreeSet<_>>();
    let snapshot_ids = snapshot
        .map(|snapshot| snapshot.track_ids.iter().copied().collect::<BTreeSet<_>>())
        .unwrap_or_default();

    let mut added = Vec::new();
    let mut conflicts = Vec::new();
    for id in cloud_ids.difference(&local_ids).copied() {
        if !snapshot_ids.contains(&id) || policy == SyncPolicy::CloudWins {
            added.push(id);
        } else {
            conflicts.push(SyncConflict::LocalRemovalKept(id));
        }
    }

    let mut removed = Vec::new();
    for id in local_ids.difference(&cloud_ids).copied() {
        // No base means no way to tell a local addition from a cloud removal, and the one reading
        // that cannot destroy work is to keep it — on the first sync and under either policy.
        if !snapshot_ids.contains(&id) {
            continue;
        }
        if policy == SyncPolicy::CloudWins {
            removed.push(id);
        } else {
            conflicts.push(SyncConflict::CloudRemovalKept(id));
        }
    }

    let (rename_to, name_kept) = rename_for(
        cloud.name,
        local.name,
        snapshot.map(|snapshot| (snapshot.cloud_name, snapshot.local_name)),
        policy,
    );
    if name_kept {
        conflicts.push(SyncConflict::NameKept);
    }

    let result_name = rename_to.as_deref().unwrap_or(local.name).to_owned();
    // The base either side needs before the next diff can read a name without re-reporting it.
    let records_link = snapshot.is_none_or(|snapshot| {
        snapshot.cloud_name != cloud.name || snapshot.local_name != result_name
    });
    SyncPlan {
        cloud_playlist_id: cloud.playlist_id,
        local_playlist_id: Some(local.id),
        create_name: None,
        rename_to: rename_to.clone(),
        added,
        removed,
        conflicts,
        unmatched: cloud.unmatched,
        cloud_name: cloud.name.to_owned(),
        result_name,
        cloud_track_ids: cloud.track_ids.to_vec(),
        records_link,
    }
}

/// The local titles behind a plan's conflicts, so a review surface can name the track rather than
/// print an id. A name conflict carries no track and contributes no entry.
pub async fn conflict_titles(db: &DbPool, plan: &SyncPlan) -> AppResult<HashMap<i64, String>> {
    let ids = plan
        .conflicts
        .iter()
        .filter_map(|conflict| match conflict {
            SyncConflict::LocalRemovalKept(id) | SyncConflict::CloudRemovalKept(id) => Some(*id),
            SyncConflict::NameKept => None,
        })
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(queries::track::get_track_summaries_by_ids(db, &ids)
        .await?
        .into_iter()
        .map(|summary| (summary.id, summary.title))
        .collect())
}

/// The name to write, and whether a genuine both-sides divergence stayed local.
///
/// One base per side: comparing both names against the cloud's alone would read a playlist linked
/// under its existing local name as locally renamed on every pass.
fn rename_for(
    cloud_name: &str,
    local_name: &str,
    names: Option<(&str, &str)>,
    policy: SyncPolicy,
) -> (Option<String>, bool) {
    let Some((cloud_base, local_base)) = names else {
        return (None, false);
    };
    let cloud_changed = cloud_name != cloud_base;
    if !cloud_changed {
        return (None, false);
    }
    let local_changed = local_name != local_base;
    if !local_changed {
        return (Some(cloud_name.to_owned()), false);
    }
    if cloud_name == local_name {
        return (None, false);
    }
    match policy {
        SyncPolicy::LocalWins => (None, true),
        SyncPolicy::CloudWins => (Some(cloud_name.to_owned()), false),
    }
}

/// Writes a reviewed plan: the only path into a local playlist's membership.
pub async fn apply(db: &DbPool, plan: &SyncPlan) -> AppResult<ApplyReport> {
    let local_playlist_id = if let Some(id) = plan.local_playlist_id {
        id
    } else {
        let name = plan.create_name.as_deref().ok_or_else(|| {
            AppError::Validation("a creating sync plan has no playlist name".to_owned())
        })?;
        queries::playlist::create_playlist_with_tracks(db, name, None, &plan.cloud_track_ids)
            .await?
    };

    queries::cloud::apply_cloud_playlist_sync(
        db,
        &CloudPlaylistSync {
            cloud_playlist_id: plan.cloud_playlist_id,
            local_playlist_id,
            rename_to: plan.rename_to.as_deref(),
            remove_track_ids: &plan.removed,
            add_track_ids: &plan.added,
            cloud_name: &plan.cloud_name,
            local_name: &plan.result_name,
            cloud_track_ids: &plan.cloud_track_ids,
        },
    )
    .await?;

    Ok(ApplyReport {
        local_playlist_id,
        added: plan.added.len(),
        removed: plan.removed.len(),
        renamed: plan.rename_to.clone(),
    })
}

#[cfg(test)]
#[path = "tests/cloud_playlist_sync_tests.rs"]
mod tests;
