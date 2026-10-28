//! Projection from the wire entities to the Slint rows.
//!
//! Times are formatted here because Slint's `int` is 32-bit and a `NetEase` timestamp is past
//! `i32::MAX`; the surface gets a label rather than a number it would truncate. Every remote id is
//! carried as decimal text for the same reason.

use slint::SharedString;

use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::entities::social::{
    SocialConversation, SocialEvent, SocialEventKind, SocialMessage, SocialNotice, SocialNoticeKind,
};
use nekonetease_ui::{
    SocialConversationRow, SocialEventRow, SocialMessageRow, SocialNoticeRow, SocialPlaylistRow,
};

pub(super) fn playlist_row(playlist: &Playlist) -> SocialPlaylistRow {
    let creator =
        playlist.creator.as_ref().map(|creator| creator.nickname.as_str()).unwrap_or_default();
    let count = playlist.track_count.to_string();
    let subtitle = if creator.is_empty() { count } else { format!("{count} · {creator}") };
    SocialPlaylistRow {
        id: playlist.id.to_string().into(),
        title: playlist.name.as_str().into(),
        subtitle: subtitle.into(),
        artwork_path: remote_artwork(playlist.cover_url.as_deref()),
    }
}

pub(super) fn event_row(event: &SocialEvent) -> SocialEventRow {
    SocialEventRow {
        id: event.id.to_string().into(),
        kind: match event.kind {
            SocialEventKind::Track => "track",
            SocialEventKind::Playlist => "playlist",
            SocialEventKind::Text => "text",
            SocialEventKind::Unknown => "unknown",
        }
        .into(),
        title: event.title.as_str().into(),
        text: event.text.as_str().into(),
        subtitle: event.track_artist.as_str().into(),
        display_time: time_label(event.time_ms),
        artwork_path: remote_artwork(event.cover_url.as_deref()),
    }
}

pub(super) fn conversation_row(conversation: &SocialConversation) -> SocialConversationRow {
    SocialConversationRow {
        peer_id: conversation.peer_id.to_string().into(),
        nickname: conversation.peer_nickname.as_str().into(),
        last_message: conversation.last_message.as_str().into(),
        display_time: time_label(conversation.time_ms),
        avatar_path: remote_artwork(conversation.peer_avatar_url.as_deref()),
        unread: i32::try_from(conversation.unread).unwrap_or(i32::MAX),
        from_me: conversation.from_me,
    }
}

pub(super) fn message_row(message: &SocialMessage) -> SocialMessageRow {
    SocialMessageRow {
        id: message.id.to_string().into(),
        author: message.from_nickname.as_str().into(),
        content: message.content.as_str().into(),
        display_time: time_label(message.time_ms),
        avatar_path: remote_artwork(message.from_avatar_url.as_deref()),
        from_me: message.from_me,
    }
}

pub(super) fn notice_row(notice: &SocialNotice) -> SocialNoticeRow {
    let user_id = if notice.user_id > 0 { notice.user_id.to_string() } else { String::new() };
    SocialNoticeRow {
        id: notice.id.to_string().into(),
        kind: match notice.kind {
            SocialNoticeKind::Comment => "comment",
            SocialNoticeKind::Notice => "notice",
        }
        .into(),
        title: notice.title.as_str().into(),
        content: notice.content.as_str().into(),
        nickname: notice.nickname.as_str().into(),
        user_id: user_id.into(),
        display_time: time_label(notice.time_ms),
        avatar_path: remote_artwork(notice.avatar_url.as_deref()),
    }
}

pub(super) fn adjust_follow_count(current: &str, delta: i64) -> String {
    let Ok(value) = current.parse::<i64>() else {
        return current.to_owned();
    };
    value.saturating_add(delta).max(0).to_string()
}

pub(super) fn remote_artwork(url: Option<&str>) -> SharedString {
    url.filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default()
        .into()
}

fn time_label(ms: i64) -> SharedString {
    if ms <= 0 {
        return SharedString::default();
    }
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
        .into()
}

#[cfg(test)]
#[path = "tests/rows_tests.rs"]
mod tests;
