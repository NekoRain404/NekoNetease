//! A user's profile and dynamics, their public playlists, and the signed-in account's private
//! messages and notifications.
//!
//! Message bodies stay in memory and are never logged. They have no database column: the rolling
//! log ships inside bug reports, and a private conversation would be the one thing in this tree
//! whose leak a user could not undo. `cloud::social` repeats the rule on the application side,
//! where the temptation to persist would live.

use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::entities::social::{
    SocialConversation, SocialConversationPage, SocialEvent, SocialEventKind, SocialEventPage,
    SocialMessage, SocialMessagePage, SocialNotice, SocialNoticeKind, SocialNoticePage,
    SocialPlaylistPage, SocialProfile, SocialSubcount,
};
use serde_json::{Value, json};

use crate::netease::endpoints::Protocol;
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, number_as_i64};

const USER_DETAIL: &str = "/api/v1/user/detail";
const SUBCOUNT: &str = "/api/subcount";
const FOLLOW: &str = "/api/user/follow";
const UNFOLLOW: &str = "/api/user/delfollow";
const USER_EVENT: &str = "/api/event/get";
const USER_PLAYLIST: &str = "/api/user/playlist";
const PRIVATE_USERS: &str = "/api/msg/private/users";
const PRIVATE_HISTORY: &str = "/api/msg/private/history";
const NOTICES: &str = "/api/msg/notices";
const USER_COMMENTS: &str = "/api/v1/user/comments";

impl Client {
    /// Reads a user's public profile. `followed` is only meaningful for a signed-in caller; an
    /// anonymous response answers `null`, which reads as false.
    pub async fn user_profile(&self, user_id: i64) -> Result<SocialProfile> {
        let path = format!("{USER_DETAIL}/{user_id}");
        let body = self.request_at(&path, Protocol::Weapi, json!({})).await?;
        decode_profile(&body)
    }

    pub async fn user_subcount(&self) -> Result<SocialSubcount> {
        let body = self.request_at(SUBCOUNT, Protocol::Weapi, json!({})).await?;
        Ok(decode_subcount(&body))
    }

    /// Follows or unfollows the user. The two directions are different wire paths rather than a
    /// flag, which is also why a retry cannot turn an unfollow into a second follow.
    pub async fn set_user_followed(&self, user_id: i64, followed: bool) -> Result<()> {
        let path = format!("{}/{user_id}", if followed { FOLLOW } else { UNFOLLOW });
        self.request_at_write(&path, Protocol::Weapi, json!({})).await?;
        Ok(())
    }

