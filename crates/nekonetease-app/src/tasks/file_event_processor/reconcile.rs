//! Apply a deduplicated batch of file events to the database: extract
//! metadata for created/modified/renamed paths, then hand the batch to
//! [`queries::scan::apply_watch_batch`], which writes the track rows.
//!
//! A batch also carries whatever a previous one could not apply. Those changes are replayed
//! first, but a fresh event for the same path supersedes them: it describes the file as it is
//! now, and replaying the older change beside it can only undo the newer one.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nekonetease_artwork::media::image::artwork::CoverCache;
use nekonetease_core::config::Paths;
use nekonetease_core::entities::scan::ExtractedMetadata;
use nekonetease_core::error::{AppResult, describe};
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries;
use nekonetease_store::database::queries::scan::{FailedChange, WatchedChange};
use nekonetease_store::media::ingest::metadata::{extract_date_modified, extract_or_filename_row};
use nekonetease_store::media::ingest::scan_pool::ScanPool;
use nekonetease_store::media::ingest::watcher::FileEvent;

use super::{MAX_ATTEMPTS, Retry, change_key};

/// Extract metadata for all paths that need it (Created, Modified, Renamed-to).
/// Runs outside any DB transaction to avoid holding the write lock during I/O.
async fn extract_metadata_batch(
    paths: &Paths,
    cover_cache: &CoverCache,
    events: &[FileEvent],
) -> HashMap<PathBuf, ExtractedMetadata> {
    let artwork_dir = paths.artwork_dir.clone();

    let mut seen = HashSet::with_capacity(events.len());
    let mut paths_to_extract: Vec<PathBuf> = Vec::with_capacity(events.len());
    for event in events {
        match event {
            FileEvent::Created(path) | FileEvent::Modified(path) => {
                if path.exists() && seen.insert(path.clone()) {
                    paths_to_extract.push(path.clone());
                }
            }
            FileEvent::Renamed { to, .. } => {
                if to.exists() && seen.insert(to.clone()) {
                    paths_to_extract.push(to.clone());
                }
            }
            FileEvent::Removed(_) => {}
            // Caller short-circuits on RescanNeeded before reaching here.
            FileEvent::RescanNeeded => unreachable!(),
        }
    }

    if paths_to_extract.is_empty() {
        return HashMap::new();
    }

    // One blocking task wrapping Rayon file-level parallelism — the same
    // shape as `scan_files_parallel`. A bulk drop into a watched folder can
    // produce thousands of Created events in one batch; fanning out one
    // `spawn_blocking` per file would burst toward tokio's blocking-thread
    // cap and thrash the disk with hundreds of concurrent readers, while
    // Rayon bounds concurrency to the core count.
    let cover_cache = cover_cache.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;
        ScanPool::for_files(paths_to_extract.len()).install(|| {
            paths_to_extract
                .into_par_iter()
                .filter_map(|path| {
                    match extract_or_filename_row(&path, &artwork_dir, &cover_cache, false) {
                        Ok(meta) => Some((path, meta)),
                        // Only an unreadable file gets this far; unparseable tags come back
                        // as a filename-derived row rather than a `None`.
                        Err(e) => {
                            log::warn!(
                                "Skipping {}: {}",
                                path.display(),
                                nekonetease_core::error::describe(&e)
                            );
                            None
                        }
                    }
                })
                .collect::<HashMap<_, _>>()
        })
    })
    .await;

    match extracted {
        Ok(results) => results,
        Err(e) => {
            log::warn!("Metadata extraction task panicked: {}", describe(&e));
            HashMap::new()
        }
    }
}

/// Processes a deduplicated batch of file events and the changes earlier batches could not apply.
/// Returns what still failed, each change's attempt count advanced, for the caller to hold.
pub(super) async fn process_batch(
    db: &DbPool,
    paths: &Paths,
    cover_cache: &CoverCache,
    events: Vec<FileEvent>,
    pending: Vec<Retry>,
) -> AppResult<Vec<Retry>> {
    let metadata = extract_metadata_batch(paths, cover_cache, &events).await;
    let fresh = watched_changes(events, metadata);

    let fresh_keys: HashSet<PathBuf> = fresh.iter().map(change_key).collect();
    let mut attempts: HashMap<PathBuf, u32> = HashMap::with_capacity(pending.len());
    let mut changes = Vec::with_capacity(fresh.len() + pending.len());
    for retry in pending {
        let key = change_key(&retry.change);
        // A fresh event for a path describes the file as it is now; replaying the older change
        // beside it can only undo the newer one.
        if fresh_keys.contains(&key) {
            continue;
        }
        attempts.insert(key, retry.attempts);
        changes.push(retry.change);
    }
    changes.extend(fresh);

    let failed = queries::scan::apply_watch_batch(db, changes).await?;
    Ok(collect_retries(failed, &attempts))
}

/// Advances each failure's attempt count, dropping the change once [`MAX_ATTEMPTS`] is spent so a
/// persistently failing file is left for the next full scan rather than replayed forever. The
/// first failure and the give-up are the only warnings: a retry failing again is expected, and
/// repeating its whole warning every batch is what buries the one that matters.
fn collect_retries(failed: Vec<FailedChange>, attempts: &HashMap<PathBuf, u32>) -> Vec<Retry> {
    let mut retries = Vec::with_capacity(failed.len());
    for FailedChange { change, error } in failed {
        let key = change_key(&change);
        let attempts = attempts.get(&key).copied().unwrap_or(0).saturating_add(1);
        if attempts >= MAX_ATTEMPTS {
            log::warn!(
                "Giving up on watcher event for {} after {attempts} attempts: {}. Its row is left \
                 for the next full scan to reconcile.",
                key.display(),
                describe(&error)
            );
            continue;
        }
        if attempts == 1 {
            log::warn!(
                "Could not apply watcher event for {}; retrying on the next batch: {}",
                key.display(),
                describe(&error)
            );
        } else {
            log::debug!(
                "Attempt {attempts} of {MAX_ATTEMPTS} for {} failed: {}",
                key.display(),
                describe(&error)
            );
        }
        retries.push(Retry { change, attempts });
    }
    retries
}

/// Pairs each event with what extraction read, dropping a create or a modify whose file could not
/// be read. Dedup leaves one event per path, so each entry is moved out of `metadata` rather than
/// cloned.
fn watched_changes(
    events: Vec<FileEvent>,
    mut metadata: HashMap<PathBuf, ExtractedMetadata>,
) -> Vec<WatchedChange> {
    events
        .into_iter()
        .filter_map(|event| match event {
            FileEvent::Created(path) => {
                let meta = metadata.remove(&path)?;
                Some(WatchedChange::Created { path, meta })
            }
            FileEvent::Modified(path) => {
                let meta = metadata.remove(&path)?;
                Some(WatchedChange::Modified { path, meta })
            }
            FileEvent::Renamed { from, to } => {
                let meta = metadata.remove(&to);
                // `WatchedChange::Renamed` argues why `meta`'s mtime wins. The `stat` is for the
                // one case with nothing in hand: extraction failed, or `to` vanished first.
                let date_modified = meta
                    .as_ref()
                    .and_then(|meta| meta.date_modified.clone())
                    .or_else(|| extract_date_modified(&to));
                Some(WatchedChange::Renamed { from, to, meta, date_modified })
            }
            FileEvent::Removed(path) => Some(WatchedChange::Removed(path)),
            // Caller short-circuits on RescanNeeded before reaching here.
            FileEvent::RescanNeeded => unreachable!(),
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/reconcile_tests.rs"]
mod tests;
