//! A comment thread's pages, and the fields a full surface needs beyond the preview.
//!
//! The preview's [`Comment`] stays the wire's six display fields; a full thread also needs to
//! know which comment a reply answers, how many replies it holds, and who wrote it, all of
//! which the like and delete routes address by id rather than by name.

use super::cloud::Comment;

/// Which route answers a page.
///
/// The two orderings are two endpoints, not a flag on one, and their offsets count
/// independently: switching restarts at zero rather than continuing the other's cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentSort {
    #[default]
    Hot,
    Time,
}

impl CommentSort {
    pub const fn is_hot(self) -> bool {
        matches!(self, Self::Hot)
    }
}

/// One row of a thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadComment {
    pub comment: Comment,
    /// The comment this one answers, `0` for a top-level row.
    pub parent_id: i64,
    /// Replies the server reports under this row. A floor page carries the count on the parent,
    /// not on its children.
    pub reply_count: u64,
    /// The author's account id, which is what decides whether the viewer may delete the row.
    pub author_id: i64,
    /// The author's picture, empty when the wire omits it. A URL rather than a decoded image, so
    /// the surface resolves it off the UI thread and every thread can carry one.
    pub avatar_url: String,
}

/// The counts a thread's own header shows. The rows carry no aggregate, and a resource detail
/// may answer a different shape than its comment thread does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommentThreadInfo {
    pub liked: bool,
    pub liked_count: u64,
}

/// One page of a thread. `hot` records which ordering answered it, so a late page can be
/// recognised against the surface's current pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommentPage {
    pub comments: Vec<ThreadComment>,
    pub total: u64,
    pub more: bool,
    pub hot: bool,
}
