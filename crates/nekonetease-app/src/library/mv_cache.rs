//! A bounded on-disk store for music-video streams, and the copy the MV page saves on request.
//!
//! Sibling to [`super::cloud_cache`] rather than a second mode of it: a video is tens to hundreds
//! of megabytes against an audio track's tens, so one byte budget cannot serve both without one
//! starving the other. Both hold a stream the signed-in account may already play, fetched from the
//! URL playback would open, and neither logs that URL — it carries a signed token.
//!
//! Entries live under [`CACHE_DIR`] in the data root, named `<id>-<quality>.<extension>`, because
//! the filesystem is the whole index: no table, no reference set, and a sweep that retires only
//! the names this module writes.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nekonetease_core::config::Paths;
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::atomic_file::write_with_sync;
use nekonetease_net::services::net::range_reader::RangeReader;

/// The cache's share of the disk. Eight gigabytes holds roughly eighty 1080p videos, or several
/// hundred at 720p — a rotation's worth of the videos a listener returns to, which is the window
/// an offline cache has to cover. It is four times the audio cache's ceiling because one video
/// entry is tens of times an audio track; one number serving both would leave the audio cache
/// either starved or claiming a disk's worth of room it never uses.
pub const CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// No single video may take more than an eighth of the budget. A ten-minute 1080p concert video
/// is around 600 MB, so a real one is never refused, while a server answering with an unbounded
/// body cannot take the cache with one response.
pub const ENTRY_MAX_BYTES: u64 = CACHE_MAX_BYTES / 8;

/// A fortnight from the last play. Videos are not replayed like tracks and each entry costs far
/// more room, so the store hands its space back sooner than the audio cache's month.
pub const ENTRY_TTL: Duration = Duration::from_hours(14 * 24);

/// A user-requested download's own ceiling. The file lands in Downloads rather than in this cache,
/// so it is not charged against [`CACHE_MAX_BYTES`], but an explicit save is still the one path a
/// broken host could use to fill the disk. Two gigabytes clears the longest 1080p video the
/// service streams and refuses a body that is plainly not one.
pub const DOWNLOAD_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const CACHE_DIR: &str = "mv-cache";

/// The containers a saved or cached video may claim. MP4 is what the service serves; the others
/// are accepted so a container change becomes a different extension rather than a silent miss.
const CACHE_EXTENSIONS: [&str; 4] = ["mp4", "flv", "mkv", "webm"];

/// What a URL naming no known container is taken to be. The stream endpoint has answered with MP4
/// for every tier, so the fallback is a guess that at least opens.
const DEFAULT_EXTENSION: &str = "mp4";

/// What a download is called when the title has nothing usable left in it. Never empty: an empty
/// stem would make the file `.mp4`, a hidden file with no name.
const DEFAULT_FILE_STEM: &str = "video";

/// How much of a title survives into a filename. Long titles are common and most filesystems stop
/// at 255 bytes; 120 characters leaves room for the extension, the duplicate suffix and a
/// multi-byte title.
const MAX_FILE_STEM_CHARS: usize = 120;

/// How many `name (n).ext` candidates are tried before the name is refused. A loop with no bound
/// is the one shape that can spin forever on a directory that rejects a name for a reason other
/// than existence.
const DUPLICATE_NAME_ATTEMPTS: u32 = 10_000;

/// What one sweep did, for the caller's log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub removed: u32,
    pub bytes_freed: u64,
    pub remaining_bytes: u64,
    pub failed: u32,
}

pub fn cache_dir(paths: &Paths) -> PathBuf {
    cache_dir_at(&paths.data_dir)
}

/// The same directory reached from the data root, for a caller that holds one instead of `Paths`.
pub fn cache_dir_at(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_DIR)
}

/// The fresh entry for this MV and tier, if one exists.
///
/// The extension is not part of the key, so every extension the writer can produce is tried rather
/// than trusting the format the URL happened to carry months ago.
pub fn cached_video(paths: &Paths, mv_id: i64, quality: u16) -> Option<PathBuf> {
    cached_video_at(&paths.data_dir, mv_id, quality)
}

/// [`cached_video`] against a root the caller already holds.
pub fn cached_video_at(data_dir: &Path, mv_id: i64, quality: u16) -> Option<PathBuf> {
    entry_at(&cache_dir_at(data_dir), mv_id, quality, SystemTime::now())
}

/// Extends an entry's lease. Freshness is measured from the last use rather than from the
/// download, or a video kept in rotation would expire a fortnight after it was first watched.
pub fn mark_used(path: &Path) -> AppResult<()> {
    File::options().write(true).open(path)?.set_modified(SystemTime::now())?;
    Ok(())
}

