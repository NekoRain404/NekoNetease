use std::collections::HashSet;
use std::sync::Arc;

use serde::Serialize;

use nekonetease_core::entities::track::TrackSummary;

use super::types::{PersistableQueue, RepeatMode};

/// Result of [`QueueState::prune_missing`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PruneOutcome {
    /// Number of distinct entries removed from `tracks`.
    pub removed: usize,
    /// True if the `play_order[current_index]` entry — the thing the player
    /// believes is currently playing — was in `ids_to_remove`. Caller uses
    /// this to decide whether to auto-skip to the next track.
    pub current_was_removed: bool,
}

/// What a removal left the entry `current_index` was naming.
///
/// That entry is the player's half of "this is what is on the deck", so a removal taking it away
/// without saying so leaves the deck playing a track the queue no longer names — and the next
/// `advance()` then steps over the entry that took its place.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RemovalOutcome {
    /// The current entry survived, shifted along if entries before it went, or no removal was
    /// made at all.
    #[default]
    Kept,
    /// The current entry went, and a surviving entry after it now holds its slot.
    Removed,
    /// The current entry went with nothing after it, so `current_index` clamps back onto the last
    /// survivor — a track the deck was not playing.
    RemovedAtTail,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueState {
    /// Canonical track data — each track stored exactly once.
    pub tracks: Vec<Arc<TrackSummary>>,
    /// Current playback order — indices into `tracks`.
    pub play_order: Vec<usize>,
    /// Original insertion order — indices into `tracks`. Used to restore on unshuffle.
    pub(crate) original_order: Vec<usize>,
    /// Index into `play_order` of the currently-playing track, or `None`
    /// when nothing is playing. The on-disk and Slint representations use a
    /// signed integer with `-1` as the no-track sentinel; conversion happens
    /// at those boundaries (`to_persistable`, `to_view_model`).
    pub current_index: Option<usize>,
    pub shuffle_enabled: bool,
    pub repeat_mode: RepeatMode,
    /// Monotonic counter incremented on every queue mutation.
    /// Used by `with_state_emit` to detect whether the queue changed.
    pub version: u64,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            play_order: Vec::new(),
            original_order: Vec::new(),
            current_index: None,
            shuffle_enabled: false,
            repeat_mode: RepeatMode::Off,
            version: 0,
        }
    }
}

/// Encode `current_index` as the on-disk / Slint i32 representation
/// (`-1` for `None`, otherwise the index saturated to `i32::MAX`).
pub fn current_index_to_i32(idx: Option<usize>) -> i32 {
    idx.map_or(-1, |i| i32::try_from(i).unwrap_or(i32::MAX))
}

/// Decode an i32 (`-1` sentinel, otherwise a non-negative index) into the
/// internal `Option<usize>` form.
pub fn current_index_from_i32(idx: i32) -> Option<usize> {
    usize::try_from(idx).ok()
}

impl QueueState {
    /// Number of tracks in the current play order.
    pub fn len(&self) -> usize {
        self.play_order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.play_order.is_empty()
    }

    /// Resolve an index in `play_order` to the actual track.
    fn track_at(&self, play_index: usize) -> Option<&Arc<TrackSummary>> {
        self.play_order.get(play_index).and_then(|&ti| self.tracks.get(ti))
    }

    pub fn add_tracks(&mut self, new_tracks: Vec<Arc<TrackSummary>>) {
        let start = self.tracks.len();
        let count = new_tracks.len();
        self.tracks.extend(new_tracks);
        let range = start..start + count;
        self.play_order.extend(range.clone());
        self.original_order.extend(range);
        self.version += 1;
    }

    pub fn insert_next(&mut self, track: Arc<TrackSummary>) {
        let track_idx = self.tracks.len();
        self.tracks.push(track);
        self.original_order.push(track_idx);

        let insert_pos = self.current_index.map_or(0, |ci| ci + 1).min(self.play_order.len());
        self.play_order.insert(insert_pos, track_idx);
        self.version += 1;
    }

    /// Remove the play-order entry at `index`, reporting how the current entry fared.
    ///
    /// Note: we don't remove from `tracks` to avoid invalidating every index into it. Orphaned
    /// track data is cleaned up on queue clear.
    pub fn remove_at(&mut self, index: usize) -> RemovalOutcome {
        if index >= self.play_order.len() {
            return RemovalOutcome::Kept;
        }
        self.remove_slots(&[index])
    }

