//! The artist-detail filmography strip, the music video opened from it, and that video's peers.
//!
//! The list pages through the server's `limit`/`offset`; the detail is fetched per open and
//! dropped on close, so nothing here outlives the page the user is looking at.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::ui::util::parse_cloud_id;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::{CommentThread, Mv};
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, CloudEntityDetail, MvCardRow, MvDetail, MvPlayer};

#[cfg(test)]
use super::cloud_mvs_player::{pick_quality, volume_amplitude};

/// Separates a list page asked for by an artist detail that has since been closed or retried.
static MV_LIST_REQUEST: AtomicU64 = AtomicU64::new(0);
pub(super) static MV_DETAIL_REQUEST: AtomicU64 = AtomicU64::new(0);
/// The thread header's like state, which the detail route does not carry and which arrives on
/// its own request, so it keeps its own token.
static MV_THREAD_REQUEST: AtomicU64 = AtomicU64::new(0);
/// The similar list re-fetches per open and retry without touching the page's own requests.
static MV_SIMILAR_REQUEST: AtomicU64 = AtomicU64::new(0);
/// Like and collect share one token because both are disabled while either is in flight: a
/// second press that landed first must retire the one before it rather than let both patch.
static MV_ACTION_REQUEST: AtomicU64 = AtomicU64::new(0);

pub(super) fn wire(ui: &AppWindow, state: &AppState) {
    wire_list(ui, state);
    wire_detail(ui, state);
    super::cloud_mvs_player::wire(ui, state);
    wire_actions(ui, state);
}

