//! The generation a replay owns, and what every write it causes re-checks.
//!
//! **The cursor check alone is not enough.** An open crosses a database read and an artwork
//! decode, so two presses landing in that gap both pass the start-of-task check and race to
//! write. [`ReplayToken`] is the monotonic order of presses; a write is conditional on the
//! token that spawned it still being the newest.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{NavEntry, PendingNav, nav};
use nekonetease_ui::AppWindow;
use slint::Weak;

tokio::task_local! {
    /// The replay driving the open on this task. Settled before the open starts, where reading
    /// the order inside the opener would pick up a press that landed in between.
    static SCOPED: ReplayToken;
}

/// One replay's generation. Only the newest may land.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReplayToken(u64);

impl ReplayToken {
    /// The order as it stands, for an open no replay is driving: any replay starting after it
    /// invalidates that open just the same.
    pub(crate) fn current() -> Self {
        Self(nav().replay_order.generation())
    }

    /// Whether no replay has started since this one.
    pub(crate) fn is_current(self) -> bool {
        self.0 == nav().replay_order.generation()
    }

    /// [`Self::is_current`] against a counter a test owns, so the gate is exercised without
    /// the process's navigation state.
    #[cfg(test)]
    pub(super) fn is_current_in(self, order: &ReplayOrder) -> bool {
        self.0 == order.generation()
    }
}

/// The token an open re-checks its writes against: the replay driving it when there is one,
/// otherwise the order at the moment it began.
pub(crate) fn open_token() -> ReplayToken {
    SCOPED.try_with(|token| *token).unwrap_or_else(|_| ReplayToken::current())
}

/// Run a replay's open under its token, so the opener reads [`open_token`] rather than racing a
/// fresh read against a press on the UI thread.
pub(super) fn in_replay<F: Future>(token: ReplayToken, open: F) -> impl Future<Output = F::Output> {
    SCOPED.scope(token, open)
}

/// The monotonic counter every [`ReplayToken`] is drawn from, one per process.
#[derive(Default)]
pub(super) struct ReplayOrder {
    generation: AtomicU64,
}

impl ReplayOrder {
    pub(super) fn begin(&self) -> ReplayToken {
        ReplayToken(self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1))
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

/// One replay's open: the entry it must still be current against when it starts, the navigation
/// it owes, and the handle to land that navigation on if the open never reaches its hook.
pub(super) struct ReplayOpen {
    pub(super) token: ReplayToken,
    pub(super) expected: NavEntry,
    pub(super) pending: Option<PendingNav>,
    pub(super) fallback: Weak<AppWindow>,
    /// Names the open in the warning a failure leaves.
    pub(super) label: &'static str,
}

/// The hook a replay's open runs once its detail is applied. The open has already refused to
/// reach this point with a stale token; checking again covers a newer press landing between that
/// check and this call.
pub(super) fn replay_hook(pending: Option<PendingNav>) -> impl FnOnce(&AppWindow) + Send + 'static {
    move |ui| {
        if let Some(pending) = pending
            && pending.token.is_current()
        {
            pending.apply_deferred(ui);
        }
    }
}

/// [`PendingNav::land`] over the same `Option`, so the bails read as one line.
pub(super) fn land_pending(pending: Option<PendingNav>, weak: &Weak<AppWindow>) {
    if let Some(pending) = pending {
        pending.land(weak);
    }
}