    /// Remove multiple play-order indices at once in O(n) time, reporting how the current entry
    /// fared.
    pub fn remove_batch(&mut self, indices: &[usize]) -> RemovalOutcome {
        self.remove_slots(indices)
    }

    /// The removal both entry points share, so what a removed current entry means is decided
    /// once.
    fn remove_slots(&mut self, indices: &[usize]) -> RemovalOutcome {
        if indices.is_empty() {
            return RemovalOutcome::Kept;
        }

        let index_set: HashSet<usize> = indices.iter().copied().collect();

        // Classified before anything moves. Only a surviving slot *after* the current one gives
        // the deck somewhere to go; a tail removal clamps the index back onto an earlier
        // survivor, which no caller may mistake for a successor.
        let outcome = match self.current_index {
            Some(ci) if index_set.contains(&ci) => {
                let has_successor =
                    (ci + 1..self.play_order.len()).any(|i| !index_set.contains(&i));
                if has_successor { RemovalOutcome::Removed } else { RemovalOutcome::RemovedAtTail }
            }
            _ => RemovalOutcome::Kept,
        };

        let removed_track_indices: HashSet<usize> =
            index_set.iter().filter_map(|&i| self.play_order.get(i).copied()).collect();

        let mut pos = 0usize;
        self.play_order.retain(|_| {
            let keep = !index_set.contains(&pos);
            pos += 1;
            keep
        });
        self.original_order.retain(|ti| !removed_track_indices.contains(ti));

        if self.play_order.is_empty() {
            self.current_index = None;
        } else if let Some(ci) = self.current_index {
            // Entries removed before the current one shift it along with them. The current entry
            // itself leaves the index naming whatever moved into its slot, or the last survivor.
            let removed_before = index_set.iter().filter(|&&i| i < ci).count();
            let mut next = ci.saturating_sub(removed_before);
            if outcome != RemovalOutcome::Kept {
                next = next.min(self.play_order.len() - 1);
            }
            self.current_index = Some(next);
        }

        self.version += 1;
        outcome
    }

    pub fn move_track(&mut self, from: usize, to: usize) -> bool {
        if from >= self.play_order.len() || to >= self.play_order.len() {
            return false;
        }

        let idx = self.play_order.remove(from);
        self.play_order.insert(to, idx);

        if let Some(ci) = self.current_index {
            self.current_index = Some(if from == ci {
                to
            } else if from < ci && to >= ci {
                ci - 1
            } else if from > ci && to <= ci {
                ci + 1
            } else {
                ci
            });
        }
        self.version += 1;
        true
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.play_order.clear();
        self.original_order.clear();
        self.current_index = None;
        self.version += 1;
    }

    /// Jump to a specific index in the play order.
    pub fn skip_to_index(&mut self, index: usize) -> Option<&Arc<TrackSummary>> {
        if index >= self.len() {
            return None;
        }
        self.move_current_to(index);
        self.track_at(index)
    }

    pub fn get_current(&self) -> Option<&Arc<TrackSummary>> {
        let ci = self.current_index?;
        if ci >= self.len() {
            return None;
        }
        self.track_at(ci)
    }

    /// Point `current_index` at `next`, bumping the version only where that moves it.
    ///
    /// `with_state_emit` re-publishes the whole queue on a bump, and the outcomes that leave the
    /// index where it was — Repeat One replaying the current entry, a one-entry queue wrapping onto
    /// itself — have changed nothing to publish.
    fn move_current_to(&mut self, next: usize) {
        if self.current_index != Some(next) {
            self.current_index = Some(next);
            self.version += 1;
        }
    }

    pub fn advance(&mut self) -> Option<&Arc<TrackSummary>> {
        if self.play_order.is_empty() {
            return None;
        }

        let len = self.len();
        let next_from = |ci: Option<usize>| ci.map_or(0, |i| i + 1);
        let next = match self.repeat_mode {
            RepeatMode::One => return self.get_current(),
            RepeatMode::All => next_from(self.current_index) % len,
            RepeatMode::Off => {
                let next = next_from(self.current_index);
                if next >= len {
                    return None;
                }
                next
            }
        };
        self.move_current_to(next);
        self.get_current()
    }

