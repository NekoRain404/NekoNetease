//! **Sync with local** — the dry run a cloud playlist's page shows before it writes anything.
//!
//! The plan is computed off the UI thread and parked on [`PlaylistsUi`] until Apply reads it, so
//! what is written is exactly the diff the panel reported. A policy pick or a retry starts a fresh
//! dry run and drops the previous plan, which is also what disables Apply until the new one lands.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::ui::playlists::PlaylistsUi;
use nekonetease_app::library::cloud_playlist_sync::{
    self, SyncConflict, SyncPlan, SyncPolicy, SyncRequest,
};
use nekonetease_app::state::AppState;
use nekonetease_core::error::{AppResult, describe};
use nekonetease_ui::{AppWindow, PlaylistDetail, SyncConflictKind, SyncConflictRow};

const SYNC_POLICY_LOCAL: &str = "local";
const SYNC_POLICY_CLOUD: &str = "cloud";

/// A dry run plus the local titles its conflicts name. Kept together because the panel renders
/// both, and a plan that outlived its titles would print ids.
pub(in crate::ui::playlists) struct ReviewedPlan {
    plan: SyncPlan,
    titles: HashMap<i64, String>,
}

pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    wire_open(ui, state, playlists_ui);
    wire_policy(ui, state, playlists_ui);
    wire_apply(ui, state, playlists_ui);
    wire_close(ui, playlists_ui);
}

fn wire_open(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_request_sync_review(move || {
        let Some(ui) = weak.upgrade() else { return };
        let policy =
            policy_of(ui.global::<PlaylistDetail>().get_sync_policy().as_str()).unwrap_or_default();
        spawn_review(&s, &pu, &weak, policy);
    });
}

fn wire_policy(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_set_sync_policy(move |token| {
        let Some(policy) = policy_of(token.as_str()) else { return };
        spawn_review(&s, &pu, &weak, policy);
    });
}

/// Apply writes the reviewed plan and nothing else, then refreshes the detail so the list under
/// the panel shows what the write produced.
fn wire_apply(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_apply_sync(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.global::<PlaylistDetail>().get_sync_applying() {
            return;
        }
        let playlist_id = pu.detail_playlist_id();
        if playlist_id < 0 {
            return;
        }
        let Some(reviewed) = pu.take_sync_plan() else { return };
        let request = pu.sync_request.load(Ordering::Acquire);
        ui.global::<PlaylistDetail>().set_sync_applying(true);
        ui.global::<PlaylistDetail>().set_sync_error(SharedString::default());

        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            match cloud_playlist_sync::apply(&s.db, &reviewed.plan).await {
                Ok(_) => {
                    // The detail refreshes before the panel closes, so the frame that takes the
                    // panel away already holds the list it wrote.
                    super::refresh_after_edit(&s, &pu, weak.clone(), &[playlist_id], "sync").await;
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        forget_panel(&ui.global::<PlaylistDetail>(), &pu);
                    });
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!("playlists::apply_sync({playlist_id}): {message}");
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<PlaylistDetail>();
                        g.set_sync_applying(false);
                        g.set_sync_error(message.into());
                        // The reviewed diff is still what the panel shows, so a failed write must
                        // not leave Apply enabled with nothing behind it — unless the panel has
                        // since been closed or reopened for another playlist.
                        if pu.sync_request.load(Ordering::Acquire) == request
                            && pu.detail_playlist_id() == playlist_id
                        {
                            *pu.sync_plan.lock() = Some(reviewed);
                        }
                    });
                }
            }
        });
    });
}

fn wire_close(ui: &AppWindow, playlists_ui: &Arc<PlaylistsUi>) {
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<PlaylistDetail>().on_close_sync(move || {
        if let Some(ui) = weak.upgrade() {
            forget_panel(&ui.global::<PlaylistDetail>(), &pu);
        }
    });
}

/// Hands the panel back: the globals, the parked plan and the in-flight request. Called from every
/// path that stops the panel describing the playlist it was opened on — close, a fresh open, a
/// section leave and an account switch — since the panel holds no state of its own.
pub(in crate::ui::playlists) fn forget_panel(g: &PlaylistDetail<'_>, playlists_ui: &PlaylistsUi) {
    playlists_ui.forget_sync_review();
    g.set_sync_open(false);
    g.set_sync_loading(false);
    g.set_sync_applying(false);
    g.set_sync_error(SharedString::default());
    g.set_sync_can_apply(false);
    g.set_sync_no_changes(false);
    g.set_sync_creates_playlist(false);
    g.set_sync_cloud_tracks(0);
    g.set_sync_added(0);
    g.set_sync_removed(0);
    g.set_sync_unmatched(0);
    g.set_sync_rename_to(SharedString::default());
    g.set_sync_conflicts(ModelRc::default());
}

