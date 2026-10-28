use super::{
    Album, Artist, Comment, CommentThread, Mv, MvStream, PlaybackRestriction, Playlist, Song,
    StreamTicket, UnavailableReason,
};
use serde_json::json;

#[test]
fn search_and_detail_song_shapes_share_identity() -> Result<(), serde_json::Error> {
    let song: Song = serde_json::from_str(
        r#"{"id":123,"name":"Song","artists":[{"id":1,"name":"Artist"}],"album":{"id":2,"name":"Album"},"duration":1000}"#,
    )?;
    assert_eq!(song.id, 123);
    assert_eq!(song.artist_names(), "Artist");
    assert_eq!(song.album.name, "Album");
    assert_eq!(song.duration_ms, 1000);
    Ok(())
}

#[test]
fn queue_metadata_contains_no_media_url() -> Result<(), serde_json::Error> {
    let song = Song { id: 9, name: "Track".into(), ..Song::default() };
    let json = serde_json::to_string(&song)?;
    assert!(!json.contains("file_path"));
    assert!(!json.contains("https:"));
    Ok(())
}

#[test]
fn a_song_detail_reports_a_missing_licence_and_its_download_right() -> Result<(), serde_json::Error>
{
    // The two fields a refusal is classified from; neither appears on the stream row.
    let song: Song = serde_json::from_value(json!({
        "id": 1_974_443_814,
        "name": "Unavailable",
        "fee": 0,
        "privilege": {"id": 1_974_443_814, "st": -200, "pl": 0, "dl": 0, "fee": 0},
        "noCopyrightRcmd": {"type": 1, "typeDesc": "无版权"}
    }))?;
    assert!(song.no_copyright);
    assert_eq!(
        song.privilege.as_ref().map(|privilege| (privilege.st, privilege.pl, privilege.dl)),
        Some((-200, 0, 0))
    );
    Ok(())
}

#[test]
fn a_null_or_absent_copyright_notice_reads_as_no_complaint() -> Result<(), serde_json::Error> {
    let null: Song =
        serde_json::from_value(json!({"id": 1, "name": "Fine", "noCopyrightRcmd": null}))?;
    let absent: Song = serde_json::from_value(json!({"id": 2, "name": "Also fine"}))?;
    assert!(!null.no_copyright);
    assert!(!absent.no_copyright);
    Ok(())
}

#[test]
fn null_artist_and_album_names_do_not_reject_a_song_batch() -> Result<(), serde_json::Error> {
    let songs: Vec<Song> = serde_json::from_value(json!([
        {"id": 41, "name": "Before"},
        {
            "id": 42, "name": "Unavailable metadata",
            "ar": [{"id": 0, "name": null}],
            "al": {"id": 0, "name": null, "picUrl": null}
        },
        {"id": 43, "name": "After"}
    ]))?;

    assert_eq!(songs.iter().map(|song| song.id).collect::<Vec<_>>(), [41, 42, 43]);
    assert_eq!(songs[1].artists[0].id, 0);
    assert_eq!(songs[1].artist_names(), "");
    assert_eq!(songs[1].album.id, 0);
    assert_eq!(songs[1].album.name, "");
    assert_eq!(songs[1].album.cover_url, None);
    Ok(())
}

#[test]
fn non_string_artist_names_remain_invalid() {
    assert!(serde_json::from_value::<Artist>(json!({"id": 1, "name": 42})).is_err());
}

#[test]
fn non_string_album_names_remain_invalid() {
    assert!(serde_json::from_value::<Album>(json!({"id": 1, "name": []})).is_err());
}

#[test]
fn missing_artist_identity_remains_invalid() {
    assert!(serde_json::from_value::<Artist>(json!({"name": null})).is_err());
}

#[test]
fn song_versions_and_album_context_survive_search_decoding() -> Result<(), serde_json::Error> {
    let song: Song = serde_json::from_value(json!({
        "id": 5, "name": "Title",
        "alia": ["Alt title"], "tns": ["翻译标题"], "mv": 77, "pop": 95
    }))?;
    assert_eq!(song.alias, ["Alt title"]);
    assert_eq!(song.translated_names, ["翻译标题"]);
    assert_eq!(song.mv, 77);
    assert_eq!(song.pop, 95);
    Ok(())
}

#[test]
fn playlist_carries_owner_tags_and_collection_state() -> Result<(), serde_json::Error> {
    let playlist: super::Playlist = serde_json::from_value(json!({
        "id": 9, "name": "Mix", "trackCount": 12, "playCount": 3456,
        "subscribed": true, "subscribedCount": 78, "tags": ["pop", "rock"],
        "creator": {"userId": 42, "nickname": "Owner", "avatarUrl": "https://example/cover"}
    }))?;
    assert_eq!(playlist.play_count, 3456);
    assert!(playlist.subscribed);
    assert_eq!(playlist.tags, ["pop", "rock"]);
    let creator = playlist.creator.unwrap_or_default();
    assert_eq!(creator.id, 42);
    assert_eq!(creator.nickname, "Owner");
    Ok(())
}

#[test]
fn account_membership_fields_default_when_the_profile_omits_them() -> Result<(), serde_json::Error>
{
    let account: super::Account =
        serde_json::from_value(json!({"userId": 7, "nickname": "Listener"}))?;
    assert_eq!(account.vip_type, 0);
    assert_eq!(account.level, 0);
    let vip: super::Account = serde_json::from_value(json!({
        "userId": 7, "nickname": "Listener", "vipType": 11, "level": 8
    }))?;
    assert_eq!(vip.vip_type, 11);
    assert_eq!(vip.level, 8);
    Ok(())
}

