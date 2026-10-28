use nekonetease_core::entities::cloud::Comment;
use nekonetease_core::entities::comment::ThreadComment;

use super::models::{count_to_i32, to_row};

fn thread_comment(author_id: i64, time_ms: i64) -> ThreadComment {
    ThreadComment {
        comment: Comment {
            id: i64::from(i32::MAX) + 5,
            author: "someone".to_owned(),
            content: "hello".to_owned(),
            time_ms,
            liked: true,
            liked_count: 9,
        },
        parent_id: 12,
        reply_count: 4,
        author_id,
        avatar_url: "https://p1.music.126.net/avatar.jpg".to_owned(),
    }
}

#[test]
fn a_row_carries_the_exact_wire_id_beside_the_aliased_one() {
    let row = to_row(&thread_comment(9, 1_700_000_000_000), 9);

    assert_eq!(row.id, i32::MAX);
    assert_eq!(row.remote_id.as_str(), "2147483652");
    assert_eq!(row.parent_id.as_str(), "12");
    assert_eq!(row.author.as_str(), "someone");
    assert_eq!(row.content.as_str(), "hello");
    assert!(row.liked);
    assert_eq!(row.liked_count, 9);
    assert_eq!(row.reply_count, 4);
    assert_eq!(row.avatar_url.as_str(), "https://p1.music.126.net/avatar.jpg");
}

#[test]
fn a_row_without_an_avatar_projects_an_empty_url() {
    let mut comment = thread_comment(9, 1);
    comment.avatar_url.clear();

    assert_eq!(to_row(&comment, 9).avatar_url.as_str(), "");
}

#[test]
fn only_the_author_may_delete_a_row() {
    assert!(to_row(&thread_comment(9, 1), 9).owned);
    assert!(!to_row(&thread_comment(9, 1), 10).owned);
}

#[test]
fn an_anonymous_viewer_owns_nothing() {
    assert!(!to_row(&thread_comment(0, 1), 0).owned);
}

#[test]
fn a_missing_timestamp_renders_as_no_label() {
    assert!(to_row(&thread_comment(9, 0), 9).display_time.is_empty());
}

#[test]
fn a_present_timestamp_renders_as_a_local_label() {
    let row = to_row(&thread_comment(9, 1_700_000_000_000), 9);
    assert!(row.display_time.contains('-'));
    assert!(row.display_time.contains(':'));
}

#[test]
fn a_reply_count_saturates_rather_than_wrapping() {
    assert_eq!(count_to_i32(u64::MAX), i32::MAX);
    assert_eq!(count_to_i32(7), 7);
}
