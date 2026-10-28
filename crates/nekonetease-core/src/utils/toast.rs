//! Process-wide bridge for surfacing backend failures as UI toasts.
//!
//! Backend code runs on tokio workers and can't touch the Slint notification stack
//! directly, `NotificationsUi` being `Rc`. So this owns a `OnceLock`
//! [`UnboundedSender`](tokio::sync::mpsc::UnboundedSender) any thread can push a
//! [`ToastRequest`] onto, drained by the UI-thread consumer
//! `boot::ui_setup::subscribers::install_toast_bridge` installs.
//!
//! It holds no `ui::*` types, which is what preserves the layering rule that `tasks`
//! never imports `ui` — the producer side is UI-free and localization happens entirely
//! on the consumer. The sender itself is [`crate::utils::event_bridge`], shared with
//! `tasks::play_count_flusher`; what stays here is the consumer, which in this
//! case is somebody else's.

use tokio::sync::mpsc::UnboundedReceiver;

use crate::utils::event_bridge::EventBridge;

/// Which localized template the UI consumer titles a toast with. A plain enum with no
/// `ui` dependency, so `player` / `library` / `tasks` classify a failure without
/// importing UI types and the consumer maps each kind to a translated string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// A track failed to decode or open. The music stopping is otherwise invisible, so this is
    /// always surfaced.
    PlaybackFailed,
    /// A user-initiated backend operation failed — folder scan, file import, settings
    /// save.
    OperationFailed,
    /// A sheet the panel resolved was written into the track's own lyrics tag. Informational and
    /// auto-dismissing, and surfaced rather than logged because it only happens when somebody
    /// presses a control that says so, and this one rewrites a file.
    LyricsSaved,
    /// A music video was written into the user's Downloads folder. Auto-dismissing, with the
    /// file's own name as the detail: an operation that can run for minutes owes a sign it
    /// finished, and a second download of the same title is told apart by that name.
    DownloadSaved,
    /// A Copy entry put its text on the clipboard. The copy itself is invisible, so this is the
    /// only sign it landed; auto-dismissing, and carrying no detail so the copied text is never
    /// shown or kept anywhere else.
    Copied,
    /// A Copy entry found the field empty on every item it was handed: a track with no album, an
    /// album with no artist. Said rather than skipped, the menu having closed on a click that
    /// otherwise did nothing visible.
    NothingToCopy,
    /// The account card's daily 云贝 check-in was claimed. The card paints the new balance, so the
    /// toast carries no detail and dismisses on its own.
    CheckInSucceeded,
    /// A restart the user asked for couldn't relaunch the app, so it stayed up instead
    /// of exiting into nothing. The setting is already persisted, so this is an
    /// instruction rather than a failure report: it sticks, and it is the one kind
    /// carrying **no** detail — there being no path or error worth showing.
    RestartRequired,
    /// The server handed back a preview window instead of the whole recording. Raised once per
    /// track start, because a listener who hears the music stop early deserves the reason.
    TrialPlayback,
    /// The recording needs a paid membership the signed-in account does not have.
    VipRequired,
    /// The recording has to be bought on its own or through its album first.
    AlbumPurchaseRequired,
    /// The service withheld the recording because it holds no licence for it.
    PlaybackNoCopyright,
    /// The service withheld the recording for a reason no fee code states: withdrawn, delisted,
    /// or blocked for the listener's region. The wire does not say which.
    PlaybackUnavailable,
    /// The server served a lower tier than the one requested. Raised once per track start — a
    /// seek keeps the tier it already announced — and the detail is the served bitrate.
    QualityDowngraded,
    /// An exclusive claim was refused and the audio went to the shared output instead. Raised once
    /// per distinct refusal rather than at every track start that retries it. The detail is the
    /// refusing device's name, or empty where it couldn't be found; the reason stays in the
    /// Output settings, whose words for it are translated where this detail is not.
    ExclusiveRefused,
}

/// A queued toast. [`kind`](Self::kind) picks the localized title on the UI side;
/// [`detail`](Self::detail) is the dynamic, untranslated body — a path or an error.
#[derive(Debug, Clone)]
pub struct ToastRequest {
    pub kind: ToastKind,
    pub detail: String,
}

/// Process-wide sender, claimed by [`init`]. Read by [`fn@notify`] so producers don't have to
/// carry a channel handle. (Disambiguated: the `notify` crate is a dependency of this build.)
static BRIDGE: EventBridge<ToastRequest> = EventBridge::new();

/// Register the process-wide sender and hand back the receiver for the UI-thread
/// consumer to drain. `None` if the bridge was already initialized — the first
/// receiver owns delivery, so the caller treats that as a no-op.
pub fn init() -> Option<UnboundedReceiver<ToastRequest>> {
    BRIDGE.install()
}

/// Queue a toast from any thread. No-op when the bridge isn't installed or after the
/// consumer has shut down.
///
/// Returns nothing, unlike its sibling in `tasks::play_count_flusher`: a failure worth toasting
/// has no second way to reach the user, so there is nothing for a caller to branch on.
pub fn notify(kind: ToastKind, detail: impl Into<String>) {
    BRIDGE.send(ToastRequest { kind, detail: detail.into() });
}

#[cfg(test)]
#[path = "tests/toast_tests.rs"]
mod tests;
