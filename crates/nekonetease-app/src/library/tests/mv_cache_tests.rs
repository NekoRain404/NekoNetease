//! The video cache's own bounds, over a temp root: what a key resolves to, when a lease ends,
//! what a sweep retires, and how a downloaded file is named. The network half is not exercised
//! here — `cache_video` and `download` add the fetch to what these cover.

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

fn write(paths: &Paths, mv_id: i64, quality: u16, bytes: &[u8]) -> AppResult<PathBuf> {
    write_entry(&cache_dir(paths), mv_id, quality, "mp4", Cursor::new(bytes), ENTRY_MAX_BYTES)
}

#[test]
fn an_entry_is_written_under_its_mv_tier_and_extension() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    let path = write_entry(
        &cache_dir(&paths),
        5_436_712,
        1080,
        "mp4",
        Cursor::new(b"bytes"),
        ENTRY_MAX_BYTES,
    )?;

    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("5436712-1080.mp4"));
    assert_eq!(std::fs::read(&path)?, b"bytes");
    Ok(())
}

#[test]
fn a_fresh_entry_is_found_under_the_key_that_wrote_it() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, 1080, b"bytes")?;

    assert_eq!(cached_video(&paths, 7, 1080), Some(path));
    assert_eq!(cached_video(&paths, 7, 720), None);
    assert_eq!(cached_video(&paths, 8, 1080), None);
    Ok(())
}

#[test]
fn a_directory_named_like_an_entry_is_not_served() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::create_dir_all(cache_dir(&paths).join("7-1080.mp4"))?;

    assert_eq!(cached_video(&paths, 7, 1080), None);
    Ok(())
}

#[test]
fn an_entry_past_its_lease_is_not_served() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, 1080, b"bytes")?;

    backdate(&path, ENTRY_TTL + Duration::from_secs(1))?;

    assert_eq!(cached_video(&paths, 7, 1080), None);
    Ok(())
}

#[test]
fn the_lease_is_measured_from_the_last_use() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let path = write(&paths, 7, 1080, b"bytes")?;
    backdate(&path, ENTRY_TTL + Duration::from_secs(1))?;
    assert_eq!(cached_video(&paths, 7, 1080), None);

    mark_used(&path)?;

    assert_eq!(cached_video(&paths, 7, 1080), Some(path));
    Ok(())
}

#[test]
fn a_body_larger_than_the_cap_is_refused_without_leaving_an_entry() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    let result = write_entry(&cache_dir(&paths), 7, 1080, "mp4", Cursor::new([1_u8; 5]), 4);

    assert!(result.is_err());
    assert!(!cache_dir(&paths).join("7-1080.mp4").exists());
    Ok(())
}

#[test]
fn a_body_exactly_on_the_cap_is_kept() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    let path = write_entry(&cache_dir(&paths), 7, 1080, "mp4", Cursor::new([1_u8; 4]), 4)?;

    assert_eq!(std::fs::metadata(&path)?.len(), 4);
    Ok(())
}

#[test]
fn a_sweep_retires_an_entry_past_its_lease() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let entry = write(&paths, 7, 1080, b"bytes")?;
    backdate(&entry, ENTRY_TTL + Duration::from_hours(1))?;

    let report = sweep(&paths)?;

    assert_eq!(report.removed, 1);
    assert!(!entry.exists());
    Ok(())
}

#[test]
fn a_sweep_evicts_the_oldest_entry_until_the_store_fits() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let old = write(&paths, 7, 1080, b"four")?;
    backdate(&old, Duration::from_hours(2))?;
    let new = write(&paths, 8, 1080, b"four")?;

    let report = sweep_dir(&cache_dir(&paths), SystemTime::now(), 4, ENTRY_TTL)?;

    assert_eq!(report.removed, 1);
    assert_eq!(report.remaining_bytes, 4);
    assert!(!old.exists());
    assert!(new.exists());
    Ok(())
}

#[test]
fn a_sweep_never_retires_a_name_it_did_not_write() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    let dir = cache_dir(&paths);
    std::fs::create_dir_all(&dir)?;
    let foreign = ["notes.txt", "7-abc.mp4", "7-0.mp4", "7-1080.MP4", "7-1080.mp4.bak"];
    for name in foreign {
        let path = dir.join(name);
        std::fs::write(&path, b"bytes")?;
        backdate(&path, ENTRY_TTL + Duration::from_hours(1))?;
    }

    let report = sweep(&paths)?;

    for name in foreign {
        assert!(dir.join(name).exists(), "{name} is not this cache's to retire");
    }
    assert_eq!(report.removed, 0);
    Ok(())
}

#[test]
fn a_missing_cache_directory_sweeps_quietly() -> AppResult<()> {
    let (_tmp, paths) = store()?;

    assert_eq!(sweep(&paths)?, SweepReport::default());
    Ok(())
}

#[test]
fn an_unreadable_cache_path_is_a_failure() -> AppResult<()> {
    let (_tmp, paths) = store()?;
    std::fs::write(cache_dir(&paths), b"")?;

    assert!(sweep(&paths).is_err());
    Ok(())
}

#[test]
fn an_extension_is_read_before_the_query_and_fragment() {
    let cases = [
        ("https://host/video.mp4?token=secret", "mp4"),
        ("https://host/video.MKV", "mkv"),
        ("https://host/video.flv#t=10", "flv"),
        ("https://host/video.webm", "webm"),
        ("https://host/video.m4a", DEFAULT_EXTENSION),
        ("https://host/video", DEFAULT_EXTENSION),
        ("https://host/video.mp4/", DEFAULT_EXTENSION),
    ];

    for (url, expected) in cases {
        assert_eq!(extension_for_url(url), expected, "{url}");
    }
}

#[test]
fn a_title_becomes_a_filename_stem() {
    let cases = [
        ("Concert / Live", "Concert _ Live"),
        ("  ...Best of...  ", "Best of"),
        ("Song\u{7}Title", "Song_Title"),
        ("   ", DEFAULT_FILE_STEM),
        ("...", DEFAULT_FILE_STEM),
    ];

    for (title, expected) in cases {
        assert_eq!(download_stem(title), expected, "{title:?}");
    }
}

#[test]
fn a_long_title_is_cut_to_the_stem_budget() {
    let title = "a".repeat(MAX_FILE_STEM_CHARS + 10);

    assert_eq!(download_stem(&title).chars().count(), MAX_FILE_STEM_CHARS);
}

#[test]
fn a_download_reserves_the_first_free_name_and_keeps_the_taken_one() -> AppResult<()> {
    let tmp = TempDir::new()?;
    let dir = tmp.path().join("Downloads");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("Song.mp4"), b"existing")?;

    let first = reserve_download_path(&dir, "Song", "mp4")?;
    let second = reserve_download_path(&dir, "Song", "mp4")?;

    assert_eq!(first.file_name().and_then(|name| name.to_str()), Some("Song (2).mp4"));
    assert_eq!(second.file_name().and_then(|name| name.to_str()), Some("Song (3).mp4"));
    assert_eq!(std::fs::read(dir.join("Song.mp4"))?, b"existing");
    Ok(())
}