    /// User-initiated skip — advances to next track.
    /// `RepeatMode::One` wraps like `RepeatMode::All`: manual navigation
    /// walks the queue in a loop. Only `Off` stops at the end.
    pub fn advance_skip(&mut self) -> Option<&Arc<TrackSummary>> {
        if self.play_order.is_empty() {
            return None;
        }

        let len = self.len();
        let next_from = |ci: Option<usize>| ci.map_or(0, |i| i + 1);
        let next = match self.repeat_mode {
            RepeatMode::All | RepeatMode::One => next_from(self.current_index) % len,
            RepeatMode::Off => {
                let next = next_from(self.current_index);
                if next >= len {
                    return None;
                }
                next
            }
        };
        self.move_current_to(next);
        self.get_current()
    }

    /// Move past an entry that could not be played. Unlike a user-initiated skip, failure
    /// recovery must not get trapped on the same item when Repeat One is selected. It still
    /// respects whether the queue wraps, and callers bound attempts to one queue lap.
    pub fn advance_after_failure(&mut self) -> Option<&Arc<TrackSummary>> {
        if self.play_order.is_empty() {
            return None;
        }

        let next = self.current_index.map_or(0, |index| index + 1);
        let next = match self.repeat_mode {
            RepeatMode::All | RepeatMode::One => next % self.len(),
            RepeatMode::Off if next < self.len() => next,
            RepeatMode::Off => return None,
        };
        self.version += 1;
        self.current_index = Some(next);
        self.get_current()
    }

    pub fn previous(&mut self) -> Option<&Arc<TrackSummary>> {
        if self.play_order.is_empty() {
            return None;
        }

        let next = if self.repeat_mode.wraps() {
            // Wraps. None and Some(0) both go to the last track.
            Some(self.current_index.filter(|&ci| ci > 0).map_or(self.len() - 1, |ci| ci - 1))
        } else {
            self.current_index.filter(|&ci| ci > 0).map(|ci| ci - 1)
        };
        if let Some(next) = next {
            self.move_current_to(next);
        }
        self.get_current()
    }

    pub fn peek_next(&self) -> Option<&Arc<TrackSummary>> {
        self.peek_next_entry().map(|(_, track)| track)
    }

    /// The entry [`Self::advance`] would move to, with its play-order slot — a queue can hold
    /// the same track twice, and only the slot tells the copies apart.
    pub fn peek_next_entry(&self) -> Option<(usize, &Arc<TrackSummary>)> {
        if self.play_order.is_empty() {
            return None;
        }

        let len = self.len();
        let next_from = |ci: Option<usize>| ci.map_or(0, |i| i + 1);
        let slot = match self.repeat_mode {
            RepeatMode::One => self.current_index?,
            RepeatMode::All => next_from(self.current_index) % len,
            RepeatMode::Off => {
                let next = next_from(self.current_index);
                if next >= len {
                    return None;
                }
                next
            }
        };
        self.track_at(slot).map(|track| (slot, track))
    }

    /// Move `current_index` onto the entry a gapless handover took, answering with the track it
    /// seated. `slot` is checked first for the duplicate case, the track itself is the fallback
    /// once a reorder moved it, and `None` means the queue no longer holds what the deck plays.
    pub fn seat_staged(
        &mut self,
        slot: usize,
        track: &Arc<TrackSummary>,
    ) -> Option<Arc<TrackSummary>> {
        let index = self.slot_holding(Some(slot), track)?;
        self.move_current_to(index);
        self.get_current().cloned()
    }

    /// Put `current_index` back on the entry holding `track`, for a control op that seated it in
    /// the monitor's gap; answers whether the queue still holds it.
    pub fn seat_source(&mut self, track: &Arc<TrackSummary>) -> bool {
        let Some(index) = self.slot_holding(None, track) else {
            return false;
        };
        self.move_current_to(index);
        true
    }

