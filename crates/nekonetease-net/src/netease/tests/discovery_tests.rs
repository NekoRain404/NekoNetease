use super::discovery::{
    comment_like_payload, decode_cloud_playlist_page, decode_comment_page, decode_comments,
    decode_high_quality_playlists, decode_hot_search, decode_playlist_categories,
    decode_playlist_tag_categories, decode_playlist_tags, decode_posted_comment,
    decode_search_suggestions, decode_thread_comment,
};
use super::ep;
use crate::netease::endpoints::Protocol;
use nekonetease_core::entities::cloud::CommentThread;
use nekonetease_core::entities::comment::CommentSort;
use serde_json::json;

#[test]
fn hot_search_accepts_the_upstream_search_hot_shape_and_preserves_order()
-> crate::netease::Result<()> {
    let rows = decode_hot_search(&json!({
        "result": {"hots": [
            {"first": "Hot one", "second": 1, "third": null, "iconType": 1},
            {"first": "Hot two", "second": 72, "third": "New album"}
        ]}
    }))?;

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].word, "Hot one");
    assert_eq!(rows[0].content.len(), 0);
    assert_eq!(rows[0].score, 1);
    assert_eq!(rows[1].word, "Hot two");
    assert_eq!(rows[1].content, "New album");
    assert_eq!(rows[1].score, 72);
    Ok(())
}

#[test]
fn hot_search_rejects_a_response_without_entries() {
    assert!(decode_hot_search(&json!({"result": {}})).is_err());
}

#[test]
fn playlist_categories_decode_nested_entries_in_upstream_order() -> crate::netease::Result<()> {
    let rows = decode_playlist_categories(&json!({
        "categories": {"0": "语种", "1": "风格"},
        "sub": [
            {"category": 0, "name": "华语", "resourceCount": 600, "hot": true},
            {"category": 1, "name": "摇滚", "resourceCount": 200}
        ]
    }))?;

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "华语");
    assert_eq!(rows[0].group, "语种");
    assert_eq!(rows[0].resource_count, 600);
    assert!(rows[0].hot);
    assert_eq!(rows[1].group, "风格");
    Ok(())
}

#[test]
fn hot_categories_keep_the_api_order_and_playlist_counts() -> crate::netease::Result<()> {
    let rows = decode_playlist_tag_categories(
        &json!({
            "tags": [
                {"name": "华语", "usedCount": 321},
                {"name": "摇滚", "usedCount": 123}
            ]
        }),
        true,
    )?;

    assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), ["华语", "摇滚"]);
    assert_eq!(rows[0].group, "热门");
    assert_eq!(rows[0].resource_count, 321);
    assert!(rows.iter().all(|row| row.hot));
    Ok(())
}

#[test]
fn high_quality_tags_decode_names_without_reordering() -> crate::netease::Result<()> {
    assert_eq!(
        decode_playlist_tags(&json!({"tags": [{"name": "电子"}, {"name": "摇滚"}]}))?,
        ["电子", "摇滚"]
    );
    assert!(decode_playlist_tags(&json!({"tags": [{}]})).is_err());
    Ok(())
}

#[test]
fn playlist_page_preserves_large_remote_ids_and_more_flag() -> crate::netease::Result<()> {
    let (rows, more) = decode_cloud_playlist_page(&json!({
        "more": true,
        "playlists": [
            {"id": 4_294_967_301_i64, "name": "Large id", "coverImgUrl": "https://p1.music.126.net/a.jpg", "trackCount": 31}
        ]
    }))?;

    assert_eq!(rows[0].id, 4_294_967_301);
    assert_eq!(rows[0].cover_url.as_deref(), Some("https://p1.music.126.net/a.jpg"));
    assert_eq!(rows[0].track_count, 31);
    assert!(more);
    Ok(())
}

#[test]
fn high_quality_pages_use_the_last_update_cursor_and_detect_end() -> crate::netease::Result<()> {
    let (rows, next_before, more) = decode_high_quality_playlists(&json!({
        "more": true,
        "lasttime": 777,
        "playlists": [
            {"id": 8_000_000_001_i64, "name": "Newest", "updateTime": 2_000},
            {"id": 8_000_000_002_i64, "name": "Older", "updateTime": 1_000}
        ]
    }))?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, 8_000_000_001);
    assert_eq!(next_before, Some(777));
    assert!(more);

    let (_, next_before, more) = decode_high_quality_playlists(&json!({
        "more": false,
        "playlists": [{"id": 5, "name": "Last", "updateTime": 500}]
    }))?;
    assert_eq!(next_before, Some(500));
    assert!(!more);
    Ok(())
}