/// Downloads `url` into the cache for this MV and tier, then trims the store back under its cap.
///
/// `Ok(None)` means the body announced itself larger than [`ENTRY_MAX_BYTES`] and was never
/// fetched. A failed download or write is an error, and `write_with_sync` discards the partial
/// file either way.
pub async fn cache_video(
    paths: &Paths,
    mv_id: i64,
    quality: u16,
    url: &str,
) -> AppResult<Option<PathBuf>> {
    let dir = cache_dir(paths);
    let extension = extension_for_url(url);
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let reader = RangeReader::open(&url)
            .map_err(|error| AppError::network("MV cache download failed", error))?;
        if reader.byte_len().is_some_and(|size| size > ENTRY_MAX_BYTES) {
            return Ok(None);
        }
        let entry = write_entry(&dir, mv_id, quality, extension, reader, ENTRY_MAX_BYTES)?;
        if let Err(error) = sweep_dir(&dir, SystemTime::now(), CACHE_MAX_BYTES, ENTRY_TTL) {
            log::warn!("MV cache trim failed: {}", describe(&error));
        }
        Ok(Some(entry))
    })
    .await
    .map_err(AppError::io_source)?
}

/// Saves `url` under `title`'s name in the user's Downloads folder and returns the file written.
///
/// The title is the MV's display name rather than anything derived from the URL, whose own last
/// segment is a CDN identifier. Nothing is logged about the URL: it carries a signed token.
///
/// Never overwrites — an existing `Title.mp4` makes this `Title (2).mp4` — and falls back to the
/// data root on a platform that has no Downloads folder to ask for.
pub async fn download(paths: &Paths, title: &str, url: &str) -> AppResult<PathBuf> {
    let dir = download_dir(paths);
    let extension = extension_for_url(url);
    let stem = download_stem(title);
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let reader = RangeReader::open(&url)
            .map_err(|error| AppError::network("MV download failed", error))?;
        if reader.byte_len().is_some_and(|size| size > DOWNLOAD_MAX_BYTES) {
            return Err(AppError::Validation(format!(
                "video exceeds the {DOWNLOAD_MAX_BYTES}-byte download cap"
            )));
        }
        let path = reserve_download_path(&dir, &stem, extension)?;
        if let Err(error) = write_capped(&path, reader, DOWNLOAD_MAX_BYTES) {
            // The reservation is an empty file; left behind it would read as a finished download
            // to the file manager this failure never reveals.
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        Ok(path)
    })
    .await
    .map_err(AppError::io_source)?
}

/// Streams `source` into an atomically written entry.
///
/// Separate from [`cache_video`] so the copy and its cap are testable without a socket. `max_bytes`
/// is [`ENTRY_MAX_BYTES`] in production and a smaller number only in a test.
pub fn write_entry(
    dir: &Path,
    mv_id: i64,
    quality: u16,
    extension: &str,
    source: impl Read,
    max_bytes: u64,
) -> AppResult<PathBuf> {
    let path = dir.join(entry_name(mv_id, quality, extension));
    write_capped(&path, source, max_bytes)?;
    Ok(path)
}

/// One pass over the cache: expired entries first, then the oldest survivors until the store fits.
pub fn sweep(paths: &Paths) -> AppResult<SweepReport> {
    sweep_dir(&cache_dir(paths), SystemTime::now(), CACHE_MAX_BYTES, ENTRY_TTL)
}

fn entry_at(dir: &Path, mv_id: i64, quality: u16, now: SystemTime) -> Option<PathBuf> {
    CACHE_EXTENSIONS.iter().find_map(|extension| {
        let path = dir.join(entry_name(mv_id, quality, extension));
        is_fresh(&path, now).then_some(path)
    })
}

fn is_fresh(path: &Path, now: SystemTime) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    // A timestamp ahead of the clock is a file this process cannot have aged; treating it as fresh
    // keeps a clock correction from deleting the cache wholesale.
    now.duration_since(modified).map_or(true, |age| age < ENTRY_TTL)
}

fn entry_name(mv_id: i64, quality: u16, extension: &str) -> String {
    format!("{mv_id}-{quality}.{extension}")
}

/// The container a URL names, or [`DEFAULT_EXTENSION`] when it names one this module does not
/// know. The query and fragment go first: a signed CDN URL ends in a token, not a suffix.
fn extension_for_url(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    let Some((_, extension)) = name.rsplit_once('.') else {
        return DEFAULT_EXTENSION;
    };
    CACHE_EXTENSIONS
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(extension))
        .unwrap_or(DEFAULT_EXTENSION)
}