fn wire_list(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<CloudEntityDetail>().on_load_more_mvs(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<CloudEntityDetail>();
        if detail.get_kind().as_str() != "artist" || detail.get_mvs_loading() {
            return;
        }
        let remote_id = detail.get_remote_id().to_string();
        let Some(artist_id) = parse_cloud_id(&remote_id) else { return };
        let offset = u64::try_from(detail.get_mvs().row_count()).unwrap_or(u64::MAX);
        let request = MV_LIST_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
        detail.set_mvs_loading(true);
        detail.set_mvs_error(SharedString::default());
        load_mvs_page(&s, ui.as_weak(), request, remote_id, artist_id, offset);
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<CloudEntityDetail>().on_open_mv(move |remote_id| {
        let Some(ui) = weak.upgrade() else { return };
        open_mv(&ui, &s, remote_id.to_string());
    });
}

fn wire_detail(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    ui.global::<MvDetail>().on_close(move || {
        MV_DETAIL_REQUEST.fetch_add(1, Ordering::Relaxed);
        MV_THREAD_REQUEST.fetch_add(1, Ordering::Relaxed);
        MV_ACTION_REQUEST.fetch_add(1, Ordering::Relaxed);
        MV_SIMILAR_REQUEST.fetch_add(1, Ordering::Relaxed);
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        detail.set_open(false);
        detail.set_remote_id(SharedString::default());
        detail.set_title(SharedString::default());
        detail.set_artist(SharedString::default());
        detail.set_artwork_path(SharedString::default());
        detail.set_publish_date(SharedString::default());
        detail.set_duration(SharedString::default());
        detail.set_play_count(SharedString::default());
        detail.set_description(SharedString::default());
        detail.set_loading(false);
        detail.set_error(SharedString::default());
        detail.set_resolving(false);
        detail.set_liked(false);
        detail.set_collected(false);
        detail.set_like_count(0);
        detail.set_collect_count(0);
        detail.set_share_count(0);
        detail.set_can_act(false);
        detail.set_acting(false);
        detail.set_similar_mvs(ModelRc::default());
        detail.set_similar_loading(false);
        detail.set_similar_error(SharedString::default());
        // The player owns the stream, the window's fullscreen flag and the surface's own
        // properties; its close handler is the one place that hands all three back.
        ui.global::<MvPlayer>().invoke_close();
        crate::ui::comments::close(&ui);
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_retry(move || {
        let Some(ui) = weak.upgrade() else { return };
        let remote_id = ui.global::<MvDetail>().get_remote_id().to_string();
        open_mv(&ui, &s, remote_id);
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_open_similar(move |remote_id| {
        let Some(ui) = weak.upgrade() else { return };
        open_mv(&ui, &s, remote_id.to_string());
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_retry_similar(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        let remote_id = detail.get_remote_id().to_string();
        let Some(id) = parse_cloud_id(&remote_id) else { return };
        let request = MV_SIMILAR_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
        detail.set_similar_loading(true);
        detail.set_similar_error(SharedString::default());
        load_similar_mvs(&s, ui.as_weak(), request, remote_id, id);
    });
}

/// Resets the strip for whichever detail is about to open, so a page already in flight can
/// neither land on an album nor append to the next artist.
pub(super) fn reset(ui: &AppWindow) {
    MV_LIST_REQUEST.fetch_add(1, Ordering::Relaxed);
    let detail = ui.global::<CloudEntityDetail>();
    detail.set_mvs(ModelRc::default());
    detail.set_mvs_loading(false);
    detail.set_mvs_has_more(false);
    detail.set_mvs_error(SharedString::default());
}

pub(super) fn start_artist_mvs(
    ui: &AppWindow,
    state: &AppState,
    artist_id: i64,
    remote_id: String,
) {
    let detail = ui.global::<CloudEntityDetail>();
    detail.set_mvs(ModelRc::from(Rc::new(VecModel::default())));
    detail.set_mvs_loading(true);
    detail.set_mvs_has_more(false);
    detail.set_mvs_error(SharedString::default());
    let request = MV_LIST_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    load_mvs_page(state, ui.as_weak(), request, remote_id, artist_id, 0);
}

fn load_mvs_page(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: u64,
    artist_remote_id: String,
    artist_id: i64,
    offset: u64,
) {
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.artist_mvs_page(artist_id, offset).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if MV_LIST_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let detail = ui.global::<CloudEntityDetail>();
            if !detail.get_open()
                || detail.get_kind().as_str() != "artist"
                || detail.get_remote_id().as_str() != artist_remote_id
            {
                return;
            }
            detail.set_mvs_loading(false);
            match result {
                Ok((mvs, has_more)) => {
                    let empty = mvs.is_empty();
                    append_mv_cards(&detail, mv_card_rows(mvs));
                    // An empty page would leave the offset where it is; stopping is the only
                    // answer that doesn't ask for the same rows forever.
                    detail.set_mvs_has_more(has_more && !empty);
                    detail.set_mvs_error(SharedString::default());
                }
                Err(error) => {
                    log::warn!(
                        "cloud_mvs::artist_mvs({artist_id}, {offset}): {}",
                        describe(&error)
                    );
                    detail.set_mvs_error(describe(&error).into());
                }
            }
        });
    });
}

fn append_mv_cards(detail: &CloudEntityDetail<'_>, rows: Vec<MvCardRow>) {
    let model = detail.get_mvs();
    let Some(model) = model.as_any().downcast_ref::<VecModel<MvCardRow>>() else { return };
    for row in rows {
        model.push(row);
    }
}

pub(in crate::ui) fn open_mv(ui: &AppWindow, state: &AppState, remote_id: String) {
    let Some(id) = parse_cloud_id(&remote_id) else {
        log::warn!("cloud_mvs::open_mv: invalid NetEase MV ID");
        return;
    };
    // The rail opens the next video over the one playing: leaving the previous stream running
    // would play it under a page describing the one the viewer picked. Closing does not spend
    // the ticket — the new open resolves its own source.
    if ui.global::<MvPlayer>().get_open() {
        ui.global::<MvPlayer>().invoke_close();
    }
    let request = MV_DETAIL_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let thread_request = MV_THREAD_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let similar_request = MV_SIMILAR_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    MV_ACTION_REQUEST.fetch_add(1, Ordering::Relaxed);
    crate::ui::comments::close(ui);
    let detail = ui.global::<MvDetail>();
    detail.set_open(true);
    detail.set_remote_id(remote_id.clone().into());
    detail.set_title(SharedString::default());
    detail.set_artist(SharedString::default());
    detail.set_artwork_path(SharedString::default());
    detail.set_publish_date(SharedString::default());
    detail.set_duration(SharedString::default());
    detail.set_play_count(SharedString::default());
    detail.set_description(SharedString::default());
    detail.set_error(SharedString::default());
    detail.set_loading(true);
    detail.set_resolving(false);
    detail.set_liked(false);
    detail.set_collected(false);
    detail.set_like_count(0);
    detail.set_collect_count(0);
    detail.set_share_count(0);
    detail.set_can_act(state.cloud.status().account.is_some());
    detail.set_acting(false);
    detail.set_similar_mvs(ModelRc::default());
    detail.set_similar_loading(true);
    detail.set_similar_error(SharedString::default());
    load_mv_detail(state, ui.as_weak(), request, remote_id.clone(), id);
    load_mv_thread_info(state, ui.as_weak(), thread_request, remote_id.clone(), id);
    load_similar_mvs(state, ui.as_weak(), similar_request, remote_id, id);
}

/// The peers load beside the page rather than in front of it: a refusal empties the rail and
/// leaves the video playing.
fn load_similar_mvs(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: u64,
    remote_id: String,
    id: i64,
) {
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.similar_mvs(id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if MV_SIMILAR_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let detail = ui.global::<MvDetail>();
            if !detail.get_open() || detail.get_remote_id().as_str() != remote_id {
                return;
            }
            detail.set_similar_loading(false);
            match result {
                Ok(mvs) => {
                    detail.set_similar_error(SharedString::default());
                    detail
                        .set_similar_mvs(ModelRc::from(Rc::new(VecModel::from(mv_card_rows(mvs)))));
                }
                Err(error) => {
                    log::warn!("cloud_mvs::similar_mvs({id}): {}", describe(&error));
                    detail.set_similar_error(describe(&error).into());
                }
            }
        });
    });
}

