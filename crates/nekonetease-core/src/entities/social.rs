//! `NetEase` social data: a user's profile and dynamics, their playlists, and the signed-in
//! account's private messages and notices.
//!
//! Message and notice text is display-only. None of it has a database column or a `Serialize`
//! derive, so it dies with the process; `cloud::social` carries the privacy argument for that.

use super::cloud::Playlist;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialProfile {
    pub id: i64,
    pub nickname: String,
    pub signature: String,
    pub avatar_url: Option<String>,
    pub background_url: Option<String>,
    pub follows: u64,
    pub followers: u64,
    pub playlist_count: u64,
    pub event_count: u64,
    pub followed: bool,
    pub mutual: bool,
    pub gender: i32,
    pub level: i32,
    pub vip_type: i32,
}

/// The signed-in account's own totals. A different endpoint from a viewed user's profile, which
/// carries the same three figures for somebody else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SocialSubcount {
    pub playlist_count: u64,
    pub event_count: u64,
    pub follow_count: u64,
    pub fans_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SocialEventKind {
    #[default]
    Unknown,
    Track,
    Playlist,
    Text,
}

/// One entry of a user's dynamics feed. The share payload arrives as a string of JSON on the wire,
/// so the client flattens it into these columns rather than handing the view another parser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialEvent {
    pub id: i64,
    pub kind: SocialEventKind,
    pub time_ms: i64,
    pub title: String,
    pub text: String,
    pub track_id: i64,
    pub track_title: String,
    pub track_artist: String,
    pub cover_url: Option<String>,
    pub playlist_id: i64,
    pub playlist_title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialEventPage {
    pub events: Vec<SocialEvent>,
    /// The cursor for the next page, or `None` when the service says there is no more.
    pub next_before: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialPlaylistPage {
    pub playlists: Vec<Playlist>,
    pub total: u64,
    pub has_more: bool,
}

/// One private conversation, already resolved against the signed-in account so nothing downstream
/// has to know which side of the exchange was ours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialConversation {
    pub peer_id: i64,
    pub peer_nickname: String,
    pub peer_avatar_url: Option<String>,
    pub last_message: String,
    pub from_me: bool,
    pub unread: u32,
    pub time_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialConversationPage {
    pub conversations: Vec<SocialConversation>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialMessage {
    pub id: i64,
    pub from_user_id: i64,
    pub from_nickname: String,
    pub from_avatar_url: Option<String>,
    pub content: String,
    pub time_ms: i64,
    pub from_me: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialMessagePage {
    pub messages: Vec<SocialMessage>,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SocialNoticeKind {
    Comment,
    #[default]
    Notice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialNotice {
    pub id: i64,
    pub kind: SocialNoticeKind,
    pub title: String,
    pub content: String,
    pub user_id: i64,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub comment_id: i64,
    pub time_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialNoticePage {
    pub notices: Vec<SocialNotice>,
    pub has_more: bool,
}
