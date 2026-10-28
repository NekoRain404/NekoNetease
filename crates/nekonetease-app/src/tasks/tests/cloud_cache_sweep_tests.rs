//! What the periodic pass answers: an untouched store is quiet, an aged entry leaves, and a cache
//! path that cannot be read is reported rather than counted as empty.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::*;
use nekonetease_core::entities::cloud::Quality;

fn store() -> AppResult<(TempDir, Arc<Paths>)> {
    let tmp = TempDir::new()?;
    let paths = Paths::rooted_at(tmp.path().to_path_buf());
    paths.create_dirs()?;
    Ok((tmp, Arc::new(paths)))
}

fn write_entry(paths: &Paths, song_id: i64) -> AppResult<std::path::PathBuf> {
    library::cloud_cache::write_entry(
        &library::cloud_cache::cache_dir(paths),
        song_id,
        Quality::Exhigh,
        "mp3",
        Cursor::new(b"bytes"),
        library::cloud_cache::ENTRY_MAX_BYTES,
    )
}

#[tokio::test]
async fn a_cache_with_nothing_cached_sweeps_quietly() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    assert_eq!(run(paths).await?, library::cloud_cache::SweepReport::default());
    Ok(())
}

#[tokio::test]
async fn an_entry_past_its_lease_is_retired() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let entry = write_entry(&paths, 7)?;
    std::fs::File::options().write(true).open(&entry)?.set_modified(
        SystemTime::now() - library::cloud_cache::ENTRY_TTL - Duration::from_hours(1),
    )?;

    let report = run(Arc::clone(&paths)).await?;

    assert_eq!(report.removed, 1);
    assert!(!entry.exists());
    Ok(())
}

#[tokio::test]
async fn a_cache_path_that_cannot_be_read_is_a_failure() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::write(library::cloud_cache::cache_dir(&paths), b"")?;

    assert!(run(paths).await.is_err());
    Ok(())
}

fn write_video_entry(paths: &Paths, mv_id: i64) -> AppResult<std::path::PathBuf> {
    library::mv_cache::write_entry(
        &library::mv_cache::cache_dir(paths),
        mv_id,
        1080,
        "mp4",
        Cursor::new(b"bytes"),
        library::mv_cache::ENTRY_MAX_BYTES,
    )
}

#[tokio::test]
async fn a_video_cache_with_nothing_cached_sweeps_quietly() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    assert_eq!(run_video(paths).await?, library::mv_cache::SweepReport::default());
    Ok(())
}

#[tokio::test]
async fn a_video_entry_past_its_lease_is_retired() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let entry = write_video_entry(&paths, 7)?;
    std::fs::File::options()
        .write(true)
        .open(&entry)?
        .set_modified(SystemTime::now() - library::mv_cache::ENTRY_TTL - Duration::from_hours(1))?;

    let report = run_video(Arc::clone(&paths)).await?;

    assert_eq!(report.removed, 1);
    assert!(!entry.exists());
    Ok(())
}

#[tokio::test]
async fn an_unreadable_video_cache_path_is_a_failure() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::write(library::mv_cache::cache_dir(&paths), b"")?;

    assert!(run_video(paths).await.is_err());
    Ok(())
}