#[test]
fn discovery_endpoints_use_verified_wire_routes_and_protocols() {
    assert_eq!(ep::HOT_SEARCH.path, "/api/search/hot");
    assert_eq!(ep::HOT_SEARCH.protocol, Protocol::Weapi);
    assert_eq!(ep::PLAYLIST_CATEGORIES.path, "/api/playlist/catalogue");
    assert_eq!(ep::PLAYLIST_CATEGORIES.protocol, Protocol::Eapi);
    assert_eq!(ep::HOT_PLAYLIST_CATEGORIES.path, "/api/playlist/hottags");
    assert_eq!(ep::HOT_PLAYLIST_CATEGORIES.protocol, Protocol::Weapi);
    assert_eq!(ep::CATEGORY_PLAYLISTS.path, "/api/playlist/list");
    assert_eq!(ep::CATEGORY_PLAYLISTS.protocol, Protocol::Weapi);
    assert_eq!(ep::HIGH_QUALITY_PLAYLISTS.path, "/api/playlist/highquality/list");
    assert_eq!(ep::HIGH_QUALITY_PLAYLISTS.protocol, Protocol::Weapi);
    assert_eq!(ep::HIGH_QUALITY_TAGS.path, "/api/playlist/highquality/tags");
    assert_eq!(ep::HIGH_QUALITY_TAGS.protocol, Protocol::Weapi);
    assert_eq!(ep::PLAYLIST_COMMENTS.path, "/api/v1/resource/comments/A_PL_0");
    assert_eq!(ep::PLAYLIST_COMMENTS.protocol, Protocol::Weapi);
    assert_eq!(ep::playlist_comments_path(42), "/api/v1/resource/comments/A_PL_0_42");
    assert_eq!(ep::COMMENTS.path, "/api/v1/resource/comments");
    assert_eq!(ep::COMMENTS.protocol, Protocol::Weapi);
}

#[test]
fn every_comment_thread_builds_its_resource_route() {
    for (thread, expected) in [
        (CommentThread::Song(7), "/api/v1/resource/comments/R_SO_4_7"),
        (CommentThread::Album(7), "/api/v1/resource/comments/R_AL_3_7"),
        (CommentThread::Playlist(7), "/api/v1/resource/comments/A_PL_0_7"),
    ] {
        assert_eq!(ep::comments_path(&thread.wire_id()), expected);
    }
}

#[test]
fn playlist_comments_decode_author_content_and_more_flag() -> crate::netease::Result<()> {
    let (rows, more) = decode_comments(
        &json!({
            "more": true,
            "comments": [{
                "commentId": 4_294_967_301_i64,
                "content": "Great mix",
                "time": 1_700_000_000_000_i64,
                "liked": true,
                "likedCount": 12,
                "user": {"nickname":"Listener"}
            }]
        }),
        20,
    )?;
    assert_eq!(rows[0].id, 4_294_967_301);
    assert_eq!(rows[0].author, "Listener");
    assert_eq!(rows[0].content, "Great mix");
    assert_eq!(rows[0].time_ms, 1_700_000_000_000);
    assert!(rows[0].liked);
    assert_eq!(rows[0].liked_count, 12);
    assert!(more);
    Ok(())
}

#[test]
fn comment_like_routes_use_weapi_and_the_typed_thread_id() {
    assert_eq!(ep::COMMENT_LIKE.path, "/api/comment/like");
    assert_eq!(ep::COMMENT_UNLIKE.path, "/api/comment/unlike");
    assert_eq!(ep::COMMENT_LIKE.protocol, Protocol::Weapi);
    const { assert!(!ep::COMMENT_LIKE.read_only) };
    assert_eq!(ep::comment_like_endpoint(true).path, "/api/comment/like");
    assert_eq!(ep::comment_like_endpoint(false).path, "/api/comment/unlike");

    let payload = comment_like_payload(CommentThread::Album(42), 7_000_000_000);
    assert_eq!(payload["threadId"], "R_AL_3_42");
    assert_eq!(payload["commentId"], 7_000_000_000_i64);
    assert_eq!(payload["csrf_token"], "");
}

