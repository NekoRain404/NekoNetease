//! The roaming page: one continuous stream, started from a mode the user picks.
//!
//! A roam is playback, so it outlives the page and the session record here describes it rather
//! than owning it. The stream itself is `cloud_roaming::run` unchanged — fetch, seat the cloud
//! queue, refill before it runs out — and this slice owns the page's three verbs: start (with the
//! current cloud track as the seed), skip, and "not interested", the last only over Personal FM
//! where the service has an opinion to record.
//!
//! No My Library global backs this page: the modes are its own, and the session is keyed by the
//! generations a queue replacement or an account switch moves.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use slint::{ComponentHandle, SharedString};
use tokio_util::sync::CancellationToken;

use crate::ui::view_ctx::ViewCtx;
use nekonetease_app::cloud_roaming::{self, RoamingRequest};
use nekonetease_app::library::playback;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::RoamingMode;
use nekonetease_core::error::AppResult;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::{PlayerState, lock_state};
use nekonetease_engine::player::engine::types::PlaybackStatus;
use nekonetease_ui::{AppWindow, Roaming};

/// This page's `Nav.selected-index`.
pub const NAV_ROAMING: i32 = 11;

/// A running roam, as this page remembers it. Every field is a guard against acting on a session
/// that has since been replaced: the request is bumped by start and stop, the queue generation
/// moves when anything else seats a queue, and the account generation moves on a sign-out.
#[derive(Clone, Copy)]
struct RoamingSession {
    request_id: u64,
    mode: RoamingMode,
    queue_generation: u64,
    account_generation: u64,
}

struct RoamingUi {
    request: AtomicU64,
    cancel: Mutex<CancellationToken>,
    session: Mutex<Option<RoamingSession>>,
    feedback_pending: AtomicBool,
}

impl RoamingUi {
    fn new() -> Self {
        Self {
            request: AtomicU64::new(0),
            cancel: Mutex::new(CancellationToken::new()),
            session: Mutex::new(None),
            feedback_pending: AtomicBool::new(false),
        }
    }

    fn next_request(&self) -> u64 {
        self.request.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    fn is_current(&self, request_id: u64) -> bool {
        self.request.load(Ordering::Acquire) == request_id
    }

    /// Replace the running cancellation, cancelling whatever it was holding.
    fn replace_cancel(&self) -> CancellationToken {
        let cancellation = CancellationToken::new();
        let mut held = self.cancel.lock();
        held.cancel();
        *held = cancellation.clone();
        cancellation
    }
}

/// Wire the page's callbacks. Mounting the view is the shell's job.
pub fn install(cx: ViewCtx<'_>) {
    let ViewCtx { app, state, .. } = cx;
    let state = state.clone();
    let roaming_ui = Arc::new(RoamingUi::new());

    let weak = app.as_weak();
    let start_state = state.clone();
    let start_ui = roaming_ui.clone();
    app.global::<Roaming>().on_start(move |mode| {
        let Some(ui) = weak.upgrade() else { return };
        start(&ui, &start_state, &start_ui, mode);
    });

    let weak = app.as_weak();
    let stop_state = state.clone();
    let stop_ui = roaming_ui.clone();
    app.global::<Roaming>().on_stop(move || {
        let Some(ui) = weak.upgrade() else { return };
        stop(&ui, &stop_state, &stop_ui);
    });

    let weak = app.as_weak();
    let skip_state = state.clone();
    app.global::<Roaming>().on_skip(move || {
        if weak.upgrade().is_none() {
            return;
        }
        let state = skip_state.clone();
        let runtime = state.runtime.clone();
        runtime.spawn(async move {
            if let Err(error) = playback::player_next(&state.playback_ctx()) {
                log::warn!("roaming::skip: {}", describe(&error));
            }
        });
    });

    let weak = app.as_weak();
    let dislike_state = state.clone();
    let dislike_ui = roaming_ui.clone();
    app.global::<Roaming>().on_dislike(move || {
        let Some(ui) = weak.upgrade() else { return };
        dislike(&ui, &dislike_state, &dislike_ui);
    });

    // A roam is playback and the page keeps no cache, so a leave returns nothing. The gate is
    // still wired because a section leave is observable nowhere else.
    app.global::<Roaming>().on_section_active_changed(|_active| {});
}

fn start(ui: &AppWindow, state: &AppState, roaming_ui: &Arc<RoamingUi>, index: i32) {
    let mode = match index {
        0 => RoamingMode::Personal,
        1 => RoamingMode::Heart,
        2 => RoamingMode::Similar,
        _ => return,
    };
    let request_id = roaming_ui.next_request();
    let cancellation = roaming_ui.replace_cancel();
    let seed = lock_state(&state.player_state)
        .source
        .as_ref()
        .filter(|track| track.file_path.is_empty())
        .map(|track| track.id);
    // The generation `play_queue` is about to take, fixed before the fetch so a queue seated in
    // the meantime reads as a different session rather than as this one's.
    let queue_generation = state.cloud_playback.queue_generation().wrapping_add(1);
    *roaming_ui.session.lock() = Some(RoamingSession {
        request_id,
        mode,
        queue_generation,
        account_generation: state.cloud.status().generation,
    });
    roaming_ui.feedback_pending.store(false, Ordering::Release);

    let g = ui.global::<Roaming>();
    g.set_mode(index);
    g.set_loading(true);
    g.set_error(SharedString::default());
    g.set_feedback_pending(false);

    let playback_generation = state.engine.reserve_playback_request();
    let runtime = state.runtime.clone();
    let state = state.clone();
    let roaming_ui = roaming_ui.clone();
    let weak = ui.as_weak();
    runtime.spawn(async move {
        let started_ui = weak.clone();
        let started_ui_state = roaming_ui.clone();
        let result = cloud_roaming::run(
            &state,
            RoamingRequest { mode, seed, cancellation, queue_generation, playback_generation },
            move || {
                let _ = started_ui.upgrade_in_event_loop(move |ui| {
                    if started_ui_state.is_current(request_id) {
                        ui.global::<Roaming>().set_loading(false);
                    }
                });
            },
        )
        .await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !roaming_ui.is_current(request_id) {
                return;
            }
            *roaming_ui.session.lock() = None;
            roaming_ui.feedback_pending.store(false, Ordering::Release);
            let g = ui.global::<Roaming>();
            g.set_mode(-1);
            g.set_loading(false);
            g.set_feedback_pending(false);
            if let Err(error) = result {
                g.set_error(describe(&error).into());
            }
        });
    });
}