fn load_mv_thread_info(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: u64,
    remote_id: String,
    id: i64,
) {
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.mv_thread_info(id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if MV_THREAD_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let detail = ui.global::<MvDetail>();
            if !detail.get_open() || detail.get_remote_id().as_str() != remote_id {
                return;
            }
            match result {
                Ok(info) => {
                    detail.set_liked(info.liked);
                    detail.set_like_count(count_i32(info.liked_count));
                }
                // The detail itself rendered; a missing like count is a quieter failure than
                // replacing the page with an error.
                Err(error) => {
                    log::warn!("cloud_mvs::mv_thread_info({id}): {}", describe(&error));
                }
            }
        });
    });
}

/// Like and collect share their body shape: both read the page, guard on the signed-in state,
/// then patch only the field the server confirmed.
#[derive(Clone, Copy)]
enum MvAction {
    Like(bool),
    Collect(bool),
}

fn wire_actions(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_toggle_like(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        if !detail.get_can_act() || detail.get_acting() {
            return;
        }
        let remote_id = detail.get_remote_id().to_string();
        let Some(id) = parse_cloud_id(&remote_id) else { return };
        let request = MV_ACTION_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
        detail.set_acting(true);
        spawn_action(&s, ui.as_weak(), request, remote_id, id, MvAction::Like(!detail.get_liked()));
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_toggle_collect(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        if !detail.get_can_act() || detail.get_acting() {
            return;
        }
        let remote_id = detail.get_remote_id().to_string();
        let Some(id) = parse_cloud_id(&remote_id) else { return };
        let request = MV_ACTION_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
        detail.set_acting(true);
        spawn_action(
            &s,
            ui.as_weak(),
            request,
            remote_id,
            id,
            MvAction::Collect(!detail.get_collected()),
        );
    });

    let weak = ui.as_weak();
    ui.global::<MvDetail>().on_share(move || {
        let Some(ui) = weak.upgrade() else { return };
        let remote_id = ui.global::<MvDetail>().get_remote_id();
        if remote_id.is_empty() {
            return;
        }
        // The clipboard helper never logs what it copies; the URL carries only the public id.
        let link = format!("https://music.163.com/mv?id={remote_id}");
        crate::ui::clipboard::write(&ui, &link);
    });
}