    /// The slot holding `track`, preferring `preferred` where it still holds it.
    fn slot_holding(&self, preferred: Option<usize>, track: &Arc<TrackSummary>) -> Option<usize> {
        let holds = |slot: usize| {
            self.track_at(slot).is_some_and(|held| Arc::ptr_eq(held, track) || held.id == track.id)
        };
        preferred
            .filter(|&slot| holds(slot))
            .or_else(|| (0..self.play_order.len()).find(|&slot| holds(slot)))
    }

    /// Begin shuffle: returns (`track_count`, `current_track_index`) for the shell to generate random indices.
    pub fn begin_shuffle(&self) -> (usize, Option<usize>) {
        (self.play_order.len(), self.current_index)
    }

    /// Apply a shuffled order provided by the shell.
    /// `indices` is a permutation of `0..play_order.len()` with the current track at index 0.
    /// These are indices into `play_order`, not into `tracks`.
    pub fn apply_shuffle_order(&mut self, indices: &[usize]) {
        // Remap: new play_order[i] = old play_order[indices[i]]
        let new_play_order: Vec<usize> =
            indices.iter().filter_map(|&i| self.play_order.get(i).copied()).collect();
        self.play_order = new_play_order;
        self.current_index = Some(0);
        self.shuffle_enabled = true;
        self.version += 1;
    }

    /// In-place version of [`Self::apply_shuffle_order`] that shuffles
    /// `play_order` directly using the caller-provided RNG. Avoids the
    /// double `Vec<usize>` allocation that the shell-driven path required
    /// (one for the indices in the caller, one for the remapped play
    /// order here). If `anchor_to_current` is true and the queue has a
    /// current track, that track is swapped to the front of the shuffled
    /// order so playback carries on from it.
    pub fn shuffle_play_order_in_place<R: rand::Rng + ?Sized>(
        &mut self,
        rng: &mut R,
        anchor_to_current: bool,
    ) {
        use rand::seq::SliceRandom;
        if self.play_order.is_empty() {
            return;
        }
        let pinned_track_idx = if anchor_to_current {
            self.current_index.and_then(|ci| self.play_order.get(ci).copied())
        } else {
            None
        };
        self.play_order.shuffle(rng);
        if let Some(ti) = pinned_track_idx
            && let Some(new_pos) = self.play_order.iter().position(|&i| i == ti)
        {
            self.play_order.swap(0, new_pos);
        }
        self.current_index = Some(0);
        self.shuffle_enabled = true;
        self.version += 1;
    }

    /// Restore original track order (unshuffle).
    /// Only clones `Vec<usize>` instead of `Vec<TrackSummary>`.
    pub fn unshuffle(&mut self) {
        let current_track_id = self.get_current().map(|t| t.id);
        self.play_order.clone_from(&self.original_order);

        if let Some(track_id) = current_track_id
            && let Some(pos) = self
                .play_order
                .iter()
                .position(|&ti| self.tracks.get(ti).is_some_and(|t| t.id == track_id))
        {
            self.current_index = Some(pos);
        }
        self.shuffle_enabled = false;
        self.version += 1;
    }

    pub fn cycle_repeat_mode(&mut self) {
        self.set_repeat_mode(match self.repeat_mode {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        });
    }

    /// Bumps `version` only on a move, so a panel re-sending the mode already set re-publishes
    /// no queue.
    pub fn set_repeat_mode(&mut self, mode: RepeatMode) {
        if self.repeat_mode == mode {
            return;
        }
        self.repeat_mode = mode;
        self.version += 1;
    }

    /// Drop tracks whose ID is in `ids_to_remove` from the queue. Maintains the
    /// `tracks` / `play_order` / `original_order` invariants by remapping all
    /// surviving track-indices. Adjusts `current_index` to land on the next
    /// surviving play-order entry at or after the old slot (or, if everything
    /// at or after was removed, the last survivor — or `None` for an empty
    /// queue).
    ///
    /// Bumps `version` only when at least one entry actually changed, so a
    /// no-op call (empty input or no matches) won't trigger a `ViewModel` re-emit.
    pub fn prune_missing(&mut self, ids_to_remove: &HashSet<i64>) -> PruneOutcome {
        self.prune_where(|track| ids_to_remove.contains(&track.id))
    }