#[test]
fn comment_threads_name_the_resource_family_in_the_wire_id() {
    assert_eq!(CommentThread::Song(42).wire_id(), "R_SO_4_42");
    assert_eq!(CommentThread::Album(42).wire_id(), "R_AL_3_42");
    assert_eq!(CommentThread::Playlist(42).wire_id(), "A_PL_0_42");
    assert_eq!(CommentThread::Mv(42).wire_id(), "R_MV_5_42");
    assert_eq!(CommentThread::Album(42).id(), 42);
    assert_eq!(CommentThread::Mv(42).id(), 42);
}

#[test]
fn comments_keep_the_playlist_comment_serde_shape() -> Result<(), serde_json::Error> {
    let comment: Comment = serde_json::from_value(json!({
        "commentId": 4_294_967_301_i64,
        "author": "Listener",
        "content": "Great mix",
        "time_ms": 1_700_000_000_000_i64,
        "liked": true,
        "likedCount": 17
    }))?;
    assert_eq!(comment.id, 4_294_967_301);
    assert_eq!(comment.author, "Listener");
    assert_eq!(comment.content, "Great mix");
    assert_eq!(comment.time_ms, 1_700_000_000_000);
    assert!(comment.liked);
    assert_eq!(comment.liked_count, 17);
    let json = serde_json::to_value(&comment)?;
    assert_eq!(json["time_ms"], 1_700_000_000_000_i64);
    Ok(())
}

#[test]
fn comment_like_fields_default_for_responses_that_omit_them() -> Result<(), serde_json::Error> {
    let comment: Comment =
        serde_json::from_value(json!({"author": "Listener", "content": "Old shape"}))?;
    assert_eq!(comment.id, 0);
    assert!(!comment.liked);
    assert_eq!(comment.liked_count, 0);
    Ok(())
}

/// The wire sends a playlist's plays as a JSON float (`25370443800.0`), a subscription count as a
/// string, and the collected flag as `null` for an anonymous viewer. Strict fields rejected the
/// whole response, which the UI could only report as "unrecognised data" on every page that
/// carries a playlist.
#[test]
fn playlist_counts_and_flags_survive_the_shapes_the_endpoint_actually_sends()
-> Result<(), serde_json::Error> {
    let playlist: Playlist = serde_json::from_value(json!({
        "id": 1,
        "name": "Mix",
        "trackCount": 12.0,
        "playCount": 25_370_443_800.0,
        "subscribedCount": "7",
        "subscribed": null
    }))?;

    assert_eq!(playlist.track_count, 12);
    assert_eq!(playlist.play_count, 25_370_443_800);
    assert_eq!(playlist.subscribed_count, 7);
    assert!(!playlist.subscribed);
    Ok(())
}

/// A deleted or region-locked recording answers with a null name and a float popularity score,
/// and one such entry may not fail the page it arrived in.
#[test]
fn a_song_survives_a_null_name_and_a_float_score() -> Result<(), serde_json::Error> {
    let song: Song =
        serde_json::from_value(json!({"id": 9, "name": null, "pop": 100.0, "dt": 1000}))?;

    assert_eq!(song.name, "");
    assert_eq!(song.pop, 100);
    assert_eq!(song.duration_ms, 1000);
    Ok(())
}

/// The detail route sends `duration` and `publishTime` as numbers a strict integer field would
/// sometimes reject, and the listing omits `desc` entirely.
#[test]
fn mv_rows_tolerate_float_counts_and_missing_detail_fields() -> Result<(), serde_json::Error> {
    let detail: Mv = serde_json::from_value(json!({
        "id": 7,
        "name": "Video",
        "cover": "https://p1.music.126.net/cover.jpg",
        "artistId": 8,
        "artistName": "Artist",
        "duration": 215_000.0,
        "playCount": "1200",
        "publishTime": 1_600_000_000_000_u64,
        "desc": "Notes"
    }))?;
    assert_eq!(detail.id, 7);
    assert_eq!(detail.duration_ms, 215_000);
    assert_eq!(detail.play_count, 1_200);
    assert_eq!(detail.publish_time_ms, 1_600_000_000_000);
    assert_eq!(detail.description, "Notes");

    let listing: Mv = serde_json::from_value(json!({
        "id": 9, "name": null, "artistName": null, "duration": 60.0
    }))?;
    assert_eq!(listing.name, "");
    assert_eq!(listing.artist_name, "");
    assert_eq!(listing.duration_ms, 60);
    assert_eq!(listing.description, "");
    assert_eq!(listing.cover_url, None);
    Ok(())
}

#[test]
fn mv_stream_keeps_a_refused_stream_absent() -> Result<(), serde_json::Error> {
    let stream: MvStream = serde_json::from_value(json!({"url": "https://cdn/video.mp4"}))?;
    assert_eq!(stream.url.as_deref(), Some("https://cdn/video.mp4"));

    let refused: MvStream = serde_json::from_value(json!({"url": null}))?;
    assert_eq!(refused.url, None);
    Ok(())
}

#[test]
fn a_url_decides_whether_a_restriction_blocks_playback() {
    let ticket = |url: &str, restriction| StreamTicket {
        song_id: 7,
        url: url.to_owned(),
        format: "mp3".to_owned(),
        expires_after_seconds: 600,
        trial: matches!(restriction, PlaybackRestriction::Trial { .. }),
        size: None,
        level: "standard".to_owned(),
        restriction: Some(restriction),
    };
    // A preview has a URL, so its restriction rides beside playback rather than blocking it.
    assert_eq!(
        ticket("https://cdn/a.mp3", PlaybackRestriction::Trial { seconds: 30 }).denial(),
        None
    );
    let withheld = PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright };
    assert_eq!(ticket("", withheld).denial(), Some(withheld));
}