#[test]
fn comments_decode_every_thread_with_the_requested_page_limit() -> crate::netease::Result<()> {
    let four = json!({"comments": [
        {"content": "1", "time": 1, "user": {"nickname": "A"}},
        {"content": "2", "time": 2, "user": {"nickname": "B"}},
        {"content": "3", "time": 3, "user": {"nickname": "C"}},
        {"content": "4", "time": 4, "user": {"nickname": "D"}}
    ]});
    let (rows, more) = decode_comments(&four, 4)?;
    assert_eq!(rows.len(), 4);
    assert!(more, "a full page with no more flag implies another page");

    let (rows, more) = decode_comments(
        &json!({"comments": [{"content": "Only", "user": {"remarkName": "Alias"}}]}),
        4,
    )?;
    assert_eq!(rows[0].author, "Alias");
    assert!(!more);
    Ok(())
}

#[test]
fn search_suggestions_keep_server_order_without_repeating_a_label() {
    let labels = decode_search_suggestions(&json!({
        "result": {
            "songs": [{"name": "Song A"}, {"name": "Song B"}, {"name": "  "}],
            "artists": [{"name": "Song A"}, {"name": "Artist C"}],
            "albums": [
                {"name": "Album D"},
                {"name": null, "title": "Album by title"},
                {"title": "Album by title fallback"}
            ],
            "playlists": [{"name": "Playlist E"}, {}]
        }
    }));

    assert_eq!(
        labels,
        [
            "Song A",
            "Song B",
            "Artist C",
            "Album D",
            "Album by title",
            "Album by title fallback",
            "Playlist E"
        ]
    );
}

#[test]
fn search_suggestions_stop_at_the_eighth_label() {
    let labels = decode_search_suggestions(&json!({
        "result": {"songs": [
            {"name": "1"}, {"name": "2"}, {"name": "3"}, {"name": "4"}, {"name": "5"},
            {"name": "6"}, {"name": "7"}, {"name": "8"}, {"name": "9"}
        ]}
    }));

    assert_eq!(labels.len(), 8);
    assert_eq!(labels.last().map(String::as_str), Some("8"));
}

#[test]
fn search_suggestions_decode_to_nothing_without_a_result_object() {
    assert_eq!(decode_search_suggestions(&json!({})), Vec::<String>::new());
    assert_eq!(decode_search_suggestions(&json!({"result": {}})), Vec::<String>::new());
}

#[test]
fn search_suggestion_route_uses_the_verified_wire_route_and_protocol() {
    assert_eq!(ep::SEARCH_SUGGEST.path, "/api/search/suggest/web");
    assert_eq!(ep::SEARCH_SUGGEST.protocol, Protocol::Weapi);
    const { assert!(ep::SEARCH_SUGGEST.read_only) };
}

fn comment_row(id: i64, content: &str) -> serde_json::Value {
    json!({
        "commentId": id,
        "content": content,
        "time": 1_700_000_000_000_i64,
        "liked": false,
        "likedCount": 3,
        "user": {"userId": 9, "nickname": "Listener"},
    })
}

#[test]
fn time_pages_preserve_the_server_order_and_report_the_ordering() -> crate::netease::Result<()> {
    let page = decode_comment_page(
        &json!({
            "comments": [comment_row(11, "first"), comment_row(12, "second"), comment_row(13, "third")],
            "total": 40,
            "more": true,
        }),
        CommentSort::Time,
        20,
    )?;

    assert!(!page.hot);
    assert!(page.more);
    assert_eq!(page.total, 40);
    assert_eq!(page.comments.iter().map(|row| row.comment.id).collect::<Vec<_>>(), [11, 12, 13]);
    assert_eq!(page.comments[1].comment.content, "second");
    assert_eq!(page.comments[0].author_id, 9);
    Ok(())
}

#[test]
fn hot_pages_read_the_hot_key_under_data_and_mark_themselves_hot() -> crate::netease::Result<()> {
    let page = decode_comment_page(
        &json!({
            "code": 200,
            "data": {
                "hotComments": [comment_row(21, "liked one"), comment_row(22, "liked two")],
                "total": 2,
                "more": false,
            }
        }),
        CommentSort::Hot,
        20,
    )?;

    assert!(page.hot);
    assert!(!page.more);
    assert_eq!(page.total, 2);
    assert_eq!(page.comments[0].comment.id, 21);
    assert_eq!(page.comments[1].comment.id, 22);
    Ok(())
}

