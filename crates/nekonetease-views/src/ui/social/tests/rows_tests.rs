use nekonetease_core::entities::cloud::{Playlist, PlaylistCreator};
use nekonetease_core::entities::social::{
    SocialConversation, SocialEvent, SocialEventKind, SocialMessage, SocialNotice, SocialNoticeKind,
};

use super::{
    adjust_follow_count, conversation_row, event_row, message_row, notice_row, playlist_row,
};

#[test]
fn follow_count_moves_within_bounds() {
    assert_eq!(adjust_follow_count("12", 1), "13");
    assert_eq!(adjust_follow_count("12", -1), "11");
    assert_eq!(adjust_follow_count("0", -1), "0");
    assert_eq!(adjust_follow_count(" 7", 1), " 7", "an unparseable label is left alone");
}

#[test]
fn playlist_row_carries_the_creator_beside_the_count() {
    let playlist = Playlist {
        id: 88,
        name: "Night drive".into(),
        track_count: 12,
        creator: Some(PlaylistCreator { nickname: "Neko".into(), ..Default::default() }),
        ..Default::default()
    };
    let row = playlist_row(&playlist);
    assert_eq!(row.id, "88");
    assert_eq!(row.title, "Night drive");
    assert_eq!(row.subtitle, "12 · Neko");
    assert_eq!(row.artwork_path, "", "a missing cover leaves the placeholder");
}

#[test]
fn event_row_names_its_kind() {
    for (kind, expected) in [
        (SocialEventKind::Track, "track"),
        (SocialEventKind::Playlist, "playlist"),
        (SocialEventKind::Text, "text"),
        (SocialEventKind::Unknown, "unknown"),
    ] {
        let event = SocialEvent { id: 4, kind, ..Default::default() };
        assert_eq!(event_row(&event).kind, expected);
    }
}

#[test]
fn event_row_keeps_a_remote_cover_address() {
    let event = SocialEvent {
        id: 5,
        cover_url: Some("https://p1.music.126.net/cover.jpg".into()),
        ..Default::default()
    };
    assert_eq!(event_row(&event).artwork_path, "https://p1.music.126.net/cover.jpg");
}

#[test]
fn conversation_row_projects_the_peer_side() {
    let conversation = SocialConversation {
        peer_id: 9_000_000_001,
        peer_nickname: "Friend".into(),
        last_message: "see you".into(),
        from_me: false,
        unread: 3,
        time_ms: 1_700_000_000_000,
        ..Default::default()
    };
    let row = conversation_row(&conversation);
    assert_eq!(row.peer_id, "9000000001", "an id past i32 is text, not a clamped int");
    assert_eq!(row.nickname, "Friend");
    assert_eq!(row.last_message, "see you");
    assert_eq!(row.unread, 3);
    assert!(!row.from_me);
    assert_ne!(row.display_time, "");
}

#[test]
fn message_row_marks_our_own_message() {
    let message = SocialMessage {
        id: 77,
        from_nickname: "Me".into(),
        content: "hello".into(),
        from_me: true,
        ..Default::default()
    };
    let row = message_row(&message);
    assert_eq!(row.id, "77");
    assert_eq!(row.content, "hello");
    assert!(row.from_me);
}

#[test]
fn notice_row_only_names_a_user_it_can_open() {
    let anonymous = SocialNotice { id: 1, user_id: 0, ..Default::default() };
    assert_eq!(notice_row(&anonymous).user_id, "");

    let comment = SocialNotice {
        id: 2,
        kind: SocialNoticeKind::Comment,
        user_id: 42,
        title: "replied".into(),
        ..Default::default()
    };
    let row = notice_row(&comment);
    assert_eq!(row.kind, "comment");
    assert_eq!(row.user_id, "42");
}
