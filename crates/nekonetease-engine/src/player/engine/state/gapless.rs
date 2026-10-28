//! What the state does when the deck hands the next gapless source over by itself.
//!
//! The handover happens in the audio callback, under neither `exec_lock` nor `deck_epoch`, so by
//! the time the monitor reports it the deck is already playing the staged entry. A pick, pause or
//! seek can land in the monitor's gap, and the queue may disagree with the deck by then — so this
//! builder re-derives from what the deck took rather than from the queue's next slot.

use std::sync::Arc;

use super::{PlayerAction, PlayerState};
use crate::player::engine::types::PlaybackStatus;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_playback::player::playback::replaygain::TrackReplayGain;

/// What the monitor staged behind the playing track when it issued the gapless preload.
///
/// The deck sequences sources without naming them — `Voice::playing` answers a shape and a
/// format, not a track — so this record is the only way the handover can be mapped back to the
/// queue entry it took.
#[derive(Debug, Clone)]
pub struct StagedTrack {
    /// The `play_order` slot it was read from.
    pub slot: usize,
    pub track: Arc<TrackSummary>,
    pub replaygain: TrackReplayGain,
}

/// What the state looked like when the deck read reported the staged source took over, for
/// [`PlayerState::build_gapless_actions`] to check again under the emit lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaplessDecision {
    pub track_id: Option<i64>,
    /// The state's own position, which only a seek or a restart moves while the track is seated.
    pub position_ms: u64,
}

impl GaplessDecision {
    /// Snapshot the two terms the builder re-checks. Taken *before* the deck read, for the reason
    /// [`EndOfStreamDecision::of`](super::EndOfStreamDecision::of) gives.
    pub fn of(state: &PlayerState) -> Self {
        Self {
            track_id: state.current_track().map(|track| track.id),
            position_ms: state.position_ms,
        }
    }
}

impl PlayerState {
    /// Build actions for the monitor's gapless transition, where the staged source has already
    /// taken the deck over.
    ///
    /// Unlike end-of-stream this decision is not a veto. A bail would leave `current_index` on the
    /// track the deck has finished with, and the next end-of-stream would skip an entry. So a
    /// stale decision still reconciles the index with what the control op seated; only the play
    /// count and the source move are withheld.
    pub fn build_gapless_actions(
        &mut self,
        decision: GaplessDecision,
        staged: Option<StagedTrack>,
    ) -> Vec<PlayerAction> {
        // A stop took the deck back behind the decision; nothing handed over.
        if self.status == PlaybackStatus::Stopped {
            return Vec::new();
        }

        let same_track = self.current_track().map(|track| track.id) == decision.track_id;
        if !same_track || self.position_ms != decision.position_ms {
            return self.reconcile_index_with_source();
        }

        let Some(staged) = staged else {
            return Vec::new();
        };
        let Some(track) = self.queue.seat_staged(staged.slot, &staged.track) else {
            // The deck is playing an entry the queue no longer holds. Nothing can name it, and
            // leaving it audible under a source naming another track is the bug this builder
            // exists to stop.
            log::warn!(
                "Gapless handover reached track {} the queue no longer holds; stopping",
                staged.track.id
            );
            return self.clear_removed_source();
        };

        let mut actions = Vec::with_capacity(1);
        // A pause can land in the gap. The index still has to move — the deck has the track — but
        // a play the user interrupted is not counted, the rule the end-of-stream builder's status
        // check applies.
        if self.status == PlaybackStatus::Playing
            && let Some(ended) = decision.track_id
        {
            actions.push(PlayerAction::UpdatePlayCount(ended));
        }
        self.position_ms = 0;
        self.duration_ms = u64::try_from(track.duration_ms.max(0)).unwrap_or(0);
        self.source = Some(track);
        actions
    }

    /// Point `current_index` at whatever `source` names after a control op seated it in the
    /// monitor's gap, or clear the source where the queue no longer holds it.
    fn reconcile_index_with_source(&mut self) -> Vec<PlayerAction> {
        let Some(track) = self.source.clone() else {
            return Vec::new();
        };
        if self.queue.seat_source(&track) {
            return Vec::new();
        }
        log::warn!("Gapless handover: source track {} is not in the queue; clearing it", track.id);
        self.clear_removed_source()
    }
}