fn stop(ui: &AppWindow, state: &AppState, roaming_ui: &Arc<RoamingUi>) {
    roaming_ui.next_request();
    roaming_ui.cancel.lock().cancel();
    *roaming_ui.session.lock() = None;
    roaming_ui.feedback_pending.store(false, Ordering::Release);
    state.engine.stop();
    let g = ui.global::<Roaming>();
    g.set_mode(-1);
    g.set_loading(false);
    g.set_feedback_pending(false);
    g.set_error(SharedString::default());
}

/// Mark the current Personal FM song "not interested", then advance past it.
///
/// The service call and the advance are two steps and the song can leave the player between them,
/// so both sides of the await re-ask [`current_personal_roaming_song`]; a mismatched generation,
/// mode or source means the answer belongs to a roam that is no longer running and the skip is
/// dropped rather than applied to whatever plays now.
fn dislike(ui: &AppWindow, state: &AppState, roaming_ui: &Arc<RoamingUi>) {
    if roaming_ui
        .feedback_pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let request_id = roaming_ui.request.load(Ordering::Acquire);
    let session = *roaming_ui.session.lock();
    let song_id = {
        let player = lock_state(&state.player_state);
        current_personal_roaming_song(
            request_id,
            session,
            state.cloud_playback.queue_generation(),
            state.cloud.status().generation,
            &player,
        )
    };
    let Some(song_id) = song_id else {
        roaming_ui.feedback_pending.store(false, Ordering::Release);
        return;
    };
    let g = ui.global::<Roaming>();
    g.set_feedback_pending(true);
    g.set_error(SharedString::default());

    let runtime = state.runtime.clone();
    let state = state.clone();
    let roaming_ui = roaming_ui.clone();
    let weak = ui.as_weak();
    runtime.spawn(async move {
        let still_current = || {
            current_personal_roaming_song(
                request_id,
                *roaming_ui.session.lock(),
                state.cloud_playback.queue_generation(),
                state.cloud.status().generation,
                &lock_state(&state.player_state),
            ) == Some(song_id)
        };
        let result = trash_then_skip(still_current, state.cloud.fm_trash(song_id), || {
            let mut skipped = false;
            state.playback_ctx().emit_and_execute(|player| {
                if current_personal_roaming_song(
                    request_id,
                    *roaming_ui.session.lock(),
                    state.cloud_playback.queue_generation(),
                    state.cloud.status().generation,
                    player,
                ) == Some(song_id)
                {
                    skipped = true;
                    player.build_next_actions()
                } else {
                    Vec::new()
                }
            });
            Ok(skipped)
        })
        .await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !roaming_ui.is_current(request_id) {
                return;
            }
            roaming_ui.feedback_pending.store(false, Ordering::Release);
            let g = ui.global::<Roaming>();
            g.set_feedback_pending(false);
            if let Err(error) = result {
                g.set_error(describe(&error).into());
            }
        });
    });
}

fn current_personal_roaming_song(
    request_id: u64,
    session: Option<RoamingSession>,
    queue_generation: u64,
    account_generation: u64,
    player: &PlayerState,
) -> Option<i64> {
    let session = session?;
    if session.request_id != request_id
        || session.mode != RoamingMode::Personal
        || session.queue_generation != queue_generation
        || session.account_generation != account_generation
        || player.status == PlaybackStatus::Stopped
    {
        return None;
    }
    let source = player.source.as_deref().filter(|track| track.file_path.is_empty())?;
    let queued = player.queue.get_current()?.as_ref();
    if queued.file_path.is_empty() && queued.id == source.id { Some(source.id) } else { None }
}

async fn trash_then_skip<F, Fut, G>(mut still_current: F, trash: Fut, skip: G) -> AppResult<bool>
where
    F: FnMut() -> bool,
    Fut: Future<Output = AppResult<()>>,
    G: FnOnce() -> AppResult<bool>,
{
    if !still_current() {
        return Ok(false);
    }
    trash.await?;
    if !still_current() {
        return Ok(false);
    }
    skip()
}

#[cfg(test)]
#[path = "tests/roaming_tests.rs"]
mod tests;
