//! What track is playing, with the fields shared by every Now-Playing surface.
//!
//! Borrowed throughout. [`PlayerViewModelLight::source`] runs on every state emit, so it may not
//! allocate; a consumer that needs to keep an answer past the borrow owns the few fields it
//! compares rather than the whole summary.
//!
//! **Spelled a second time in Slint**, as `Player.source-{title,subtitle,tertiary}`, because only
//! `@tr` at a literal reaches the catalogues.

use super::state::PlayerViewModelLight;

/// Which source a summary describes, and what tells one instance of it from the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceId {
    Track(i64),
}

/// What a Now-Playing surface states about whatever is on the deck.
///
/// Shaped after the fields the OS media controls and tray tooltip need, since both surfaces agree
/// on the questions and only disagree on which to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSummary<'a> {
    pub id: SourceId,
    pub title: &'a str,
    /// Who by, if the track has an artist.
    pub secondary: Option<&'a str>,
    pub album: Option<&'a str>,
    pub artwork_path: Option<&'a str>,
    pub duration_ms: Option<u64>,
}

impl PlayerViewModelLight {
    /// The source on the deck, or `None` when there is none.
    ///
    pub fn source(&self) -> Option<SourceSummary<'_>> {
        let track = self.current_track.as_deref()?;
        Some(SourceSummary {
            id: SourceId::Track(track.id),
            title: track.title.as_str(),
            secondary: track.artist.as_deref().and_then(non_empty),
            album: track.album.as_deref().and_then(non_empty),
            artwork_path: track.artwork_path.as_deref().and_then(non_empty),
            duration_ms: Some(self.duration_ms),
        })
    }
}

/// A field with something in it, trimmed. The ICY reader already trims what it stores, so this is
/// what keeps the ladder honest for the fields nobody else guards — a track's blank artist column
/// reaching MPRIS as an empty string rather than as an absent one.
fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
#[path = "tests/now_playing_tests.rs"]
mod tests;