/// Parses back only the names [`entry_name`] writes, so the sweep can never retire a file it did
/// not put there. The tier is any positive number rather than the endpoint's tier list: the
/// grammar is what this module wrote, and which tiers exist is `CloudService::mv_url`'s question.
fn parse_entry_name(name: &str) -> bool {
    let Some((id, rest)) = name.split_once('-') else {
        return false;
    };
    if !id.parse::<i64>().is_ok_and(|id| id > 0) {
        return false;
    }
    let Some((quality, extension)) = rest.split_once('.') else {
        return false;
    };
    quality.parse::<u16>().is_ok_and(|quality| quality > 0) && CACHE_EXTENSIONS.contains(&extension)
}

fn download_dir(paths: &Paths) -> PathBuf {
    // A platform with no Downloads folder still has the data root, the one directory this app
    // owns and can write to without asking.
    dirs::download_dir().unwrap_or_else(|| paths.data_dir.clone())
}

/// The title as a filename stem, and the one place a display name is made safe for a filesystem.
fn download_stem(title: &str) -> String {
    let cleaned: String = title.chars().map(|ch| if names_a_path(ch) { '_' } else { ch }).collect();
    let stem: String = cleaned.chars().take(MAX_FILE_STEM_CHARS).collect();
    let stem = stem.trim().trim_matches('.').trim();
    if stem.is_empty() { DEFAULT_FILE_STEM.to_owned() } else { stem.to_owned() }
}

/// The characters that would make a title name a path instead of a file. Windows rejects the whole
/// list; Linux needs only the separator, and a saved name has to survive either platform.
fn names_a_path(ch: char) -> bool {
    ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// Claims the first free `stem.ext`, `stem (2).ext`, … by creating the file exclusively. The
/// reservation is what makes two downloads of one title safe, and the write that follows renames
/// over the empty file it left.
fn reserve_download_path(dir: &Path, stem: &str, extension: &str) -> AppResult<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for attempt in 1..=DUPLICATE_NAME_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({attempt}).{extension}")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(AppError::io("could not reserve the download name", error));
            }
        }
    }
    Err(AppError::Validation(format!("too many files named {stem} in {}", dir.display())))
}

/// Streams at most `max_bytes` into a file, refusing the body after one byte past the cap. The
/// extra byte is what tells an exact fit from an overrun, and the temp file is discarded either
/// way.
fn write_capped(path: &Path, source: impl Read, max_bytes: u64) -> AppResult<()> {
    let mut source = source.take(max_bytes + 1);
    write_with_sync(path, |writer| {
        let written = std::io::copy(&mut source, writer)?;
        if written > max_bytes {
            return Err(AppError::Validation(format!("media body exceeded {max_bytes} bytes")));
        }
        Ok(())
    })
}

/// `max_bytes` and `ttl` are arguments rather than the constants so a test can drive the cap and
/// the clock without touching the real store's numbers.
fn sweep_dir(dir: &Path, now: SystemTime, max_bytes: u64, ttl: Duration) -> AppResult<SweepReport> {
    if !dir.exists() {
        return Ok(SweepReport::default());
    }

    let mut report = SweepReport::default();
    let mut live = Vec::new();
    // Bytes an expired entry still holds: a removal that failed has to stay in the accounting, or
    // the store reads smaller than it is and the next pass has the same budget to spend again.
    let mut stuck_bytes = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !parse_entry_name(&name) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            report.failed += 1;
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let path = entry.path();
        if now.duration_since(modified).is_ok_and(|age| age >= ttl) {
            if !retire(&path, metadata.len(), &mut report) {
                stuck_bytes += metadata.len();
            }
        } else {
            live.push((path, metadata.len(), modified));
        }
    }

    let mut total: u64 = live.iter().map(|(_, len, _)| *len).sum();
    if total > max_bytes {
        live.sort_by_key(|(_, _, modified)| *modified);
        for (path, len, _) in &live {
            if total <= max_bytes {
                break;
            }
            if retire(path, *len, &mut report) {
                total = total.saturating_sub(*len);
            }
        }
    }
    report.remaining_bytes = total + stuck_bytes;
    Ok(report)
}

fn retire(path: &Path, len: u64, report: &mut SweepReport) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => {
            report.removed += 1;
            report.bytes_freed += len;
            true
        }
        Err(error) => {
            log::warn!("Could not retire {}: {}", path.display(), describe(&error));
            report.failed += 1;
            false
        }
    }
}

#[cfg(test)]
#[path = "tests/mv_cache_tests.rs"]
mod tests;
