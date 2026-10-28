use serde_json::json;

use crate::netease::Result;

use super::{
    decode_comment_notice_page, decode_conversation_page, decode_event_page, decode_message_page,
    decode_notice_page, decode_playlist_page, decode_profile, decode_subcount,
};

#[test]
fn profile_reads_counts_floats_and_strings_around_a_nullable_signature() -> Result<()> {
    let body = json!({
        "code": 200,
        "profile": {
            "userId": 42,
            "nickname": "Neko",
            "signature": null,
            "avatarUrl": "https://p1.music.126.net/avatar.jpg",
            "backgroundUrl": "",
            "follows": "12",
            "followers": 3.0,
            "playlistCount": 4,
            "eventCount": 7,
            "followed": 1,
            "mutual": "true",
            "gender": 2.0,
            "level": 10,
            "vipType": 0
        }
    });

    let profile = decode_profile(&body)?;

    assert_eq!(profile.id, 42);
    assert_eq!(profile.nickname, "Neko");
    assert_eq!(profile.signature, "");
    assert_eq!(profile.avatar_url.as_deref(), Some("https://p1.music.126.net/avatar.jpg"));
    assert_eq!(profile.background_url, None);
    assert_eq!(profile.follows, 12);
    assert_eq!(profile.followers, 3);
    assert_eq!(profile.playlist_count, 4);
    assert_eq!(profile.event_count, 7);
    assert!(profile.followed);
    assert!(profile.mutual);
    assert_eq!(profile.gender, 2);
    assert_eq!(profile.level, 10);
    Ok(())
}

#[test]
fn profile_accepts_a_top_level_object_and_refuses_a_body_without_an_id() -> Result<()> {
    let top_level = json!({"userId": "77", "nickname": "Plain"});
    assert_eq!(decode_profile(&top_level)?.id, 77);

    assert!(decode_profile(&json!({"code": 200, "profile": {"nickname": "Nameless"}})).is_err());
    Ok(())
}

#[test]
fn subcount_reads_the_wrapped_totals_and_the_older_fans_alias() {
    let body = json!({
        "data": {
            "playlistCount": "3",
            "eventCount": 4.0,
            "followCount": 5,
            "followeds": 6
        }
    });

    let subcount = decode_subcount(&body);

    assert_eq!(subcount.playlist_count, 3);
    assert_eq!(subcount.event_count, 4);
    assert_eq!(subcount.follow_count, 5);
    assert_eq!(subcount.fans_count, 6);
}

#[test]
fn a_track_event_flattens_its_json_payload_and_keeps_the_wire_cursor() -> Result<()> {
    let body = json!({
        "more": true,
        "lasttime": 1_600_000_000_000_i64,
        "events": [{
            "id": 9_001,
            "eventTime": 1_500_000_000_000_i64,
            "json": "{\"msg\":\"Listening\",\"song\":{\"id\":7,\"name\":\"Song\",\"ar\":[{\"name\":\"A\"},{\"name\":\"B\"}],\"al\":{\"picUrl\":\"https://p1.music.126.net/a.jpg\"}}}"
        }]
    });

    let page = decode_event_page(&body)?;

    assert_eq!(page.events.len(), 1);
    let event = &page.events[0];
    assert_eq!(event.id, 9_001);
    assert_eq!(event.time_ms, 1_500_000_000_000);
    assert_eq!(event.title, "Song");
    assert_eq!(event.text, "Listening");
    assert_eq!(event.track_id, 7);
    assert_eq!(event.track_title, "Song");
    assert_eq!(event.track_artist, "A / B");
    assert_eq!(event.cover_url.as_deref(), Some("https://p1.music.126.net/a.jpg"));
    assert_eq!(page.next_before, Some(1_600_000_000_000));
    Ok(())
}

#[test]
fn a_playlist_or_text_event_decodes_without_a_song() -> Result<()> {
    let body = json!({
        "more": true,
        "event": [
            {
                "id": 1,
                "eventTime": 50,
                "json": "{\"playlist\":{\"id\":2,\"name\":\"List\",\"coverImgUrl\":\"https://p1.music.126.net/l.jpg\"}}"
            },
            {"id": 2, "eventTime": 10, "json": "{\"msg\":\"Plain words\"}"}
        ]
    });

    let page = decode_event_page(&body)?;

    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].title, "List");
    assert_eq!(page.events[0].playlist_id, 2);
    assert_eq!(page.events[0].playlist_title, "List");
    assert_eq!(page.events[0].track_id, 0);
    assert_eq!(page.events[1].title, "");
    assert_eq!(page.events[1].text, "Plain words");
    assert_eq!(page.next_before, Some(9));
    Ok(())
}

#[test]
fn an_exhausted_event_page_reports_no_cursor() -> Result<()> {
    let page = decode_event_page(&json!({"events": []}))?;

    assert_eq!(page.events.len(), 0);
    assert_eq!(page.next_before, None);
    Ok(())
}

