//! The periodic save, which is what survives a SIGKILL.
//!
//! Persistence is deliberately off the `PlayerAction` list, so nothing else in the tree writes
//! what this does: the resume position onto the row, and the queue into `queue.json`. Every
//! failure inside it is a warning, so a half that stops writing costs a
//! user their place and reports nothing at all.

use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use nekonetease_core::error::AppError;
use nekonetease_engine::player::engine::types::{PersistableQueue, PersistedPlayback};
use nekonetease_store::database::queries::fixtures::insert_test_track;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A library with one track row, and the `queue.json` the monitor writes beside it.
struct Fixture {
    db: DbPool,
    queue_path: PathBuf,
    track_id: i64,
    _tmp: TempDir,
}

impl Fixture {
    async fn new() -> Result<Self, AppError> {
        let tmp = TempDir::new()?;
        let db = DbPool::test_pool().await?;
        queries::folder::insert_folder(&db, "/music", true).await?;
        let track_id =
            insert_test_track(&db, "/music/song.mp3", "Song", "Artist", "Album", "Rock").await?;
        Ok(Self { db, queue_path: tmp.path().join("queue.json"), track_id, _tmp: tmp })
    }

    async fn last_position(&self) -> Result<i64, AppError> {
        let position =
            sqlx::query_scalar::<_, i64>("SELECT last_position FROM tracks WHERE id = ?")
                .bind(self.track_id)
                .fetch_one(self.db.read())
                .await?;
        Ok(position)
    }

    /// Read the file back the way the startup restore does, so a shape only this test can parse
    /// would fail here too.
    fn written_playback(&self) -> TestResult<PersistedPlayback> {
        Ok(serde_json::from_str(&std::fs::read_to_string(&self.queue_path)?)?)
    }
}

/// A queue of one, over the fixture's own row.
fn playback(track_id: i64) -> PersistedPlayback {
    PersistedPlayback {
        queue: PersistableQueue { track_ids: vec![track_id], current_index: 0 },
        playlist_tracks: Vec::new(),
        playlist_current_index: -1,
        current_position_ms: None,
    }
}

/// The resume position, which is the only thing that puts a user back where they were after a
/// crash — the clean-exit snapshot never ran.
#[tokio::test]
async fn the_snapshot_position_lands_on_the_track_row() -> TestResult {
    let fixture = Fixture::new().await?;

    persist(
        &fixture.db,
        &fixture.queue_path,
        PlaybackSnapshot {
            track: Some((fixture.track_id, 42_000)),
            playback: playback(fixture.track_id),
        },
    )
    .await;

    assert_eq!(fixture.last_position().await?, 42_000);
    Ok(())
}

/// A tick can land with nothing playing while the queue is still seated. The row write is guarded
/// on a playing track and the file write is not, because a restart restores the queue either way.
#[tokio::test]
async fn a_snapshot_with_nothing_playing_still_writes_the_queue_file() -> TestResult {
    let fixture = Fixture::new().await?;

    persist(
        &fixture.db,
        &fixture.queue_path,
        PlaybackSnapshot { track: None, playback: playback(fixture.track_id) },
    )
    .await;

    assert_eq!(fixture.last_position().await?, 0, "there was no track to write a position for");
    assert_eq!(fixture.written_playback()?.queue.track_ids, [fixture.track_id]);
    Ok(())
}