    /// Prune missing local-library rows without treating a cloud song with the same numeric ID as
    /// the same queue entry. Cloud and local IDs belong to separate namespaces.
    pub fn prune_missing_local(&mut self, ids_to_remove: &HashSet<i64>) -> PruneOutcome {
        self.prune_where(|track| !track.file_path.is_empty() && ids_to_remove.contains(&track.id))
    }

    fn prune_where(
        &mut self,
        mut should_remove: impl FnMut(&TrackSummary) -> bool,
    ) -> PruneOutcome {
        if self.tracks.is_empty() {
            return PruneOutcome::default();
        }

        // Step 1: classify each `tracks` slot and build the old→new remap.
        let old_len = self.tracks.len();
        let mut keep_mask: Vec<bool> = Vec::with_capacity(old_len);
        let mut remap: Vec<Option<usize>> = Vec::with_capacity(old_len);
        let mut new_tracks: Vec<Arc<TrackSummary>> = Vec::with_capacity(old_len);
        for t in &self.tracks {
            if should_remove(t) {
                keep_mask.push(false);
                remap.push(None);
            } else {
                keep_mask.push(true);
                remap.push(Some(new_tracks.len()));
                new_tracks.push(Arc::clone(t));
            }
        }

        let removed = old_len - new_tracks.len();

        if removed == 0 {
            return PruneOutcome::default();
        }

        // Step 2: figure out whether the "currently playing" entry is among
        // the casualties — *before* mutating anything else, so `current_index`
        // still resolves through the old `play_order`.
        let current_was_removed = self
            .current_index
            .and_then(|ci| self.play_order.get(ci).copied())
            .is_some_and(|ti| !keep_mask[ti]);

        // Step 3: rebuild play_order, dropping entries whose target was
        // removed, and remember where each old slot lands (or that it
        // didn't) so we can fix up `current_index` afterwards.
        let old_play_order = std::mem::take(&mut self.play_order);
        let mut new_play_order: Vec<usize> = Vec::with_capacity(old_play_order.len());
        let mut slot_remap: Vec<Option<usize>> = Vec::with_capacity(old_play_order.len());
        for old_ti in old_play_order {
            match remap[old_ti] {
                Some(new_ti) => {
                    slot_remap.push(Some(new_play_order.len()));
                    new_play_order.push(new_ti);
                }
                None => slot_remap.push(None),
            }
        }
        self.play_order = new_play_order;

        // Step 4: rebuild original_order with the same remap rules. We don't
        // need a slot map for it — it's never indexed by `current_index`.
        let old_original_order = std::mem::take(&mut self.original_order);
        let mut new_original_order: Vec<usize> = Vec::with_capacity(old_original_order.len());
        for old_ti in old_original_order {
            if let Some(new_ti) = remap[old_ti] {
                new_original_order.push(new_ti);
            }
        }
        self.original_order = new_original_order;

        // Step 5: recompute current_index in the new play_order space.
        // Strategy: find the next surviving slot at or after the old index;
        // fall back to the last survivor; fall back to None when empty.
        self.current_index = self.current_index.and_then(|old_ci| {
            slot_remap
                .iter()
                .skip(old_ci)
                .find_map(|s| *s)
                .or_else(|| self.play_order.len().checked_sub(1))
        });

        self.tracks = new_tracks;
        self.version += 1;

        PruneOutcome { removed, current_was_removed }
    }

    pub fn to_persistable(&self) -> PersistableQueue {
        let mut track_ids: Vec<i64> = Vec::with_capacity(self.play_order.len());
        track_ids
            .extend(self.play_order.iter().filter_map(|&ti| self.tracks.get(ti).map(|t| t.id)));
        PersistableQueue { track_ids, current_index: current_index_to_i32(self.current_index) }
    }

    /// Return tracks in current play order for `ViewModel` emission.
    /// With Arc, each clone is a ref-count increment (no deep String copies).
    pub fn tracks_in_play_order(&self) -> Vec<Arc<TrackSummary>> {
        let mut result = Vec::with_capacity(self.play_order.len());
        for &ti in &self.play_order {
            if let Some(track) = self.tracks.get(ti) {
                result.push(Arc::clone(track));
            }
        }
        result
    }
}

#[cfg(test)]
#[path = "tests/queue_tests.rs"]
mod tests;
