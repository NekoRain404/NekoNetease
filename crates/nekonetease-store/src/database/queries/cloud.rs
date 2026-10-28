//! Queries behind the two cloud bridges: matching a cloud-disk song to a local
//! track, and reconciling a cloud playlist into a local one.
//!
//! The match answers with the four columns a comparison needs rather than a
//! `TrackListRow`, which would decode a list view's artwork fallback per row.
//! The playlist link keeps its last-synced membership relationally rather than
//! as JSON, so this side of the boundary needs no serializer.

use sqlx::AssertSqlSafe;

use crate::database::queries::playlist::{
    insert_items_tx, renumber_playlist_positions_tx, update_playlist_thumbnail_and_timestamp_tx,
};
use crate::database::{DbPool, MAX_BINDS_PER_STATEMENT, placeholders};
use nekonetease_core::error::AppError;

/// One local track's matching key, narrowed to what the comparison reads.
#[derive(Debug, sqlx::FromRow)]
pub struct MatchRow {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub duration_ms: i64,
}

/// Every local track's matching key, in whatever order the table scan returns.
pub async fn match_candidates(db: &DbPool) -> Result<Vec<MatchRow>, AppError> {
    let rows = sqlx::query_as::<_, MatchRow>("SELECT id, title, artist, duration_ms FROM tracks")
        .fetch_all(db.read())
        .await?;
    Ok(rows)
}

/// A playlist's track ids in playlist order.
pub async fn playlist_track_ids(db: &DbPool, playlist_id: i64) -> Result<Vec<i64>, AppError> {
    let ids = sqlx::query_scalar::<_, i64>(
        "SELECT track_id FROM playlist_items WHERE playlist_id = ? ORDER BY position, id",
    )
    .bind(playlist_id)
    .fetch_all(db.read())
    .await?;
    Ok(ids)
}

/// The local playlist a cloud playlist was last reconciled into, and the two
/// names at that point. `None` means this cloud playlist has never been applied.
///
/// Both names are kept because either side can rename between syncs: with one
/// base, a playlist linked under its existing local name looks locally renamed
/// on every later pass.
#[derive(Debug, sqlx::FromRow)]
pub struct PlaylistLink {
    pub local_playlist_id: i64,
    pub cloud_name: String,
    pub local_name: String,
}

pub async fn playlist_link(
    db: &DbPool,
    cloud_playlist_id: i64,
) -> Result<Option<PlaylistLink>, AppError> {
    let link = sqlx::query_as::<_, PlaylistLink>(
        "SELECT local_playlist_id, cloud_name, local_name FROM cloud_playlist_links \
         WHERE cloud_playlist_id = ?",
    )
    .bind(cloud_playlist_id)
    .fetch_optional(db.read())
    .await?;
    Ok(link)
}

/// The cloud membership recorded by the last apply, in cloud order.
pub async fn playlist_link_track_ids(
    db: &DbPool,
    cloud_playlist_id: i64,
) -> Result<Vec<i64>, AppError> {
    let ids = sqlx::query_scalar::<_, i64>(
        "SELECT local_track_id FROM cloud_playlist_link_tracks \
         WHERE cloud_playlist_id = ? ORDER BY position, local_track_id",
    )
    .bind(cloud_playlist_id)
    .fetch_all(db.read())
    .await?;
    Ok(ids)
}

/// What one reviewed sync asks the store to write.
#[derive(Clone, Copy)]
pub struct CloudPlaylistSync<'a> {
    pub cloud_playlist_id: i64,
    pub local_playlist_id: i64,
    pub rename_to: Option<&'a str>,
    pub remove_track_ids: &'a [i64],
    pub add_track_ids: &'a [i64],
    pub cloud_name: &'a str,
    /// The local playlist's name once this sync commits, which the next diff
    /// compares against to tell a local rename from one it has already seen.
    pub local_name: &'a str,
    pub cloud_track_ids: &'a [i64],
}

