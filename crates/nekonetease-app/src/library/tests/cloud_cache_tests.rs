//! The cache's own bounds, over a temp root: what a key resolves to, when a lease ends, and what a
//! sweep retires. The network half is not exercised here — `cache_stream` only adds the download
//! and the trim to what these cover.

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::*;

fn store() -> AppResult<(TempDir, Paths)> {
    let tmp = TempDir::new()?;
    let paths = Paths::rooted_at(tmp.path().to_path_buf());
    paths.create_dirs()?;
    Ok((tmp, paths))
}

fn backdate(path: &Path, age: Duration) -> AppResult<()> {
    File::options().write(true).open(path)?.set_modified(SystemTime::now() - age)?;
    Ok(())
}

fn write(paths: &Paths, song_id: i64, quality: Quality, bytes: &[u8]) -> AppResult<PathBuf> {
    write_entry(&cache_dir(paths), song_id, quality, "mp3", Cursor::new(bytes), ENTRY_MAX_BYTES)
}

#[test]
fn an_entry_is_written_under_its_song_tier_and_format() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    let path = write_entry(
        &cache_dir(&paths),
        7,
        Quality::Lossless,
        "flac",
        Cursor::new(b"bytes"),
        ENTRY_MAX_BYTES,
    )?;

    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("7-lossless.flac"));
    assert_eq!(std::fs::read(&path)?, b"bytes");
    Ok(())
}

#[test]
fn a_fresh_entry_is_found_under_the_key_that_wrote_it() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, Quality::Exhigh, b"bytes")?;

    assert_eq!(cached_stream(&paths, 7, Quality::Exhigh), Some(path));
    assert_eq!(cached_stream(&paths, 7, Quality::Lossless), None);
    assert_eq!(cached_stream(&paths, 8, Quality::Exhigh), None);
    Ok(())
}

#[test]
fn a_directory_named_like_an_entry_is_not_served() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::create_dir_all(cache_dir(&paths).join("7-exhigh.mp3"))?;

    assert_eq!(cached_stream(&paths, 7, Quality::Exhigh), None);
    Ok(())
}

#[test]
fn an_entry_past_its_lease_is_not_served() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, Quality::Exhigh, b"bytes")?;

    backdate(&path, ENTRY_TTL + Duration::from_secs(1))?;

    assert_eq!(cached_stream(&paths, 7, Quality::Exhigh), None);
    Ok(())
}

#[test]
fn the_lease_is_measured_from_the_last_use() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, Quality::Exhigh, b"bytes")?;
    backdate(&path, ENTRY_TTL + Duration::from_secs(1))?;
    assert_eq!(cached_stream(&paths, 7, Quality::Exhigh), None);

    mark_used(&path)?;

    assert_eq!(cached_stream(&paths, 7, Quality::Exhigh), Some(path));
    Ok(())
}

#[test]
fn the_lease_lasts_until_exactly_the_ttl() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, Quality::Exhigh, b"bytes")?;
    let dir = cache_dir(&paths);
    let modified = path.metadata()?.modified()?;

    let just_before = modified + ENTRY_TTL - Duration::from_secs(1);
    assert!(entry_at(&dir, 7, Quality::Exhigh, just_before).is_some());
    assert!(entry_at(&dir, 7, Quality::Exhigh, modified + ENTRY_TTL).is_none());
    Ok(())
}

#[test]
fn a_body_over_the_cap_is_refused_and_leaves_no_entry() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let dir = cache_dir(&paths);

    let refused = write_entry(&dir, 7, Quality::Exhigh, "mp3", Cursor::new(b"012345678"), 8);

    assert!(matches!(refused, Err(AppError::Validation(_))));
    assert_eq!(std::fs::read_dir(&dir)?.count(), 0);
    Ok(())
}

#[test]
fn a_sweep_retires_expired_entries_and_counts_what_it_freed() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let expired = write(&paths, 1, Quality::Exhigh, b"1234")?;
    let fresh = write(&paths, 2, Quality::Exhigh, b"12")?;
    backdate(&expired, ENTRY_TTL + Duration::from_secs(1))?;

    let report = sweep_dir(&cache_dir(&paths), SystemTime::now(), u64::MAX, ENTRY_TTL)?;

    assert_eq!(report.removed, 1);
    assert_eq!(report.bytes_freed, 4);
    assert_eq!(report.remaining_bytes, 2);
    assert!(!expired.exists());
    assert!(fresh.exists());
    Ok(())
}

#[test]
fn a_sweep_evicts_the_oldest_entries_until_the_store_fits() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let oldest = write(&paths, 1, Quality::Exhigh, b"1234")?;
    let middle = write(&paths, 2, Quality::Exhigh, b"1234")?;
    let newest = write(&paths, 3, Quality::Exhigh, b"1234")?;
    backdate(&oldest, Duration::from_hours(3))?;
    backdate(&middle, Duration::from_hours(2))?;
    backdate(&newest, Duration::from_hours(1))?;

    let report = sweep_dir(&cache_dir(&paths), SystemTime::now(), 8, ENTRY_TTL)?;

    assert_eq!(report.removed, 1);
    assert_eq!(report.remaining_bytes, 8);
    assert!(!oldest.exists());
    assert!(middle.exists());
    assert!(newest.exists());
    Ok(())
}

#[test]
fn a_sweep_retires_only_names_it_wrote() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let foreign = cache_dir(&paths).join("notes.txt");
    std::fs::create_dir_all(cache_dir(&paths))?;
    std::fs::write(&foreign, b"kept")?;
    backdate(&foreign, ENTRY_TTL + Duration::from_secs(1))?;

    let report = sweep_dir(&cache_dir(&paths), SystemTime::now(), 0, ENTRY_TTL)?;

    assert_eq!(report.removed, 0);
    assert_eq!(report.remaining_bytes, 0);
    assert!(foreign.exists());
    Ok(())
}

#[test]
fn a_cache_that_does_not_exist_yet_is_an_empty_sweep() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    assert_eq!(sweep(&paths)?, SweepReport::default());
    Ok(())
}

#[test]
fn a_cache_path_that_is_not_a_directory_is_a_failure() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::write(cache_dir(&paths), b"")?;

    assert!(sweep(&paths).is_err());
    Ok(())
}
