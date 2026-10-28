//! Account-library categories and the three continuous roaming entry points.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::ComponentHandle;
use tokio_util::sync::CancellationToken;

use nekonetease_app::cloud_roaming::{self, RoamingRequest};
use nekonetease_app::services::view_state::mutate_view_state;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::{DAILY_PLAYLIST_ID, PERSONAL_RADAR_ID, RoamingMode};
use nekonetease_core::error::AppResult;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::{PlayerState, lock_state};
use nekonetease_engine::player::engine::types::PlaybackStatus;
use nekonetease_ui::{AppWindow, MyLibrary, Nav, PlaylistDetail, Playlists};

use super::super::{PlaylistsUi, fetch_grid, open_playlist};

pub(super) fn wire(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let weak = ui.as_weak();
    let s = state.clone();
    let pu = playlists_ui.clone();
    ui.global::<Playlists>().on_category_changed(move |category| {
        let Some(ui) = weak.upgrade() else { return };
        let category =
            crate::ui::tab_bar::clamp_tab(category, ui.global::<Playlists>().get_category_count());
        pu.category.store(category, Ordering::Release);
        pu.grid_request.fetch_add(1, Ordering::AcqRel);
        pu.detail_request.fetch_add(1, Ordering::AcqRel);
        let detail = ui.global::<PlaylistDetail>();
        if detail.get_playlist_id() >= 0 {
            detail.invoke_close_detail();
        }
        let library = ui.global::<MyLibrary>();
        library.set_tab_idx(library.get_tab_playlists());
        library.invoke_tab_changed(library.get_tab_playlists());
        let playlists = ui.global::<Playlists>();
        playlists.set_cloud_error(slint::SharedString::default());
        playlists.set_retry_special(-1);
        playlists.set_cloud_loading(true);
        playlists.set_total_count(-1);
        crate::ui::model_diff::clear_vec_model::<nekonetease_ui::PlaylistGridRow>(
            &playlists.get_grid_rows(),
            "library category change",
        );
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            let paths = s.paths.clone();
            let saved = tokio::task::spawn_blocking(move || {
                mutate_view_state(&paths, |view| {
                    view.music_library_category = category;
                })
            })
            .await;
            if let Err(description) = saved
                .map_err(|join_error| describe(&join_error))
                .and_then(|result| result.map_err(|persist_error| describe(&persist_error)))
            {
                log::warn!("library category persistence: {description}");
            }
            if let Err(error) = fetch_grid(&s, &pu, weak).await {
                log::warn!("library category: {}", describe(&error));
            }
        });
    });

    let weak = ui.as_weak();
    let s = state.clone();
    let pu = playlists_ui.clone();
    ui.global::<Playlists>().on_open_special(move |kind| {
        let id = match kind {
            0 => DAILY_PLAYLIST_ID,
            1 => PERSONAL_RADAR_ID,
            _ => return,
        };
        let weak = weak.clone();
        let s = s.clone();
        let pu = pu.clone();
        if let Some(ui) = weak.upgrade() {
            // The special playlists live on My Library's Playlists tab, and the Recommended page
            // calls this door while My Library is unmounted. Flip the page first so the detail
            // the fetch opens is visible, and clear a detail an earlier visit left open before
            // the nav moves over it — Search's `open-chart` does the same, in the same order.
            if ui.global::<Nav>().get_selected_index() != crate::ui::my_library::NAV_MY_LIBRARY {
                ui.global::<PlaylistDetail>().set_playlist_id(-1);
                let tab = ui.global::<MyLibrary>().get_tab_playlists();
                crate::ui::my_library::go_to_tab(&ui, tab);
            }
            ui.global::<Playlists>().set_cloud_loading(true);
            ui.global::<Playlists>().set_cloud_error(slint::SharedString::default());
            ui.global::<Playlists>().set_retry_special(kind);
        }
        s.runtime.clone().spawn(async move {
            let request = pu.detail_request.load(Ordering::Acquire).wrapping_add(1);
            let result =
                open_playlist(&s, &pu, weak.clone(), id, nekonetease_ui::NavEnterFrom::Right).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if pu.detail_request.load(Ordering::Acquire) != request {
                    return;
                }
                let playlists = ui.global::<Playlists>();
                playlists.set_cloud_loading(false);
                match result {
                    Err(error) => playlists.set_cloud_error(describe(&error).into()),
                    Ok(()) => playlists.set_retry_special(-1),
                }
            });
        });
    });

    wire_roaming(ui, state, playlists_ui);
}

fn wire_roaming(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    wire_start_roaming(ui, state, playlists_ui);
    wire_stop_roaming(ui, state, playlists_ui);
    wire_dislike_roaming(ui, state, playlists_ui);
}