fn spawn_review(
    state: &AppState,
    playlists_ui: &Arc<PlaylistsUi>,
    weak: &Weak<AppWindow>,
    policy: SyncPolicy,
) {
    let playlist_id = playlists_ui.detail_playlist_id();
    if playlist_id < 0 {
        return;
    }
    let Some(ui) = weak.upgrade() else { return };
    let request = playlists_ui.sync_request.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let generation = state.cloud.status().generation;
    playlists_ui.forget_sync_plan();

    let g = ui.global::<PlaylistDetail>();
    g.set_sync_open(true);
    g.set_sync_loading(true);
    g.set_sync_applying(false);
    g.set_sync_error(SharedString::default());
    g.set_sync_can_apply(false);
    g.set_sync_policy(SharedString::from(policy_token(policy)));
    g.set_sync_conflicts(ModelRc::default());

    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = weak.clone();
    s.runtime.clone().spawn(async move {
        let result = compute_review(&s, playlist_id, policy).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if pu.sync_request.load(Ordering::Acquire) != request
                || s.cloud.status().generation != generation
                || pu.detail_playlist_id() != playlist_id
            {
                return;
            }
            let g = ui.global::<PlaylistDetail>();
            g.set_sync_loading(false);
            match result {
                Ok(reviewed) => {
                    publish_review(&g, &reviewed);
                    *pu.sync_plan.lock() = Some(reviewed);
                }
                Err(error) => {
                    log::warn!("playlists::sync_review({playlist_id}): {}", describe(&error));
                    g.set_sync_error(describe(&error).into());
                }
            }
        });
    });
}

/// The dry run itself. The cloud side is refetched rather than read from the detail's rows: the
/// plan diffs against the last apply's snapshot, and the page's rows may be sorted, filtered or a
/// refresh behind.
async fn compute_review(
    state: &AppState,
    playlist_id: i64,
    policy: SyncPolicy,
) -> AppResult<ReviewedPlan> {
    let (playlist, songs) = state.cloud.library_playlist(playlist_id).await?;
    let request = SyncRequest {
        cloud_playlist_id: playlist.id,
        cloud_name: &playlist.name,
        cloud_songs: &songs,
        chosen_local_playlist_id: None,
        policy,
    };
    let plan = cloud_playlist_sync::plan(&state.db, &request).await?;
    let titles = cloud_playlist_sync::conflict_titles(&state.db, &plan).await?;
    Ok(ReviewedPlan { plan, titles })
}

fn publish_review(g: &PlaylistDetail<'_>, reviewed: &ReviewedPlan) {
    let plan = &reviewed.plan;
    g.set_sync_loading(false);
    g.set_sync_creates_playlist(plan.creates_playlist());
    g.set_sync_no_changes(plan.is_empty());
    g.set_sync_cloud_tracks(count(plan.cloud_track_ids().len()));
    g.set_sync_added(count(plan.added().len()));
    g.set_sync_removed(count(plan.removed().len()));
    g.set_sync_unmatched(count(plan.unmatched()));
    g.set_sync_rename_to(plan.rename_to().map(SharedString::from).unwrap_or_default());
    g.set_sync_conflicts(ModelRc::from(Rc::new(VecModel::from(conflict_rows(
        plan,
        &reviewed.titles,
    )))));
    // A creating plan is never empty, so this covers the first sync too.
    g.set_sync_can_apply(!plan.is_empty());
}

/// One row per conflict, in the order the plan reports them. The sentence each kind reads is the
/// panel's, Rust having no `@tr`; the label is the local track where the kind names one.
fn conflict_rows(plan: &SyncPlan, titles: &HashMap<i64, String>) -> Vec<SyncConflictRow> {
    plan.conflicts()
        .iter()
        .map(|conflict| match conflict {
            SyncConflict::LocalRemovalKept(id) => SyncConflictRow {
                kind: SyncConflictKind::LocalRemovalKept,
                label: title_of(titles, *id),
            },
            SyncConflict::CloudRemovalKept(id) => SyncConflictRow {
                kind: SyncConflictKind::CloudRemovalKept,
                label: title_of(titles, *id),
            },
            SyncConflict::NameKept => {
                SyncConflictRow { kind: SyncConflictKind::NameKept, label: SharedString::default() }
            }
        })
        .collect()
}

fn title_of(titles: &HashMap<i64, String>, id: i64) -> SharedString {
    titles.get(&id).map_or_else(SharedString::default, |title| SharedString::from(title.as_str()))
}

fn count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn policy_token(policy: SyncPolicy) -> &'static str {
    match policy {
        SyncPolicy::LocalWins => SYNC_POLICY_LOCAL,
        SyncPolicy::CloudWins => SYNC_POLICY_CLOUD,
    }
}

fn policy_of(token: &str) -> Option<SyncPolicy> {
    if token == SYNC_POLICY_LOCAL {
        Some(SyncPolicy::LocalWins)
    } else if token == SYNC_POLICY_CLOUD {
        Some(SyncPolicy::CloudWins)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/sync_tests.rs"]
mod tests;