#[test]
fn a_page_without_flags_falls_back_to_what_its_length_implies() -> crate::netease::Result<()> {
    let full = decode_comment_page(
        &json!({"comments": [comment_row(1, "a"), comment_row(2, "b")]}),
        CommentSort::Time,
        2,
    )?;
    assert!(full.more);
    assert_eq!(full.total, 2);

    let short =
        decode_comment_page(&json!({"comments": [comment_row(1, "a")]}), CommentSort::Time, 2)?;
    assert!(!short.more);
    Ok(())
}

#[test]
fn a_page_without_an_array_is_rejected() {
    assert!(decode_comment_page(&json!({"code": 200}), CommentSort::Time, 20).is_err());
}

#[test]
fn a_floor_row_keeps_its_quote_and_reply_count() {
    let row = decode_thread_comment(&json!({
        "commentId": 31,
        "content": "replying",
        "user": {"userId": 5, "nickname": "Replier"},
        "beReplied": [{"commentId": 30, "content": "quoted"}],
        "showFloorComment": {"replyCount": 4},
    }));

    assert_eq!(row.parent_id, 30);
    assert_eq!(row.reply_count, 4);
    assert_eq!(row.comment.id, 31);
    assert_eq!(row.comment.author, "Replier");
    assert_eq!(row.author_id, 5);
}

#[test]
fn a_comment_row_carries_the_author_avatar_the_wire_supplies() {
    let row = decode_thread_comment(&json!({
        "user": {"avatarUrl": "https://p1.music.126.net/avatar.jpg"},
    }));

    assert_eq!(row.avatar_url, "https://p1.music.126.net/avatar.jpg");
}

#[test]
fn a_null_or_absent_avatar_decodes_to_an_empty_url() {
    let null = decode_thread_comment(&json!({"user": {"avatarUrl": null}}));
    assert_eq!(null.avatar_url, "");

    let absent = decode_thread_comment(&json!({"user": {"nickname": "Nameless"}}));
    assert_eq!(absent.avatar_url, "");

    let anonymous = decode_thread_comment(&json!({"content": "no author"}));
    assert_eq!(anonymous.avatar_url, "");
}

#[test]
fn posted_comment_decoding_accepts_the_top_and_nested_shapes() {
    let top = decode_posted_comment(&json!({"code": 200, "comment": comment_row(41, "sent")}))
        .map(|comment| (comment.id, comment.content));
    assert_eq!(top, Some((41, "sent".to_owned())));

    let nested =
        decode_posted_comment(&json!({"code": 200, "data": {"comment": comment_row(42, "reply")}}))
            .map(|comment| comment.id);
    assert_eq!(nested, Some(42));

    let bare_status = decode_posted_comment(&json!({"code": 200, "data": {"code": 200}}));
    assert_eq!(bare_status, None);
}

#[test]
fn comment_write_routes_are_the_ones_api_enhanced_names() {
    assert_eq!(ep::comment_hot_path("R_SO_4_42"), "/api/v1/resource/hotcomments/R_SO_4_42");
    assert_eq!(ep::COMMENT_HOT.protocol, Protocol::Weapi);
    assert_eq!(ep::COMMENT_FLOOR.path, "/api/resource/comment/floor/get");
    assert_eq!(ep::COMMENT_FLOOR.protocol, Protocol::Weapi);
    assert_eq!(ep::COMMENT_ADD.path, "/api/resource/comments/add");
    assert_eq!(ep::COMMENT_ADD.protocol, Protocol::Xeapi);
    assert_eq!(ep::COMMENT_REPLY.path, "/api/v1/resource/comments/reply");
    assert_eq!(ep::COMMENT_REPLY.protocol, Protocol::Xeapi);
    assert_eq!(ep::COMMENT_DELETE.path, "/api/resource/comments/delete");
    assert_eq!(ep::COMMENT_DELETE.protocol, Protocol::Xeapi);
    const { assert!(!ep::COMMENT_ADD.read_only) };
    const { assert!(!ep::COMMENT_REPLY.read_only) };
    const { assert!(!ep::COMMENT_DELETE.read_only) };
}