    /// One page of a user's dynamics. `before` is the service's millisecond cursor, `-1` for the
    /// first page.
    pub async fn user_events(
        &self,
        user_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> Result<SocialEventPage> {
        let path = format!("{USER_EVENT}/{user_id}");
        let body = self
            .request_at(
                &path,
                Protocol::Weapi,
                json!({
                    "getcounts": true,
                    "time": before.unwrap_or(-1),
                    "limit": limit,
                    "total": false,
                    "fromRN": "true",
                }),
            )
            .await?;
        decode_event_page(&body)
    }

    pub async fn user_playlist_page(
        &self,
        user_id: i64,
        offset: u64,
        limit: u64,
    ) -> Result<SocialPlaylistPage> {
        let body = self
            .request_at(
                USER_PLAYLIST,
                Protocol::Weapi,
                json!({
                    "uid": user_id,
                    "limit": limit,
                    "offset": offset,
                    "includeVideo": true,
                }),
            )
            .await?;
        decode_playlist_page(&body, offset)
    }

    /// The conversations list, resolved against `self_id` so the caller never has to decide which
    /// side of an exchange was its own.
    pub async fn private_conversations(
        &self,
        self_id: i64,
        offset: u64,
        limit: u64,
    ) -> Result<SocialConversationPage> {
        let body = self
            .request_at(
                PRIVATE_USERS,
                Protocol::Weapi,
                json!({"offset": offset, "limit": limit, "total": true}),
            )
            .await?;
        decode_conversation_page(&body, self_id, offset)
    }

    pub async fn private_messages(
        &self,
        self_id: i64,
        user_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> Result<SocialMessagePage> {
        let body = self
            .request_at(
                PRIVATE_HISTORY,
                Protocol::Weapi,
                json!({
                    "userId": user_id,
                    "limit": limit,
                    "time": before.unwrap_or(0),
                    "total": true,
                }),
            )
            .await?;
        decode_message_page(&body, self_id)
    }

    /// System notices, paged by the service's millisecond cursor.
    pub async fn notices(&self, before: Option<i64>, limit: u32) -> Result<SocialNoticePage> {
        let body = self
            .request_at(
                NOTICES,
                Protocol::Weapi,
                json!({"limit": limit, "time": before.unwrap_or(-1)}),
            )
            .await?;
        decode_notice_page(&body)
    }

    /// Replies and mentions on the account's own comments.
    pub async fn comment_notifications(
        &self,
        user_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> Result<SocialNoticePage> {
        let path = format!("{USER_COMMENTS}/{user_id}");
        let body = self
            .request_at(
                &path,
                Protocol::Weapi,
                json!({
                    "beforeTime": before.unwrap_or(-1),
                    "limit": limit,
                    "total": true,
                    "uid": user_id,
                }),
            )
            .await?;
        decode_comment_notice_page(&body)
    }
}

fn decode_profile(body: &Value) -> Result<SocialProfile> {
    let profile = object_at(body, "profile").unwrap_or(body);
    let id = number_as_i64(&profile["userId"])
        .or_else(|| number_as_i64(&profile["id"]))
        .ok_or(Error::InvalidResponse)?;
    Ok(SocialProfile {
        id,
        nickname: text_at(profile, "nickname"),
        signature: text_at(profile, "signature"),
        avatar_url: url_at(profile, "avatarUrl"),
        background_url: url_at(profile, "backgroundUrl"),
        follows: count_at(profile, "follows"),
        followers: count_at(profile, "followers"),
        playlist_count: count_at(profile, "playlistCount"),
        event_count: count_at(profile, "eventCount"),
        followed: flag_at(profile, "followed"),
        mutual: flag_at(profile, "mutual"),
        gender: i32::try_from(count_at(profile, "gender")).unwrap_or_default(),
        level: i32::try_from(count_at(profile, "level")).unwrap_or_default(),
        vip_type: i32::try_from(count_at(profile, "vipType")).unwrap_or_default(),
    })
}

fn decode_subcount(body: &Value) -> SocialSubcount {
    let data = object_at(body, "data").unwrap_or(body);
    SocialSubcount {
        playlist_count: count_at(data, "playlistCount"),
        event_count: count_at(data, "eventCount"),
        follow_count: count_at(data, "followCount"),
        fans_count: count_at(data, "fansCount").max(count_at(data, "followeds")),
    }
}

pub(super) fn decode_event_page(body: &Value) -> Result<SocialEventPage> {
    let entries = array_at(body, &["events", "event"]).ok_or(Error::InvalidResponse)?;
    let events = entries.iter().map(decode_event).collect::<Vec<_>>();
    let last = events.last().map(|event| event.time_ms).filter(|time| *time > 0);
    let more = flag_at(body, "more")
        || body.get("lasttime").and_then(number_as_i64).is_some_and(|time| time > 0);
    Ok(SocialEventPage { events, next_before: next_before(body, last, more) })
}

fn decode_event(entry: &Value) -> SocialEvent {
    let payload = entry
        .get("json")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let payload = payload.as_ref().unwrap_or(entry);
    let song = object_at(payload, "song").or_else(|| object_at(entry, "song"));
    let playlist = object_at(payload, "playlist").or_else(|| object_at(entry, "playlist"));
    let track_title = song.map(|song| text_at(song, "name")).unwrap_or_default();
    let playlist_title = playlist.map(|list| text_at(list, "name")).unwrap_or_default();
    let text =
        nonempty(&[text_at(payload, "msg"), text_at(payload, "comment"), text_at(entry, "msg")]);
    let kind = if song.is_some() {
        SocialEventKind::Track
    } else if playlist.is_some() {
        SocialEventKind::Playlist
    } else if text.is_empty() {
        SocialEventKind::Unknown
    } else {
        SocialEventKind::Text
    };
    let cover_url = song
        .and_then(|song| object_at(song, "al").or_else(|| object_at(song, "album")))
        .and_then(|album| url_at(album, "picUrl"))
        .or_else(|| playlist.and_then(|list| url_at(list, "coverImgUrl")));
    SocialEvent {
        id: number_as_i64(&entry["id"]).unwrap_or_default(),
        kind,
        time_ms: time_at(entry, &["eventTime", "showTime", "time"]),
        title: nonempty(&[text_at(payload, "title"), track_title.clone(), playlist_title.clone()]),
        text,
        track_id: song.and_then(|song| number_as_i64(&song["id"])).unwrap_or_default(),
        track_title,
        track_artist: song.map(artist_names).unwrap_or_default(),
        cover_url,
        playlist_id: playlist.and_then(|list| number_as_i64(&list["id"])).unwrap_or_default(),
        playlist_title,
    }
}

fn decode_playlist_page(body: &Value, offset: u64) -> Result<SocialPlaylistPage> {
    let entries = array_at(body, &["playlist", "playlists"]).ok_or(Error::InvalidResponse)?;
    let mut playlists = Vec::with_capacity(entries.len());
    for entry in entries {
        playlists.push(decode_ref::<Playlist>(entry)?);
    }
    let reported = count_at(body, "total");
    let total = if reported == 0 { offset + playlists.len() as u64 } else { reported };
    let has_more = flag_at(body, "more") || offset + (playlists.len() as u64) < total;
    Ok(SocialPlaylistPage { playlists, total, has_more })
}

pub(super) fn decode_conversation_page(
    body: &Value,
    self_id: i64,
    offset: u64,
) -> Result<SocialConversationPage> {
    let entries = array_at(body, &["msgs", "conversations"]).ok_or(Error::InvalidResponse)?;
    let conversations = entries.iter().map(|entry| decode_conversation(entry, self_id)).collect();
    let total = count_at(body, "total");
    let has_more = flag_at(body, "more")
        || flag_at(body, "hasMore")
        || (total > 0 && offset + (entries.len() as u64) < total);
    Ok(SocialConversationPage { conversations, has_more })
}

fn decode_conversation(entry: &Value, self_id: i64) -> SocialConversation {
    let from = object_at(entry, "fromUser").or_else(|| object_at(entry, "user"));
    let to = object_at(entry, "toUser");
    let from_id = from.and_then(user_id).unwrap_or_default();
    let to_id = to.and_then(user_id).unwrap_or_default();
    let outgoing = from_id == self_id;
    let peer = if outgoing { to } else { from };
    let last =
        entry.get("lastMsg").or_else(|| entry.get("lastMessage")).or_else(|| entry.get("msg"));
    SocialConversation {
        peer_id: if outgoing { to_id } else { from_id },
        peer_nickname: peer.map(|user| text_at(user, "nickname")).unwrap_or_default(),
        peer_avatar_url: peer.and_then(|user| url_at(user, "avatarUrl")),
        last_message: message_text(last),
        from_me: outgoing,
        unread: u32::try_from(count_at(entry, "unreadCount"))
            .unwrap_or_default()
            .max(u32::try_from(count_at(entry, "unread")).unwrap_or_default()),
        time_ms: time_at(entry, &["time", "updateTime"])
            .max(last.map(|value| time_at(value, &["time"])).unwrap_or_default()),
    }
}

pub(super) fn decode_message_page(body: &Value, self_id: i64) -> Result<SocialMessagePage> {
    let entries = array_at(body, &["msgs"]).ok_or(Error::InvalidResponse)?;
    let mut messages = Vec::with_capacity(entries.len());
    for entry in entries {
        let from = object_at(entry, "fromUser");
        let from_id = from.and_then(user_id).unwrap_or_default();
        messages.push(SocialMessage {
            id: number_as_i64(&entry["id"])
                .or_else(|| number_as_i64(&entry["msgId"]))
                .unwrap_or_default(),
            from_user_id: from_id,
            from_nickname: from.map(|user| text_at(user, "nickname")).unwrap_or_default(),
            from_avatar_url: from.and_then(|user| url_at(user, "avatarUrl")),
            content: message_text(entry.get("msg").or_else(|| entry.get("content"))),
            time_ms: time_at(entry, &["time"]),
            from_me: from_id == self_id,
        });
    }
    let has_more = flag_at(body, "more") || flag_at(body, "hasMore");
    Ok(SocialMessagePage { messages, has_more })
}

pub(super) fn decode_notice_page(body: &Value) -> Result<SocialNoticePage> {
    let entries = array_at(body, &["notices", "msgs"]).ok_or(Error::InvalidResponse)?;
    let notices =
        entries.iter().map(|entry| decode_notice(entry, SocialNoticeKind::Notice)).collect();
    Ok(SocialNoticePage { notices, has_more: flag_at(body, "more") })
}

pub(super) fn decode_comment_notice_page(body: &Value) -> Result<SocialNoticePage> {
    let data = object_at(body, "data").unwrap_or(body);
    let entries = array_at(data, &["comments", "notices"]).ok_or(Error::InvalidResponse)?;
    let notices =
        entries.iter().map(|entry| decode_notice(entry, SocialNoticeKind::Comment)).collect();
    Ok(SocialNoticePage { notices, has_more: flag_at(data, "more") || flag_at(body, "more") })
}

fn decode_notice(entry: &Value, kind: SocialNoticeKind) -> SocialNotice {
    let comment = object_at(entry, "comment").unwrap_or(entry);
    let user = object_at(comment, "user").or_else(|| object_at(entry, "user"));
    let resource = object_at(entry, "resource");
    SocialNotice {
        id: number_as_i64(&entry["id"])
            .or_else(|| number_as_i64(&comment["id"]))
            .or_else(|| number_as_i64(&comment["commentId"]))
            .unwrap_or_default(),
        kind,
        title: resource.map(|value| text_at(value, "title")).unwrap_or_default(),
        content: nonempty(&[text_at(comment, "content"), text_at(comment, "msg")]),
        user_id: user.and_then(user_id).unwrap_or_default(),
        nickname: user.map(|value| text_at(value, "nickname")).unwrap_or_default(),
        avatar_url: user.and_then(|value| url_at(value, "avatarUrl")),
        comment_id: number_as_i64(&comment["commentId"])
            .or_else(|| number_as_i64(&comment["id"]))
            .unwrap_or_default(),
        time_ms: time_at(comment, &["time"]).max(time_at(entry, &["time"])),
    }
}

fn next_before(body: &Value, last: Option<i64>, more: bool) -> Option<i64> {
    if !more {
        return None;
    }
    body.get("lasttime")
        .and_then(number_as_i64)
        .filter(|time| *time > 0)
        .or_else(|| last.map(|time| time.saturating_sub(1)))
}

fn object_at<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|value| value.is_object())
}

fn array_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    keys.iter().find_map(|key| value.get(*key).and_then(Value::as_array))
}

