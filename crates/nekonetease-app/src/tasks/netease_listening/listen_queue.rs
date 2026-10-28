//! Durable retry queue for completed `NetEase` listens.
//!
//! The live tap writes here only when a report could not be delivered, so the hot path still
//! touches no disk. The file is bounded and self-healing: entries older than a week or past the
//! cap are dropped on load, and a file that will not parse reads as empty rather than failing boot.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use nekonetease_core::error::{AppResult, describe};
use nekonetease_core::utils::atomic_file;

pub(super) const FILE_NAME: &str = "listen-queue.json";
const VERSION: u8 = 1;
/// Oldest reports go first. Two hundred listens is many days of offline play for one user.
const CAPACITY: usize = 200;
const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// A live attempt and its retry can only land this close together if they are the same listen.
const DEDUPE_WINDOW_MS: u64 = 5_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct QueueEntry {
    pub song_id: i64,
    pub seconds: u64,
    pub captured_at_ms: u64,
    /// The account that heard the listen. Absent only in a file no [`ListenQueue::push`] wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<i64>,
    /// Identifies the row for removal while it is in flight. Not persisted; a reload renumbers.
    #[serde(skip)]
    pub(super) seq: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct QueueFile {
    version: u8,
    entries: Vec<QueueEntry>,
}

#[derive(Default)]
struct State {
    entries: VecDeque<QueueEntry>,
    next_seq: u64,
    dirty: bool,
}

pub(super) struct ListenQueue {
    path: PathBuf,
    state: Mutex<State>,
}

impl ListenQueue {
    pub(super) fn open(path: PathBuf) -> Arc<Self> {
        Self::open_at(path, now_ms())
    }

    fn open_at(path: PathBuf, now_ms: u64) -> Arc<Self> {
        let file = match atomic_file::load_json_or_default_sync::<QueueFile>(&path) {
            Ok(file) => file,
            Err(error) => {
                log::warn!("NetEase listening queue {}: {}", path.display(), describe(&error));
                QueueFile::default()
            }
        };
        if file.version > VERSION {
            log::warn!(
                "NetEase listening queue v{} is newer than v{VERSION}; reading it anyway",
                file.version
            );
        }
        let loaded = file.entries.len();
        let cutoff = now_ms.saturating_sub(MAX_AGE_MS);
        let mut entries: VecDeque<QueueEntry> =
            file.entries.into_iter().filter(|entry| entry.captured_at_ms >= cutoff).collect();
        while entries.len() > CAPACITY {
            entries.pop_front();
        }
        let mut next_seq = 0;
        for entry in &mut entries {
            entry.seq = next_seq;
            next_seq = next_seq.wrapping_add(1);
        }
        let dirty = entries.len() != loaded;
        Arc::new(Self { path, state: Mutex::new(State { entries, next_seq, dirty }) })
    }

    /// Appends unless the new report repeats the last one inside the dedupe window. Returns
    /// whether the queue changed, which is also the caller's cue to flush.
    pub(super) fn push(
        &self,
        song_id: i64,
        seconds: u64,
        account_id: Option<i64>,
        captured_at_ms: u64,
    ) -> bool {
        let mut state = self.state.lock();
        if state.entries.back().is_some_and(|last| {
            last.song_id == song_id
                && last.seconds == seconds
                && last.account_id == account_id
                && last.captured_at_ms.abs_diff(captured_at_ms) <= DEDUPE_WINDOW_MS
        }) {
            return false;
        }
        let seq = state.next_seq;
        state.next_seq = state.next_seq.wrapping_add(1);
        state.entries.push_back(QueueEntry { song_id, seconds, captured_at_ms, account_id, seq });
        while state.entries.len() > CAPACITY {
            state.entries.pop_front();
        }
        state.dirty = true;
        true
    }

    /// The entries `account_id` may report, oldest first. An entry loaded without an owner is
    /// delivered to whoever is signed in, which is the only reading a hand-made file supports.
    pub(super) fn deliverable(&self, account_id: i64) -> Vec<QueueEntry> {
        self.state
            .lock()
            .entries
            .iter()
            .filter(|entry| entry.account_id.is_none_or(|id| id == account_id))
            .cloned()
            .collect()
    }

    pub(super) fn has_deliverable(&self, account_id: i64) -> bool {
        self.state
            .lock()
            .entries
            .iter()
            .any(|entry| entry.account_id.is_none_or(|id| id == account_id))
    }

    pub(super) fn remove(&self, seq: u64) {
        let mut state = self.state.lock();
        let before = state.entries.len();
        state.entries.retain(|entry| entry.seq != seq);
        if state.entries.len() != before {
            state.dirty = true;
        }
    }

    /// Writes the queue through the shared atomic-file writer. The lock is held across the write
    /// so two flushes cannot land out of order and resurrect an entry the other removed.
    pub(super) fn flush(&self) -> AppResult<()> {
        let mut state = self.state.lock();
        if !state.dirty {
            return Ok(());
        }
        let file = QueueFile { version: VERSION, entries: state.entries.iter().cloned().collect() };
        atomic_file::write_json_sync(&self.path, &file)?;
        state.dirty = false;
        Ok(())
    }
}

pub(super) fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../tests/listen_queue_tests.rs"]
mod tests;
