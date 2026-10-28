use serde_json::json;

use nekonetease_core::entities::cloud::{Mv, MvStream};

use crate::netease::Result;
use crate::netease::endpoints::Protocol;

use super::decode::decode_ref;
use super::ep;
use super::mv::{
    decode_mv_detail, decode_thread_info, mv_like_body, mv_subscribe_body, mv_url_body,
};

#[test]
fn mv_detail_reads_artist_count_and_publish_time() -> Result<()> {
    let data = json!({
        "id": 5_436_712,
        "name": "Example",
        "cover": "https://p1.music.126.net/cover.jpg",
        "artistId": 6_452,
        "artistName": "Singer",
        "duration": 215_000.0,
        "playCount": "25370443800",
        "publishTime": 1_600_000_000_000_u64,
        "desc": "A description"
    });

    let mv: Mv = decode_ref(&data)?;

    assert_eq!(mv.id, 5_436_712);
    assert_eq!(mv.name, "Example");
    assert_eq!(mv.artist_id, 6_452);
    assert_eq!(mv.artist_name, "Singer");
    assert_eq!(mv.duration_ms, 215_000);
    assert_eq!(mv.play_count, 25_370_443_800);
    assert_eq!(mv.publish_time_ms, 1_600_000_000_000);
    assert_eq!(mv.description, "A description");
    assert_eq!(mv.cover_url.as_deref(), Some("https://p1.music.126.net/cover.jpg"));
    Ok(())
}

#[test]
fn artist_listing_rows_survive_missing_detail_fields() -> Result<()> {
    let rows = json!([
        {"id": 1, "name": "First", "cover": "https://p1.music.126.net/a.jpg", "playCount": 10},
        {"id": 2, "name": "Second", "artistName": null, "duration": 60_000},
    ]);

    let mvs: Vec<Mv> = decode_ref(&rows)?;

    assert_eq!(mvs.len(), 2);
    assert_eq!(mvs[0].id, 1);
    assert_eq!(mvs[0].play_count, 10);
    assert_eq!(mvs[0].artist_id, 0);
    assert_eq!(mvs[0].description, "");
    assert_eq!(mvs[1].artist_name, "");
    assert_eq!(mvs[1].duration_ms, 60_000);
    Ok(())
}

#[test]
fn mv_stream_keeps_an_absent_url_absent() -> Result<()> {
    let with_url = json!({"url": "https://m10.music.126.net/video.mp4"});
    assert_eq!(
        decode_ref::<MvStream>(&with_url)?.url.as_deref(),
        Some("https://m10.music.126.net/video.mp4")
    );

    let without = json!({"url": null});
    assert_eq!(decode_ref::<MvStream>(&without)?.url, None);
    Ok(())
}

#[test]
fn mv_routes_are_read_only_weapi() {
    for endpoint in [ep::MV_DETAIL, ep::MV_URL, ep::ARTIST_MVS, ep::SIMILAR_MVS] {
        assert_eq!(endpoint.protocol, Protocol::Weapi);
        assert!(endpoint.read_only);
    }
    assert_eq!(ep::MV_DETAIL.path, "/api/v1/mv/detail");
    assert_eq!(ep::MV_URL.path, "/api/song/enhance/play/mv/url");
    assert_eq!(ep::ARTIST_MVS.path, "/api/artist/mvs");
    assert_eq!(ep::SIMILAR_MVS.path, "/api/discovery/simiMV");
}

#[test]
fn mv_stream_request_carries_the_chosen_quality() {
    assert_eq!(mv_url_body(7, 720), json!({"id": 7, "r": 720}));
}

#[test]
fn mv_detail_reads_the_action_counts_and_the_envelope_collect_state() -> Result<()> {
    let body = json!({
        "subed": true,
        "data": {
            "id": 7,
            "name": "Counted",
            "subCount": 12,
            "shareCount": "34",
            "commentCount": 56,
        },
    });

    let mv: Mv = decode_mv_detail(&body)?;

    assert_eq!(mv.sub_count, 12);
    assert_eq!(mv.share_count, 34);
    assert_eq!(mv.comment_count, 56);
    assert!(mv.subscribed);
    Ok(())
}

#[test]
fn mv_detail_defaults_counters_and_collect_state_when_the_wire_omits_them() -> Result<()> {
    let mv: Mv = decode_mv_detail(&json!({"data": {"id": 7, "name": "Bare"}}))?;

    assert_eq!(mv.sub_count, 0);
    assert_eq!(mv.share_count, 0);
    assert_eq!(mv.comment_count, 0);
    assert!(!mv.subscribed);
    Ok(())
}

#[test]
fn an_absent_mv_detail_row_is_an_invalid_response() {
    assert!(decode_mv_detail(&json!({"code": 200, "data": null})).is_err());
}

#[test]
fn mv_thread_info_reads_both_envelope_shapes() {
    let flat = decode_thread_info(&json!({"liked": true, "likedCount": 9}));
    assert!(flat.liked);
    assert_eq!(flat.liked_count, 9);

    let nested = decode_thread_info(&json!({"data": {"liked": false, "likedCount": "3"}}));
    assert!(!nested.liked);
    assert_eq!(nested.liked_count, 3);

    let absent = decode_thread_info(&json!({"code": 200}));
    assert!(!absent.liked);
    assert_eq!(absent.liked_count, 0);
}

#[test]
fn mv_action_routes_are_weapi_writes() {
    assert_eq!(ep::MV_SUB.path, "/api/mv/sub");
    assert_eq!(ep::MV_UNSUB.path, "/api/mv/unsub");
    assert_eq!(ep::MV_LIKE.path, "/api/resource/like");
    assert_eq!(ep::MV_UNLIKE.path, "/api/resource/unlike");
    for endpoint in [ep::MV_SUB, ep::MV_UNSUB, ep::MV_LIKE, ep::MV_UNLIKE] {
        assert_eq!(endpoint.protocol, Protocol::Weapi);
        assert!(!endpoint.read_only);
    }
    assert_eq!(ep::mv_subscription(true).path, "/api/mv/sub");
    assert_eq!(ep::mv_subscription(false).path, "/api/mv/unsub");
    assert_eq!(ep::mv_like_endpoint(true).path, "/api/resource/like");
    assert_eq!(ep::mv_like_endpoint(false).path, "/api/resource/unlike");
}

#[test]
fn the_thread_info_route_is_a_public_weapi_read() {
    assert_eq!(ep::COMMENT_THREAD_INFO.path, "/api/comment/commentthread/info");
    assert_eq!(ep::COMMENT_THREAD_INFO.protocol, Protocol::Weapi);
    const { assert!(ep::COMMENT_THREAD_INFO.read_only) };
}

#[test]
fn the_action_bodies_name_the_mv_once_as_an_id_and_once_as_a_thread() {
    assert_eq!(mv_like_body(7), json!({"threadId": "R_MV_5_7"}));
    assert_eq!(mv_subscribe_body(7), json!({"mvId": 7, "mvIds": "[\"7\"]"}));
}