fn spawn_action(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: u64,
    remote_id: String,
    id: i64,
    action: MvAction,
) {
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = match action {
            MvAction::Like(like) => state.cloud.mv_like(id, like).await,
            MvAction::Collect(subscribed) => state.cloud.mv_subscribe(id, subscribed).await,
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let detail = ui.global::<MvDetail>();
            if !detail.get_open() || detail.get_remote_id().as_str() != remote_id {
                return;
            }
            // A newer press owns the acting flag; the stale answer must not clear it.
            if MV_ACTION_REQUEST.load(Ordering::Relaxed) != request {
                return;
            }
            if state.cloud.status().generation != generation {
                detail.set_acting(false);
                return;
            }
            detail.set_acting(false);
            match result {
                Ok(()) => match action {
                    MvAction::Like(liked) => {
                        detail.set_liked(liked);
                        detail.set_like_count(bump(detail.get_like_count(), liked));
                    }
                    MvAction::Collect(collected) => {
                        detail.set_collected(collected);
                        detail.set_collect_count(bump(detail.get_collect_count(), collected));
                    }
                },
                Err(error) => {
                    log::warn!("cloud_mvs::action({id}): {}", describe(&error));
                    toast::notify(ToastKind::OperationFailed, describe(&error));
                }
            }
        });
    });
}

fn bump(count: i32, added: bool) -> i32 {
    if added { count.saturating_add(1) } else { count.saturating_sub(1).max(0) }
}

fn count_i32(count: u64) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn load_mv_detail(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: u64,
    remote_id: String,
    id: i64,
) {
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.mv_detail(id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if MV_DETAIL_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let detail = ui.global::<MvDetail>();
            if !detail.get_open() || detail.get_remote_id().as_str() != remote_id {
                return;
            }
            detail.set_loading(false);
            match result {
                Ok(mv) => {
                    let title = mv.name.clone();
                    detail.set_title(mv.name.into());
                    detail.set_artist(mv.artist_name.into());
                    detail.set_artwork_path(remote_artwork(mv.cover_url).into());
                    detail.set_publish_date(publish_date(mv.publish_time_ms).into());
                    detail.set_duration(duration_text(mv.duration_ms).into());
                    detail.set_play_count(mv.play_count.to_string().into());
                    detail.set_description(mv.description.trim().to_owned().into());
                    detail.set_collected(mv.subscribed);
                    detail.set_collect_count(count_i32(mv.sub_count));
                    detail.set_share_count(count_i32(mv.share_count));
                    detail.set_can_act(state.cloud.status().account.is_some());
                    // The page owns the thread below the video; the standalone branch is
                    // excluded while `MvDetail.open`, so `Comments.open` stays its own flag.
                    crate::ui::comments::open(&ui, &state, CommentThread::Mv(id), title.as_str());
                }
                Err(error) => {
                    log::warn!("cloud_mvs::mv_detail({id}): {}", describe(&error));
                    detail.set_error(describe(&error).into());
                }
            }
        });
    });
}

fn mv_card_rows(mvs: Vec<Mv>) -> Vec<MvCardRow> {
    mvs.into_iter()
        .map(|mv| MvCardRow {
            remote_id: mv.id.to_string().into(),
            title: mv.name.into(),
            subtitle: duration_text(mv.duration_ms).into(),
            artwork_path: remote_artwork(mv.cover_url).into(),
        })
        .collect()
}

fn duration_text(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    crate::ui::tracks::format_duration_ms(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// The wire sometimes carries a publish time of `0` for a video the uploader hid.
fn publish_date(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|at| at.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn remote_artwork(url: Option<String>) -> String {
    url.filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url)).unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/cloud_mvs_tests.rs"]
mod tests;