#[test]
fn playlist_page_uses_the_shared_playlist_decoder_and_its_total() -> Result<()> {
    let body = json!({
        "more": false,
        "total": "100",
        "playlist": [
            {
                "id": 1,
                "name": "First",
                "coverImgUrl": "https://p1.music.126.net/f.jpg",
                "trackCount": "3",
                "playCount": 4.0,
                "subscribed": null
            },
            {"id": 2, "name": "Second"}
        ]
    });

    let page = decode_playlist_page(&body, 0)?;

    assert_eq!(page.total, 100);
    assert!(page.has_more);
    assert_eq!(page.playlists.len(), 2);
    assert_eq!(page.playlists[0].track_count, 3);
    assert_eq!(page.playlists[0].play_count, 4);
    assert!(!page.playlists[0].subscribed);
    assert_eq!(page.playlists[1].name, "Second");
    Ok(())
}

#[test]
fn conversations_resolve_the_peer_from_the_signed_in_account() -> Result<()> {
    let body = json!({
        "total": 3,
        "msgs": [
            {
                "fromUser": {
                    "userId": 200,
                    "nickname": "Peer",
                    "avatarUrl": "https://p1.music.126.net/p.jpg"
                },
                "toUser": {"userId": 100},
                "lastMsg": "Hi there",
                "time": 500,
                "unreadCount": 2.0
            },
            {
                "fromUser": {"userId": 100},
                "toUser": {"userId": 300, "nickname": "Other"},
                "lastMsg": {"msg": "Yo", "time": 600},
                "unread": 1
            }
        ]
    });

    let page = decode_conversation_page(&body, 100, 0)?;

    assert_eq!(page.conversations.len(), 2);
    let incoming = &page.conversations[0];
    assert_eq!(incoming.peer_id, 200);
    assert_eq!(incoming.peer_nickname, "Peer");
    assert_eq!(incoming.last_message, "Hi there");
    assert!(!incoming.from_me);
    assert_eq!(incoming.unread, 2);
    assert_eq!(incoming.time_ms, 500);

    let outgoing = &page.conversations[1];
    assert_eq!(outgoing.peer_id, 300);
    assert_eq!(outgoing.peer_nickname, "Other");
    assert_eq!(outgoing.last_message, "Yo");
    assert!(outgoing.from_me);
    assert_eq!(outgoing.unread, 1);
    assert_eq!(outgoing.time_ms, 600);
    assert!(page.has_more);
    Ok(())
}

#[test]
fn message_page_marks_our_own_messages_and_accepts_nested_text() -> Result<()> {
    let body = json!({
        "more": true,
        "msgs": [
            {
                "id": "123",
                "fromUser": {"userId": 100, "nickname": "Me"},
                "msg": "Sent",
                "time": 1
            },
            {
                "msgId": 124,
                "fromUser": {"userId": 200, "nickname": "Peer"},
                "msg": {"msg": "Received"},
                "time": 2
            },
            {
                // The shape the service actually sends for a share card: the body is a JSON
                // *string* holding an envelope, and the reader must see the inner text rather
                // than the raw body.
                "msgId": 125,
                "fromUser": {"userId": 200, "nickname": "Peer"},
                "msg": "{\"msg\":\"A new song\",\"song\":{\"name\":\"Over Shine\",\"id\":345}}",
                "time": 3
            }
        ]
    });

    let page = decode_message_page(&body, 100)?;

    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].id, 123);
    assert_eq!(page.messages[0].content, "Sent");
    assert!(page.messages[0].from_me);
    assert_eq!(page.messages[1].id, 124);
    assert_eq!(page.messages[1].content, "Received");
    assert!(!page.messages[1].from_me);
    assert_eq!(page.messages[1].from_nickname, "Peer");
    assert_eq!(page.messages[2].content, "A new song");
    assert!(page.has_more);
    Ok(())
}

#[test]
fn notices_read_the_sender_and_the_system_text() -> Result<()> {
    let body = json!({
        "more": true,
        "notices": [{
            "id": 31,
            "msg": "Welcome",
            "time": 5,
            "user": {"userId": 7, "nickname": "Admin"}
        }]
    });

    let page = decode_notice_page(&body)?;

    assert_eq!(page.notices.len(), 1);
    let notice = &page.notices[0];
    assert_eq!(notice.id, 31);
    assert_eq!(notice.content, "Welcome");
    assert_eq!(notice.user_id, 7);
    assert_eq!(notice.nickname, "Admin");
    assert_eq!(notice.time_ms, 5);
    assert!(page.has_more);
    Ok(())
}

#[test]
fn comment_notifications_unwrap_the_comment_and_its_resource_title() -> Result<()> {
    let body = json!({
        "data": {
            "more": false,
            "comments": [{
                "comment": {
                    "commentId": "55",
                    "content": "Nice track",
                    "time": 9,
                    "user": {
                        "userId": 8,
                        "nickname": "Fan",
                        "avatarUrl": "https://p1.music.126.net/fan.jpg"
                    }
                },
                "resource": {"title": "A Song"}
            }]
        }
    });

    let page = decode_comment_notice_page(&body)?;

    assert_eq!(page.notices.len(), 1);
    let notice = &page.notices[0];
    assert_eq!(notice.id, 55);
    assert_eq!(notice.comment_id, 55);
    assert_eq!(notice.title, "A Song");
    assert_eq!(notice.content, "Nice track");
    assert_eq!(notice.user_id, 8);
    assert_eq!(notice.nickname, "Fan");
    assert_eq!(notice.time_ms, 9);
    assert!(!page.has_more);
    Ok(())
}