fn wire_start_roaming(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let s = state.clone();
    let pu = playlists_ui.clone();
    let weak = ui.as_weak();
    ui.global::<Playlists>().on_start_roaming(move |index| {
        let mode = match index {
            0 => RoamingMode::Personal,
            1 => RoamingMode::Heart,
            2 => RoamingMode::Similar,
            _ => return,
        };
        let Some(ui) = weak.upgrade() else { return };
        let cancellation = CancellationToken::new();
        pu.roaming_cancel.lock().cancel();
        *pu.roaming_cancel.lock() = cancellation.clone();
        let request_id = pu.roaming_request.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let seed = lock_state(&s.player_state)
            .source
            .as_ref()
            .filter(|track| track.file_path.is_empty())
            .map(|track| track.id);
        let playlists = ui.global::<Playlists>();
        playlists.set_roaming_mode(index);
        playlists.set_roaming_loading(true);
        playlists.set_roaming_error(slint::SharedString::default());
        let queue_generation = s.cloud_playback.queue_generation();
        let account_generation = s.cloud.status().generation;
        *pu.roaming_session.lock() = Some(super::super::RoamingSession {
            request_id,
            mode,
            queue_generation: queue_generation.wrapping_add(1),
            account_generation,
        });
        pu.roaming_feedback_pending.store(false, Ordering::Release);
        playlists.set_roaming_feedback_pending(false);
        let playback_generation = s.engine.reserve_playback_request();
        let s = s.clone();
        let pu = pu.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            let started_ui = weak.clone();
            let started_pu = pu.clone();
            let result = cloud_roaming::run(
                &s,
                RoamingRequest { mode, seed, cancellation, queue_generation, playback_generation },
                move || {
                    let _ = started_ui.upgrade_in_event_loop(move |ui| {
                        if started_pu.roaming_request.load(Ordering::Acquire) == request_id {
                            ui.global::<Playlists>().set_roaming_loading(false);
                        }
                    });
                },
            )
            .await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if pu.roaming_request.load(Ordering::Acquire) != request_id {
                    return;
                }
                *pu.roaming_session.lock() = None;
                pu.roaming_feedback_pending.store(false, Ordering::Release);
                let playlists = ui.global::<Playlists>();
                playlists.set_roaming_loading(false);
                playlists.set_roaming_feedback_pending(false);
                playlists.set_roaming_mode(-1);
                if let Err(error) = result {
                    playlists.set_roaming_error(describe(&error).into());
                }
            });
        });
    });
}

fn wire_stop_roaming(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let pu = playlists_ui.clone();
    let s = state.clone();
    let weak = ui.as_weak();
    ui.global::<Playlists>().on_stop_roaming(move || {
        pu.roaming_request.fetch_add(1, Ordering::AcqRel);
        pu.roaming_cancel.lock().cancel();
        *pu.roaming_session.lock() = None;
        pu.roaming_feedback_pending.store(false, Ordering::Release);
        s.engine.stop();
        if let Some(ui) = weak.upgrade() {
            let playlists = ui.global::<Playlists>();
            playlists.set_roaming_mode(-1);
            playlists.set_roaming_loading(false);
            playlists.set_roaming_feedback_pending(false);
        }
    });
}

