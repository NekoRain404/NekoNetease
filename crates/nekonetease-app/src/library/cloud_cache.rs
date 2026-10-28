//! A bounded on-disk cache for cloud streams the signed-in account may already play.
//!
//! Nothing here grants access to anything: the ticket a download starts from is the same one
//! playback would open, and a listener who may stream a track may keep a copy of that stream on
//! their own disk for offline listening. The cache never stores a trial: the endpoint's own
//! `trial` flag says the bytes are a preview excerpt, so caching one would replay a fragment as
//! the whole track, and would be the one case where the copy is not what the account could stream.
//!
//! Entries live under [`CACHE_DIR`] in the data root because core's `Paths` names no cache
//! directory and the filesystem is the whole index: no table, no reference sweep, no second source
//! of truth to reconcile when an unlink fails.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use nekonetease_core::config::Paths;
use nekonetease_core::entities::cloud::{Quality, StreamTicket};
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::atomic_file::write_with_sync;
use nekonetease_net::services::net::range_reader::RangeReader;

/// The cache's share of the disk. Two gigabytes is roughly a hundred lossless albums' worth of
/// tracks, or a day of high-bitrate streaming, which is the window an offline cache has to cover;
/// beyond it the user is better served by a smaller store than by a player quietly claiming tens of
/// gigabytes.
pub const CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// No single track may take more than a quarter of the budget. A real lossless track is tens of
/// megabytes, so this never refuses one, while a server that answers with an unbounded body cannot
/// fill the cache with one response.
pub const ENTRY_MAX_BYTES: u64 = CACHE_MAX_BYTES / 4;

/// A month from the last use. The bytes themselves never go stale — a streamed master does not
/// change under the same ticket — so this is purely a disk budget: it lets a track the listener
/// returns to repeatedly stay cached while anything untouched for a month surrenders its space.
pub const ENTRY_TTL: Duration = Duration::from_hours(30 * 24);

const CACHE_DIR: &str = "cloud-cache";

const CACHE_EXTENSIONS: [&str; 4] = ["flac", "aac", "m4a", "mp3"];

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

/// The fresh entry for this song and tier, if one exists.
///
/// The extension is not part of the key, so every extension the writer can produce is tried rather
/// than trusting a format the caller would have to carry here as well.
pub fn cached_stream(paths: &Paths, song_id: i64, quality: Quality) -> Option<PathBuf> {
    cached_stream_at(&paths.data_dir, song_id, quality)
}

/// [`cached_stream`] against a root the caller already holds.
pub fn cached_stream_at(data_dir: &Path, song_id: i64, quality: Quality) -> Option<PathBuf> {
    entry_at(&cache_dir_at(data_dir), song_id, quality, SystemTime::now())
}

/// Extends an entry's lease. Freshness is measured from the last use rather than from the
/// download, or a track kept in rotation would expire a month after it was first heard.
pub fn mark_used(path: &Path) -> AppResult<()> {
    File::options().write(true).open(path)?.set_modified(SystemTime::now())?;
    Ok(())
}

/// Downloads `ticket`'s stream into the cache and trims the store back under its cap.
///
/// `quality` is the tier the bytes are — the served level where the endpoint reported one, since
/// keying a downgraded stream as the requested tier would replay it as something it is not.
///
/// `Ok(None)` means the ticket was deliberately not cached: a trial excerpt, or a body already
/// announced as larger than [`ENTRY_MAX_BYTES`]. A failed download or a failed write is an error,
/// and `write_with_sync` discards the partial file either way.
pub async fn cache_stream(
    paths: &Paths,
    ticket: &StreamTicket,
    quality: Quality,
) -> AppResult<Option<PathBuf>> {
    cache_stream_at(&paths.data_dir, ticket, quality).await
}

/// [`cache_stream`] against a root the caller already holds, which is the form `CloudPlayback`
/// can reach: it is handed the artwork directory rather than `Paths`.
///
/// The trial and size refusals live here so neither entry point can bypass one.
pub async fn cache_stream_at(
    data_dir: &Path,
    ticket: &StreamTicket,
    quality: Quality,
) -> AppResult<Option<PathBuf>> {
    if ticket.trial {
        return Ok(None);
    }
    if ticket.size.is_some_and(|size| size > ENTRY_MAX_BYTES) {
        return Ok(None);
    }

    let dir = cache_dir_at(data_dir);
    let url = ticket.url.clone();
    let song_id = ticket.song_id;
    let format = ticket.format.clone();
    tokio::task::spawn_blocking(move || {
        let reader = RangeReader::open(&url)
            .map_err(|error| AppError::network("Cloud cache download failed", error))?;
        let entry = write_entry(&dir, song_id, quality, &format, reader, ENTRY_MAX_BYTES)?;
        if let Err(error) = sweep_dir(&dir, SystemTime::now(), CACHE_MAX_BYTES, ENTRY_TTL) {
            log::warn!("Cloud cache trim failed: {}", describe(&error));
        }
        Ok(Some(entry))
    })
    .await
    .map_err(AppError::io_source)?
}

/// Streams `source` into an atomically written entry.
///
/// Separate from [`cache_stream`] so the copy and its cap are testable without a socket, and so the
/// blocking half stays one function with no async in it. `max_bytes` is [`ENTRY_MAX_BYTES`] in
/// production and a smaller number only in a test.
pub fn write_entry(
    dir: &Path,
    song_id: i64,
    quality: Quality,
    format: &str,
    source: impl Read,
    max_bytes: u64,
) -> AppResult<PathBuf> {
    let path = dir.join(entry_name(song_id, quality, extension_for(format)));
    let mut source = source.take(max_bytes + 1);
    write_with_sync(&path, |writer| {
        let written = std::io::copy(&mut source, writer)?;
        if written > max_bytes {
            return Err(AppError::Validation(format!(
                "cloud cache entry exceeded {max_bytes} bytes"
            )));
        }
        Ok(())
    })?;
    Ok(path)
}

/// One pass over the cache: expired entries first, then the oldest survivors until the store fits.
pub fn sweep(paths: &Paths) -> AppResult<SweepReport> {
    sweep_dir(&cache_dir(paths), SystemTime::now(), CACHE_MAX_BYTES, ENTRY_TTL)
}

fn entry_at(dir: &Path, song_id: i64, quality: Quality, now: SystemTime) -> Option<PathBuf> {
    CACHE_EXTENSIONS.iter().find_map(|extension| {
        let path = dir.join(entry_name(song_id, quality, extension));
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

fn entry_name(song_id: i64, quality: Quality, extension: &str) -> String {
    format!("{song_id}-{}.{extension}", quality.level())
}

/// Parses back only the names [`entry_name`] writes, so the sweep can never retire a file it did
/// not put there.
fn parse_entry_name(name: &str) -> Option<Quality> {
    let (id, rest) = name.split_once('-')?;
    id.parse::<i64>().ok().filter(|id| *id > 0)?;
    let (level, extension) = rest.split_once('.')?;
    if !CACHE_EXTENSIONS.contains(&extension) {
        return None;
    }
    match level {
        "standard" | "higher" | "exhigh" | "lossless" | "hires" => Some(Quality::from_level(level)),
        _ => None,
    }
}

fn extension_for(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "flac" => "flac",
        "aac" => "aac",
        "m4a" => "m4a",
        _ => "mp3",
    }
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
        if parse_entry_name(&name).is_none() {
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
#[path = "tests/cloud_cache_tests.rs"]
mod tests;
