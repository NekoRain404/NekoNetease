//! The back/forward stack itself. The parent module owns how a walk is replayed.

use std::collections::VecDeque;

const HISTORY_CAP: usize = 24;

/// One snapshot of the visible state. `detail_id == None` means the section's grid or
/// list is showing, `Some(id)` its detail view.
///
/// **A section alone stopped identifying a view when the five library pages became tabs
/// of one.** `tab` is the second half — `MyLibrary.tab-idx` inside section 3,
/// [`crate::ui::my_library::NO_TAB`] everywhere else. Without it a Songs → Albums move
/// duplicates the entry it followed and `record`'s dedup drops it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NavEntry {
    pub section: i32,
    pub tab: i32,
    pub detail_id: Option<i64>,
}

/// Linear back/forward stack with a cursor. See module docs.
pub struct NavHistory {
    entries: VecDeque<NavEntry>,
    cursor: usize,
    suppress: bool,
}

impl NavHistory {
    pub fn new() -> Self {
        Self { entries: VecDeque::with_capacity(HISTORY_CAP), cursor: 0, suppress: false }
    }

    /// Push a new entry at `cursor + 1`, truncating any forward history. A no-op while a
    /// replay is in flight; consecutive duplicates collapse.
    ///
    /// Returns whether the entry was taken, which is what [`record_current`] logs on:
    /// the eleven hooks fire two or three times per move, so an unconditional line reads
    /// as a stutter rather than as a navigation.
    pub fn record(&mut self, entry: NavEntry) -> bool {
        if self.suppress {
            return false;
        }
        if self.entries.is_empty() {
            self.entries.push_back(entry);
            self.cursor = 0;
            return true;
        }
        if self.entries.get(self.cursor) == Some(&entry) {
            return false;
        }
        // User-initiated navigation invalidates the forward stack.
        self.entries.truncate(self.cursor + 1);
        self.entries.push_back(entry);
        if self.entries.len() > HISTORY_CAP {
            self.entries.pop_front();
        }
        self.cursor = self.entries.len() - 1;
        true
    }

    /// Move cursor one step back; return the entry now under the cursor,
    /// or `None` if already at the start.
    pub fn back(&mut self) -> Option<NavEntry> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.entries.get(self.cursor).copied()
    }

    /// Move cursor one step forward; return the entry now under the
    /// cursor, or `None` if already at the end.
    pub fn forward(&mut self) -> Option<NavEntry> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor).copied()
    }

    pub fn set_suppress(&mut self, on: bool) {
        self.suppress = on;
    }

    /// The entry the cursor is on, or `None` if the history is empty. Read by
    /// [`spawn_open_detail`]'s future to bail when a newer replay has already moved the
    /// cursor past the target it was scheduled to fulfil.
    pub fn current(&self) -> Option<NavEntry> {
        self.entries.get(self.cursor).copied()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }
}

impl Default for NavHistory {
    fn default() -> Self {
        Self::new()
    }
}
