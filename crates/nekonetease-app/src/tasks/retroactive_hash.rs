//! One-shot background task that hashes every track row whose
//! `file_hash` column is still NULL. Safe to invoke on every startup
//! (no-op when the column is already populated) and after a folder
//! add (new rows ingested without hashes get backfilled).

use std::path::Path;

use crate::state::AppState;
use crate::tasks::TaskSpawner;
use nekonetease_core::error::{AppResult, describe};
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries;
use nekonetease_store::media::ingest::scan_pool::ScanPool;

/// Spawn the retroactive-hash task on the shared task lifecycle so the
/// main shutdown sequence waits for the pending batch update to commit
/// before the runtime is torn down.
pub fn spawn(spawner: &TaskSpawner, state: &AppState) {
    let db = state.db.clone();
    spawner.spawn(async move {
        if let Err(e) = hash_unhashed_tracks(&db).await {
            log::warn!("Background hashing failed: {}", describe(&e));
        }
    });
    log::info!("Retroactive hash task started");
}

/// How many files are hashed before their rows are committed. The batch bounds the peak rather
/// than the batch *cost*: without it a first launch over a large collection holds every path and
/// hash until the last file, and a quit part way keeps none of the work.
const HASH_BATCH: usize = 256;

/// Hash all tracks in the database that are missing a `file_hash`, one batch at a time.
async fn hash_unhashed_tracks(db: &DbPool) -> AppResult<()> {
    let unhashed = queries::track::get_unhashed_track_paths(db).await?;
    if unhashed.is_empty() {
        return Ok(());
    }

    log::info!("Starting retroactive hashing for {} tracks", unhashed.len());

    let mut hashed = 0_usize;
    for batch in unhashed.chunks(HASH_BATCH) {
        let batch = batch.to_vec();
        let updates = tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;

            ScanPool::for_files(batch.len()).install(|| {
                batch
                    .par_iter()
                    .filter_map(|(id, path_str)| {
                        let path = Path::new(path_str);
                        // One `stat` answers both "is it still there" and "when was it last
                        // written" — an absent file fails here exactly as the old
                        // `path.exists()` check did, and the mtime comes from the same
                        // instant as that existence proof.
                        let Ok(meta) = std::fs::metadata(path) else {
                            log::debug!(
                                "Skipping missing file during retroactive hash: {path_str}"
                            );
                            return None;
                        };

                        let hash =
                            match nekonetease_store::media::ingest::metadata::compute_file_hash(
                                path,
                            ) {
                                Ok(h) => h,
                                Err(e) => {
                                    log::warn!("Failed to hash {path_str}: {}", describe(&e));
                                    return None;
                                }
                            };

                        let mtime =
                            nekonetease_store::media::ingest::metadata::date_modified_from_metadata(
                                &meta,
                            );

                        Some((*id, hash, mtime))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .await
        .map_err(|e| nekonetease_core::error::AppError::scanner("Hashing task panicked", e))?;

        hashed += updates.len();
        if !updates.is_empty() {
            queries::track::batch_update_hashes(db, &updates).await?;
        }
    }

    log::info!("Retroactive hashing complete: {hashed} files hashed");
    Ok(())
}

#[cfg(test)]
#[path = "tests/retroactive_hash_tests.rs"]
mod tests;
