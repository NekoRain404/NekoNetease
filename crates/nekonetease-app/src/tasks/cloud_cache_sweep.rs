//! Holds both cloud stream caches to their expiry and their byte caps.
//!
//! The write paths trim their caps after every download, so what is left for this pass is age: an
//! entry downloaded months ago and never played again. That happens while nothing is being
//! written, which is why this is a periodic pass rather than a write-time hook — a listener who
//! streams one album a week would otherwise keep its predecessors until a download evicted them,
//! and a listener who has stopped would keep them indefinitely.
//!
//! The video store gets the same cadence for the same reason. One task rather than a second loop:
//! the two passes answer one question about one data root, and a failure on either is logged
//! without stopping the other.

use std::sync::Arc;
use std::time::Duration;

use crate::library;
use crate::state::AppState;
use crate::tasks::TaskSpawner;
use nekonetease_core::config::Paths;
use nekonetease_core::error::{AppError, AppResult, describe};

/// Six hours. Long enough that a session which plays all day runs the listing a handful of times,
/// short enough that a cache filled this morning gives its space back before the day is out.
const SWEEP_INTERVAL: Duration = Duration::from_hours(6);

pub fn spawn(spawner: &TaskSpawner, state: &AppState) {
    let paths = Arc::clone(&state.paths);
    spawner.spawn_cancellable(move |shutdown| async move {
        loop {
            match run(Arc::clone(&paths)).await {
                Ok(report) if report.removed > 0 => log::info!(
                    "Cloud cache: retired {} entr(ies), {} KiB reclaimed",
                    report.removed,
                    report.bytes_freed / 1024
                ),
                Ok(_) => {}
                Err(e) => log::warn!("Cloud cache sweep failed: {}", describe(&e)),
            }
            match run_video(Arc::clone(&paths)).await {
                Ok(report) if report.removed > 0 => log::info!(
                    "MV cache: retired {} entr(ies), {} KiB reclaimed",
                    report.removed,
                    report.bytes_freed / 1024
                ),
                Ok(_) => {}
                Err(e) => log::warn!("MV cache sweep failed: {}", describe(&e)),
            }
            if shutdown.run_until_cancelled(tokio::time::sleep(SWEEP_INTERVAL)).await.is_none() {
                break;
            }
        }
    });
}

/// Narrowed off `spawn` so a pass's report is observable; a `JoinError` reads as a failed pass
/// rather than as its own arm, since the caller does nothing different either way.
async fn run(paths: Arc<Paths>) -> AppResult<library::cloud_cache::SweepReport> {
    tokio::task::spawn_blocking(move || library::cloud_cache::sweep(&paths))
        .await
        .map_err(AppError::io_source)?
}

/// The video store's pass, kept beside `run` rather than folded into it: the two stores answer
/// with their own report types and one sweep may fail where the other does not.
async fn run_video(paths: Arc<Paths>) -> AppResult<library::mv_cache::SweepReport> {
    tokio::task::spawn_blocking(move || library::mv_cache::sweep(&paths))
        .await
        .map_err(AppError::io_source)?
}

#[cfg(test)]
#[path = "tests/cloud_cache_sweep_tests.rs"]
mod tests;