fn wire_dislike_roaming(ui: &AppWindow, state: &AppState, playlists_ui: &Arc<PlaylistsUi>) {
    let pu = playlists_ui.clone();
    let s = state.clone();
    let weak = ui.as_weak();
    ui.global::<Playlists>().on_dislike_roaming_current(move || {
        if pu
            .roaming_feedback_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let request_id = pu.roaming_request.load(Ordering::Acquire);
        let session = *pu.roaming_session.lock();
        let queue_generation = s.cloud_playback.queue_generation();
        let account_generation = s.cloud.status().generation;
        let song_id = {
            let player = lock_state(&s.player_state);
            current_personal_roaming_song(
                request_id,
                session,
                queue_generation,
                account_generation,
                &player,
            )
        };
        let Some(song_id) = song_id else {
            pu.roaming_feedback_pending.store(false, Ordering::Release);
            return;
        };
        if let Some(ui) = weak.upgrade() {
            let playlists = ui.global::<Playlists>();
            playlists.set_roaming_feedback_pending(true);
            playlists.set_roaming_error(slint::SharedString::default());
        }

        let weak_ui = weak.clone();
        let pu_task = pu.clone();
        let state_task = s.clone();
        s.runtime.clone().spawn(async move {
            let still_current = || {
                current_personal_roaming_song(
                    request_id,
                    *pu_task.roaming_session.lock(),
                    state_task.cloud_playback.queue_generation(),
                    state_task.cloud.status().generation,
                    &lock_state(&state_task.player_state),
                ) == Some(song_id)
            };
            let result = trash_then_skip(still_current, state_task.cloud.fm_trash(song_id), || {
                let mut skipped = false;
                state_task.playback_ctx().emit_and_execute(|player| {
                    if current_personal_roaming_song(
                        request_id,
                        *pu_task.roaming_session.lock(),
                        state_task.cloud_playback.queue_generation(),
                        state_task.cloud.status().generation,
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
            let _ = weak_ui.upgrade_in_event_loop(move |ui| {
                if pu_task.roaming_request.load(Ordering::Acquire) != request_id {
                    return;
                }
                pu_task.roaming_feedback_pending.store(false, Ordering::Release);
                let playlists = ui.global::<Playlists>();
                playlists.set_roaming_feedback_pending(false);
                if let Err(error) = result {
                    playlists.set_roaming_error(describe(&error).into());
                }
            });
        });
    });
}

fn current_personal_roaming_song(
    request_id: u64,
    session: Option<super::super::RoamingSession>,
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
mod tests {
    use super::{current_personal_roaming_song, trash_then_skip};
    use crate::ui::playlists::RoamingSession;
    use nekonetease_core::entities::cloud::RoamingMode;
    use nekonetease_core::entities::track::TrackSummary;
    use nekonetease_core::error::AppError;
    use nekonetease_engine::player::engine::queue::QueueState;
    use nekonetease_engine::player::engine::state::PlayerState;
    use nekonetease_engine::player::engine::types::PlaybackStatus;
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    const SONG_ID: i64 = 3_000_000_000;

    fn session(mode: RoamingMode) -> RoamingSession {
        RoamingSession { request_id: 7, mode, queue_generation: 11, account_generation: 5 }
    }

    fn track(id: i64, file_path: &str) -> Arc<TrackSummary> {
        Arc::new(TrackSummary {
            id,
            file_path: file_path.to_owned(),
            file_name: String::new(),
            title: format!("Song {id}"),
            artist: None,
            album: None,
            duration_ms: 180_000,
            artwork_path: None,
            track_number: None,
            disc_number: None,
            last_position: 0,
            is_favorite: false,
            rating: 0,
            replaygain_track_gain: None,
            replaygain_track_peak: None,
            replaygain_album_gain: None,
            replaygain_album_peak: None,
            stream: None,
            mv_id: None,
        })
    }

    fn player(source: Arc<TrackSummary>, queued: Arc<TrackSummary>) -> PlayerState {
        let mut player = PlayerState {
            status: PlaybackStatus::Playing,
            source: Some(source),
            ..PlayerState::default()
        };
        player.queue = QueueState::default();
        player.queue.add_tracks(vec![queued]);
        let _ = player.queue.skip_to_index(0);
        player
    }

    fn current_song(
        request: u64,
        roaming: Option<RoamingSession>,
        queue: u64,
        account: u64,
        player: &PlayerState,
    ) -> Option<i64> {
        current_personal_roaming_song(request, roaming, queue, account, player)
    }

    #[test]
    fn personal_fm_feedback_requires_the_same_live_cloud_track_and_session() {
        let source = track(SONG_ID, "");
        let mut player = player(source.clone(), source.clone());
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &player),
            Some(SONG_ID)
        );

        player.status = PlaybackStatus::Paused;
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &player),
            Some(SONG_ID),
            "paused playback is still the current song and may be marked not interested",
        );
    }

    #[test]
    fn personal_fm_feedback_rejects_stale_sessions_and_local_or_replaced_tracks() {
        let source = track(SONG_ID, "");
        let queued = track(SONG_ID, "");
        let player_state = player(source.clone(), queued.clone());
        assert_eq!(
            current_song(6, Some(session(RoamingMode::Personal)), 11, 5, &player_state),
            None
        );
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 12, 5, &player_state),
            None
        );
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 6, &player_state),
            None
        );
        assert_eq!(current_song(7, Some(session(RoamingMode::Heart)), 11, 5, &player_state), None);
        assert_eq!(current_song(7, None, 11, 5, &player_state), None);

        let local_source = player(track(SONG_ID, "/music/song.flac"), queued.clone());
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &local_source),
            None
        );
        let local_queue = player(source.clone(), track(SONG_ID, "/music/song.flac"));
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &local_queue),
            None
        );
        let replaced_queue = player(source, track(SONG_ID + 1, ""));
        assert_eq!(
            current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &replaced_queue),
            None
        );
    }

    #[tokio::test]
    async fn fm_feedback_skips_only_after_trash_succeeds() {
        let events = RefCell::new(Vec::new());
        let result = trash_then_skip(
            || true,
            async {
                events.borrow_mut().push("trash");
                Ok(())
            },
            || {
                events.borrow_mut().push("next");
                Ok(true)
            },
        )
        .await;

        assert!(matches!(result, Ok(true)));
        assert_eq!(*events.borrow(), ["trash", "next"]);
    }

    #[tokio::test]
    async fn fm_feedback_keeps_the_song_when_trash_fails() {
        let events = RefCell::new(Vec::new());
        let result = trash_then_skip(
            || true,
            async {
                events.borrow_mut().push("trash");
                Err(AppError::network_msg("trash rejected"))
            },
            || {
                events.borrow_mut().push("next");
                Ok(true)
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(*events.borrow(), ["trash"]);
    }

    #[tokio::test]
    async fn fm_feedback_does_not_advance_after_the_queue_changes_in_flight() {
        let checks = Cell::new(0);
        let events = RefCell::new(Vec::new());
        let result = trash_then_skip(
            || {
                checks.set(checks.get() + 1);
                checks.get() == 1
            },
            async {
                events.borrow_mut().push("trash");
                Ok(())
            },
            || {
                events.borrow_mut().push("next");
                Ok(true)
            },
        )
        .await;

        assert!(matches!(result, Ok(false)));
        assert_eq!(checks.get(), 2);
        assert_eq!(*events.borrow(), ["trash"]);
    }
}