/// Apply a reviewed sync plan in one transaction: rename, remove, add, then
/// record the cloud snapshot this pass reconciled against.
///
/// The snapshot is the *cloud* membership, not what the local playlist ended
/// up holding. A local-only track the plan kept is therefore absent from it,
/// which is what makes the next diff read it as a local addition instead of a
/// cloud removal. The names follow the same rule, one per side.
pub async fn apply_cloud_playlist_sync(
    db: &DbPool,
    sync: &CloudPlaylistSync<'_>,
) -> Result<(), AppError> {
    let sync = *sync;
    let CloudPlaylistSync {
        cloud_playlist_id,
        local_playlist_id,
        rename_to,
        remove_track_ids,
        add_track_ids,
        cloud_name,
        local_name,
        cloud_track_ids,
    } = sync;
    let mut tx = db.write().begin().await?;

    let is_smart: Option<bool> = sqlx::query_scalar("SELECT is_smart FROM playlists WHERE id = ?")
        .bind(local_playlist_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(is_smart) = is_smart else {
        return Err(AppError::not_found("Playlist", local_playlist_id));
    };
    if is_smart {
        return Err(AppError::Validation(
            "a smart playlist resolves its own membership and cannot be synced".to_owned(),
        ));
    }

    // A local playlist pairs with one cloud playlist. Re-pointing it silently
    // would leave the old cloud side writing into a snapshot nobody reads.
    let linked: Option<i64> = sqlx::query_scalar(
        "SELECT cloud_playlist_id FROM cloud_playlist_links WHERE local_playlist_id = ?",
    )
    .bind(local_playlist_id)
    .fetch_optional(&mut *tx)
    .await?;
    if linked.is_some_and(|linked| linked != cloud_playlist_id) {
        return Err(AppError::Validation(
            "this local playlist is already paired with another cloud playlist".to_owned(),
        ));
    }

    let now = nekonetease_core::utils::now_rfc3339();
    if let Some(name) = rename_to {
        sqlx::query("UPDATE playlists SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(&now)
            .bind(local_playlist_id)
            .execute(&mut *tx)
            .await?;
    }

    delete_tracks(&mut tx, local_playlist_id, remove_track_ids).await?;

    if !add_track_ids.is_empty() {
        let max_position: Option<i32> =
            sqlx::query_scalar("SELECT MAX(position) FROM playlist_items WHERE playlist_id = ?")
                .bind(local_playlist_id)
                .fetch_one(&mut *tx)
                .await?;
        let start = max_position.unwrap_or(-1) + 1;
        insert_items_tx(&mut tx, local_playlist_id, start, add_track_ids).await?;
    }

    renumber_playlist_positions_tx(&mut tx, local_playlist_id).await?;
    update_playlist_thumbnail_and_timestamp_tx(&mut tx, local_playlist_id).await?;
    sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(local_playlist_id)
        .execute(&mut *tx)
        .await?;

    store_link(
        &mut tx,
        cloud_playlist_id,
        local_playlist_id,
        cloud_name,
        local_name,
        cloud_track_ids,
        &now,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn delete_tracks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist_id: i64,
    track_ids: &[i64],
) -> Result<(), AppError> {
    // One bind is the playlist id, so each chunk keeps a slot of the budget for
    // it.
    for chunk in track_ids.chunks(MAX_BINDS_PER_STATEMENT - 1) {
        let placeholders = placeholders(chunk.len());
        let sql = format!(
            "DELETE FROM playlist_items WHERE playlist_id = ? AND track_id IN ({placeholders})"
        );
        let mut query = sqlx::query(AssertSqlSafe(sql)).persistent(false).bind(playlist_id);
        for id in chunk {
            query = query.bind(*id);
        }
        query.execute(&mut **tx).await?;
    }
    Ok(())
}

/// Three binds per snapshot row: the cloud id, its position and the local id.
const COLS_PER_SNAPSHOT_ROW: usize = 3;
const SNAPSHOT_CHUNK: usize = MAX_BINDS_PER_STATEMENT / COLS_PER_SNAPSHOT_ROW;

async fn store_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cloud_playlist_id: i64,
    local_playlist_id: i64,
    cloud_name: &str,
    local_name: &str,
    cloud_track_ids: &[i64],
    now: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO cloud_playlist_links \
             (cloud_playlist_id, local_playlist_id, cloud_name, local_name, synced_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(cloud_playlist_id) DO UPDATE SET \
             local_playlist_id = excluded.local_playlist_id, \
             cloud_name = excluded.cloud_name, \
             local_name = excluded.local_name, \
             synced_at = excluded.synced_at",
    )
    .bind(cloud_playlist_id)
    .bind(local_playlist_id)
    .bind(cloud_name)
    .bind(local_name)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM cloud_playlist_link_tracks WHERE cloud_playlist_id = ?")
        .bind(cloud_playlist_id)
        .execute(&mut **tx)
        .await?;

    for (chunk_index, chunk) in cloud_track_ids.chunks(SNAPSHOT_CHUNK).enumerate() {
        let offset = chunk_index * SNAPSHOT_CHUNK;
        let mut query_builder = sqlx::QueryBuilder::new(
            "INSERT INTO cloud_playlist_link_tracks \
                 (cloud_playlist_id, position, local_track_id) ",
        );
        query_builder.push_values(chunk.iter().enumerate(), |mut row, (index, track_id)| {
            let position = i64::try_from(offset + index).unwrap_or(i64::MAX);
            row.push_bind(cloud_playlist_id).push_bind(position).push_bind(*track_id);
        });
        query_builder.build().persistent(false).execute(&mut **tx).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/cloud_tests.rs"]
mod tests;
