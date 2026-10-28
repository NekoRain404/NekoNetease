//! What a withheld stream tells the listener, and the error and toast it travels on.

use nekonetease_core::entities::cloud::{PlaybackRestriction, Song, UnavailableReason};
use nekonetease_core::error::AppError;
use nekonetease_core::utils::toast::ToastKind;

/// The notice a restriction raises. A trial still plays, so that one says what the listener is
/// hearing; the rest report why nothing started.
pub(super) const fn kind_for(restriction: PlaybackRestriction) -> ToastKind {
    match restriction {
        PlaybackRestriction::Trial { .. } => ToastKind::TrialPlayback,
        PlaybackRestriction::VipRequired => ToastKind::VipRequired,
        PlaybackRestriction::AlbumPurchaseRequired => ToastKind::AlbumPurchaseRequired,
        PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright } => {
            ToastKind::PlaybackNoCopyright
        }
        PlaybackRestriction::Unavailable {
            reason: UnavailableReason::NotAvailable | UnavailableReason::Unknown,
        } => ToastKind::PlaybackUnavailable,
    }
}

/// The error text a refusal carries, matching what the net layer reported before the reason was
/// typed. The detail a toast shows is the track title, not this.
pub(super) const fn refusal_message(restriction: PlaybackRestriction) -> &'static str {
    match restriction {
        PlaybackRestriction::Trial { .. } => "网易云仅提供试听",
        PlaybackRestriction::VipRequired => "该内容需要登录或会员权限",
        PlaybackRestriction::AlbumPurchaseRequired => "该歌曲需要购买专辑或数字单曲后才能播放",
        PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright } => {
            "该歌曲没有版权，暂时无法播放"
        }
        PlaybackRestriction::Unavailable {
            reason: UnavailableReason::NotAvailable | UnavailableReason::Unknown,
        } => "歌曲暂时无法播放或没有版权",
    }
}

/// A refusal's typed cause, so a caller or a test can read the reason without parsing the
/// message. `PlaybackRestriction` stays in core and is not an error by itself. Its `Display` is
/// the same sentence as the `AppError`'s message, which is what `error::describe` uses to avoid
/// repeating a cause the outer error already printed.
#[derive(Debug)]
pub(super) struct PlaybackDenied {
    restriction: PlaybackRestriction,
}

impl PlaybackDenied {
    // The reason is only ever read back by the suite; production callers already hold it.
    #[cfg(test)]
    pub(super) const fn restriction(&self) -> PlaybackRestriction {
        self.restriction
    }
}

impl std::fmt::Display for PlaybackDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(refusal_message(self.restriction))
    }
}

impl std::error::Error for PlaybackDenied {}

pub(super) fn refusal_error(restriction: PlaybackRestriction) -> AppError {
    AppError::network(refusal_message(restriction), PlaybackDenied { restriction })
}

/// Resolution either produced a stream or was refused. The refusal is not an `AppError` yet: the
/// queue walk suppresses the toast on its retries while still treating the entry as failed.
pub(super) enum ResolveFailure {
    Refused { restriction: PlaybackRestriction, detail: String },
    Failed(AppError),
}

impl From<AppError> for ResolveFailure {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

/// Narrow a stream row's unclassified refusal with what the song detail says. The detail can name
/// a missing licence or a withdrawn recording; it cannot separate a regional block from either,
/// so an otherwise unexplained refusal stays `Unknown` rather than being guessed at.
pub(super) fn refine_refusal(
    restriction: PlaybackRestriction,
    song: Option<&Song>,
) -> PlaybackRestriction {
    if !matches!(restriction, PlaybackRestriction::Unavailable { .. }) {
        return restriction;
    }
    let Some(song) = song else { return restriction };
    let fees = [song.fee, song.privilege.as_ref().map_or(0, |privilege| privilege.fee)];
    if fees.contains(&1) {
        return PlaybackRestriction::VipRequired;
    }
    if fees.contains(&4) {
        return PlaybackRestriction::AlbumPurchaseRequired;
    }
    if song.no_copyright {
        return PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright };
    }
    if let Some(privilege) = song.privilege.as_ref()
        && (privilege.st < 0 || (privilege.pl == 0 && privilege.dl == 0))
    {
        return PlaybackRestriction::Unavailable { reason: UnavailableReason::NotAvailable };
    }
    restriction
}