fn user_id(user: &Value) -> Option<i64> {
    number_as_i64(&user["userId"]).or_else(|| number_as_i64(&user["id"]))
}

fn text_at(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn url_at(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).filter(|url| !url.is_empty()).map(str::to_owned)
}

fn flag_at(value: &Value, key: &str) -> bool {
    let Some(flag) = value.get(key) else { return false };
    flag.as_bool()
        .or_else(|| flag.as_i64().map(|number| number != 0))
        .or_else(|| flag.as_f64().map(|number| number != 0.0))
        .unwrap_or_else(|| matches!(flag.as_str(), Some("true" | "1")))
}

fn count_at(value: &Value, key: &str) -> u64 {
    let Some(count) = value.get(key) else { return 0 };
    if let Some(count) = count.as_u64() {
        return count;
    }
    if let Some(count) = count.as_f64() {
        return float_count(count);
    }
    count.as_str().and_then(|raw| raw.trim().parse().ok()).unwrap_or(0)
}

fn time_at(value: &Value, keys: &[&str]) -> i64 {
    keys.iter().find_map(|key| value.get(*key).and_then(number_as_i64)).unwrap_or_default()
}

fn message_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => unwrap_envelope(text),
        Some(Value::Object(entry)) => inner_text(entry).unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

/// Some private messages arrive as a JSON *string* holding an envelope — `{"msg":"…","song":{…}}`
/// — where what a reader sees is the inner `msg` and the rest is an attachment card. The
/// conversation preview and the thread both decode through here, so unwrapping once keeps the
/// raw body off the screen. A body that is not that envelope passes through untouched.
fn unwrap_envelope(text: &str) -> String {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') {
        return text.to_owned();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(entry)) => {
            inner_text(&entry).map_or_else(|| text.to_owned(), str::to_owned)
        }
        _ => text.to_owned(),
    }
}

fn inner_text(entry: &serde_json::Map<String, Value>) -> Option<&str> {
    ["msg", "content"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str).filter(|text| !text.is_empty()))
}

fn artist_names(song: &Value) -> String {
    let artists = song.get("ar").or_else(|| song.get("artists")).and_then(Value::as_array);
    artists
        .map(|artists| {
            artists
                .iter()
                .map(|artist| text_at(artist, "name"))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default()
}

fn nonempty(candidates: &[String]) -> String {
    candidates.iter().find(|text| !text.is_empty()).cloned().unwrap_or_default()
}

/// Counts arrive as JSON floats often enough that a strict integer field would reject a page the
/// view can render. The fraction is noise, and anything unreadable reads as zero.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts sit far below 2^53; clamping keeps the cast defined"
)]
fn float_count(value: f64) -> u64 {
    value.max(0.0) as u64
}

#[cfg(test)]
#[path = "../tests/social_tests.rs"]
mod tests;
