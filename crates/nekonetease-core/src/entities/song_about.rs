//! The song wiki's "About this song" page.

/// The editorial half of a recording's wiki page: the short attributes it leads with, then the
/// paragraphs a reviewer wrote. Both are empty when the service has no entry for the recording.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongAbout {
    /// Tags, language, tempo and awards, in the page's own order.
    pub facts: Vec<String>,
    /// Review prose, one entry per paragraph the page carries.
    pub reviews: Vec<String>,
}

impl SongAbout {
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty() && self.reviews.is_empty()
    }
}
