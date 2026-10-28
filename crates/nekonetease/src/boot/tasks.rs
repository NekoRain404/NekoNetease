//! Background-task spawning, queue restore, and what the two ways of starting
//! resolve to: a restored queue left paused, or the files this launch was handed.

use std::sync::Arc;

use slint::ComponentHandle;

use nekonetease_app::state::{AppState, StartupChannels};
use nekonetease_app::{library, tasks};
use nekonetease_core::error::describe;
use nekonetease_core::utils;
use nekonetease_engine::player::engine::event_sink::EventSink;
use nekonetease_integrations::services::integrations;
use nekonetease_platform::services::platform;
use nekonetease_ui::AppWindow;
use nekonetease_views::ui;

/// Spawn every always-running background task and the OS media-control event
/// receiver. Consumes `channels`.
pub fn spawn_background_tasks(
    spawner: &tasks::TaskSpawner,
    state: &AppState,
    mut channels: StartupChannels,
) {
    let cloud = state.cloud.clone();
    spawner.spawn_cancellable(move |shutdown| async move {
        // The schedule and its ceiling live with the session, where they are unit-tested.
        cloud.restore_session_with_retry(&shutdown).await;
    });

    tasks::playback_monitor::spawn(spawner, state);
    tasks::netease_listening::spawn(spawner, state);
    // Playback trims the offline stream cache on every write; this reclaims what the listener
    // stopped touching, without needing a playback to trigger it.
    tasks::cloud_cache_sweep::spawn(spawner, state);
    tasks::file_event_processor::spawn(spawner, state, channels.file_event_rx);
    tasks::queue_prune::spawn(spawner, state);
    tasks::retroactive_hash::spawn(spawner, state);
    // Puts back a stored cover a cache finds missing: mid-session, and with watching off, where no
    // boot scan reconciles the library, one gone while the app was closed too.
    tasks::artwork_restore::spawn(spawner, state);
    // Every other sweep follows a scan, and a launch with no library folders runs none, so the
    // covers of a library removed last session would otherwise stay on disk.
    tasks::artwork_sweep::spawn(spawner, state);
    // One-shot, and gated on its own settings marker rather than on anything here.
    tasks::artwork_renormalize::spawn(spawner, state);
    // The other one-shot: reads the ratings a library already carried in before the scan
    // learned to look for them, which no rescan can reach.
    tasks::rating_import::spawn(spawner, state);
    // And the third: marks a library indexed before the ingest widened as stale, so the boot scan
    // re-reads the tags the migrations could only seed from what the database already knew.
    tasks::tag_backfill::spawn(spawner, state);
    // Carries a star back out into the file. Before any callback can set one.
    tasks::rating_writeback::spawn(spawner, state);
    tasks::heap_trim::spawn(spawner);
    // Folds the output device's fault counters into one line per window, and is
    // the only thing watching for a device that goes away mid-session.
    tasks::audio_health::spawn(spawner, state);
    // Follows the system moving a device whose own control carries the volume. Inert until a
    // claim with hardware volume reports a move.
    tasks::device_volume::spawn(spawner, state);
    // Batches `play_count` / `skip_count` UPDATEs so a fast skip burst is one
    // write. Before any playback can fire an `UpdatePlayCount`.
    tasks::play_count_flusher::spawn(spawner, state.db.clone(), state.stats_changed.clone());
    // OS media-control events drive the same `library::*` paths the UI does,
    // keeping MPRIS / SMTC in lockstep with it.
    if let Some(rx) = channels.media_control_rx.take() {
        let sink: Arc<dyn EventSink> =
            Arc::new(ui::shell::event_sink::SlintEventSink { state: state.clone() });
        integrations::media_controls::spawn_event_receiver(
            &state.task_tracker,
            state.shutdown_token.clone(),
            rx,
            sink,
        );
        log::info!("Media controls event receiver started");
    }
}

/// Restore the persisted queue from disk (best-effort; missing file is OK).
pub fn restore_persisted_playback(runtime: &tokio::runtime::Runtime, state: &AppState) {
    if let Err(e) = runtime.block_on(library::queue::restore_persisted_playback(state)) {
        log::warn!("Failed to restore persisted playback: {}", describe(&e));
    }
}

/// Play the files this launch was handed on the command line.
///
/// After [`restore_persisted_playback`] so an opened track wins over what the
/// last session left. A normal launch leaves the restored track paused.
/// Synchronous like the restore — the first frame should paint what was
/// double-clicked.
pub fn open_startup_files(
    runtime: &tokio::runtime::Runtime,
    state: &AppState,
    files: &[std::path::PathBuf],
) {
    let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    // Too early to toast: that bridge installs with the UI and drops rather
    // than queues what arrives before it.
    if let Err(e) = runtime.block_on(library::queue::open_files(state, paths)) {
        log::warn!("Failed to open the files given on the command line: {}", describe(&e));
    }
}

/// Start accepting forwarded launches. The listener has been bound since before
/// the logger opened, so what waited in the backlog is delivered from here.
pub fn serve_file_opens(
    state: &AppState,
    app: &AppWindow,
    listener: platform::single_instance::Listener,
) {
    let state = state.clone();
    let weak = app.as_weak();

    platform::single_instance::serve(listener, move |paths| {
        // Raise either way — an empty forward is someone launching NekoNetease
        // again to get at the window.
        let _ = weak.upgrade_in_event_loop(|ui| ui::shell::tray_bridge::raise_window(&ui));

        if paths.is_empty() {
            return;
        }
        let state = state.clone();
        state.runtime.clone().spawn(async move {
            if let Err(e) = library::queue::open_files(&state, paths).await {
                log::warn!("Failed to open forwarded files: {}", describe(&e));
                utils::toast::notify(utils::toast::ToastKind::OperationFailed, e.to_string());
            }
        });
    });
}

/// Folder-watcher restart and the launch reconcile (off-thread, tracked so
/// shutdown can await its DB writes before dropping the runtime).
pub fn spawn_resume_watching(spawner: &tasks::TaskSpawner, state: &AppState) {
    let state_for_init = state.clone();
    spawner.spawn(async move {
        if let Err(e) = tasks::resume_watching::run(&state_for_init).await {
            log::warn!("Resuming folder watching failed: {}", describe(&e));
        }
    });
}
