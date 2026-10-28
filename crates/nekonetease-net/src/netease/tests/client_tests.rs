use std::collections::BTreeMap;

use nekonetease_core::entities::cloud::{Album, Song};
use reqwest::header::COOKIE;
use serde_json::{Value, json};

use super::favorites::{
    assemble_liked_songs, decode_liked_playlist_entries, decode_liked_playlist_id,
    decode_recent_songs, decode_saved_playlists,
};
use super::library::{decode_playlist_header, fill_album_covers, reorder_songs};
use super::request::{DESKTOP_APPVER, EAPI_UA};
use super::session::{merge_set_cookie, valid_cookie_pair};
use super::{Client, Result, crypto, ep};

#[test]
fn cookie_rotation_and_deletion_keep_other_cookies() {
    let mut jar = BTreeMap::new();
    merge_set_cookie(&mut jar, "MUSIC_U=old; Domain=.music.163.com; HttpOnly");
    merge_set_cookie(&mut jar, "__csrf=csrf; Path=/");
    merge_set_cookie(&mut jar, "MUSIC_U=new; Path=/");
    assert_eq!(jar.get("MUSIC_U").map(String::as_str), Some("new"));
    merge_set_cookie(&mut jar, "MUSIC_U=; Max-Age=0");
    assert!(!jar.contains_key("MUSIC_U"));
    assert!(jar.contains_key("__csrf"));
}

#[test]
fn header_injection_is_rejected() {
    assert!(!valid_cookie_pair("MUSIC_U", "bad\r\nInjected: yes"));
}

#[tokio::test]
async fn qr_key_request_matches_the_reference_desktop_eapi_context()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let device_id = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123";
    let client = Client::with_device_id(device_id)?;
    let request =
        client.prepare_request(ep::QR_KEY, json!({"type":3}), &BTreeMap::new()).await?.build()?;

    assert_eq!(
        request.url().as_str(),
        "https://interfacepc.music.163.com/eapi/login/qrcode/unikey"
    );
    assert_eq!(request.headers()["user-agent"], EAPI_UA);
    assert!(!request.headers().contains_key("referer"));
    let cookie = request.headers()[COOKIE].to_str()?;
    assert!(cookie.contains("os=pc"));
    assert!(cookie.contains("appver=3.1.17.204416"));
    assert!(cookie.contains("osver=Microsoft-Windows-10-Professional-build-19045-64bit"));
    assert!(cookie.contains(&format!("deviceId={device_id}")));
    assert!(!cookie.contains("MUSIC_U="));
    assert!(cookie.contains("MUSIC_A="));

    let body = request
        .body()
        .and_then(reqwest::Body::as_bytes)
        .ok_or_else(|| std::io::Error::other("missing request body"))?;
    let params =
        reqwest::Url::parse(&format!("https://test.invalid/?{}", String::from_utf8_lossy(body)))?
            .query_pairs()
            .find_map(|(name, value)| (name == "params").then(|| value.into_owned()))
            .ok_or_else(|| std::io::Error::other("missing EAPI params"))?;
    let encrypted = params
        .as_bytes()
        .chunks(2)
        .map(|digits| {
            let digits = std::str::from_utf8(digits)?;
            Ok::<_, Box<dyn std::error::Error>>(u8::from_str_radix(digits, 16)?)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let plaintext = String::from_utf8(crypto::decrypt_eapi_for_test(&encrypted)?)?;
    let mut envelope = plaintext.split("-36cd479b6b5-");
    assert_eq!(envelope.next(), Some("/api/login/qrcode/unikey"));
    let data: Value = serde_json::from_str(
        envelope.next().ok_or_else(|| std::io::Error::other("missing EAPI JSON payload"))?,
    )?;
    assert_eq!(data["type"], 3);
    assert_eq!(data["header"]["os"], "pc");
    assert_eq!(data["header"]["appver"], DESKTOP_APPVER);
    Ok(())
}

#[tokio::test]
async fn logout_generation_invalidates_pending_requests() {
    let client = Client::new();
    let before = client.generation();
    client.restore_cookie("MUSIC_U=test; __csrf=csrf").await;
    assert!(client.generation() > before);
    client.clear_session().await;
    assert_eq!(client.cookie_for_storage().await, "");
}

#[test]
fn persisted_device_identifier_must_be_uppercase_hex() {
    let valid = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123";
    assert!(Client::with_device_id(valid).is_ok());
    assert!(Client::with_device_id(valid.to_ascii_lowercase()).is_err());
}

#[test]
fn reordered_song_details_keep_requested_order_and_repetitions() {
    let result = reorder_songs(
        &[9, 4, 9, 15],
        vec![
            Song { id: 4, name: "four".into(), ..Song::default() },
            Song { id: 9, name: "nine".into(), ..Song::default() },
        ],
    );
    assert_eq!(result.iter().map(|song| song.id).collect::<Vec<_>>(), [9, 4, 9]);
    assert_eq!(result[0].name, "nine");
    assert_eq!(result[2].name, "nine");
}

#[test]
fn album_detail_supplies_missing_song_covers_without_overwriting_other_albums() {
    let album = Album {
        id: 7,
        cover_url: Some("https://p1.music.126.net/album.jpg".into()),
        ..Album::default()
    };
    let mut songs = vec![
        Song { album: Album { id: 7, ..Album::default() }, ..Song::default() },
        Song {
            album: Album { id: 7, cover_url: Some("existing.jpg".into()), ..Album::default() },
            ..Song::default()
        },
        Song { album: Album { id: 8, ..Album::default() }, ..Song::default() },
    ];

    fill_album_covers(&album, &mut songs);

    assert_eq!(songs[0].album.cover_url, album.cover_url);
    assert_eq!(songs[1].album.cover_url.as_deref(), Some("existing.jpg"));
    assert_eq!(songs[2].album.cover_url, None);
}

#[test]
fn saved_playlist_page_omits_playlists_created_by_the_account() -> Result<()> {
    let (playlists, more) = decode_saved_playlists(
        &json!({
            "more": true,
            "playlist": [
                {"id": 10, "name": "Created", "creator": {"userId": 55}},
                {"id": 11, "name": "Collected", "creator": {"userId": 77}}
            ]
        }),
        55,
    )?;

    assert_eq!(playlists.iter().map(|playlist| playlist.id).collect::<Vec<_>>(), [11]);
    assert!(more);
    Ok(())
}

#[test]
fn liked_playlist_lookup_uses_the_netease_special_playlist() -> Result<()> {
    let body = json!({
        "playlist": [
            {"id": 99, "name": "Collected", "specialType": 0},
            {"id": 88, "name": "My liked songs", "specialType": 5}
        ]
    });

    assert_eq!(decode_liked_playlist_id(&body)?, 88);
    assert!(decode_liked_playlist_id(&json!({"playlist": []})).is_err());
    Ok(())
}

#[test]
fn liked_song_sequence_and_favorite_times_come_from_track_ids() -> Result<()> {
    let body = json!({
        "playlist": {
            "trackIds": [
                {"id": 42, "v": 1, "at": 1_700_000_000_000_i64},
                {"id": 7, "v": 1, "at": 1_600_000_000_000_i64}
            ]
        }
    });
    let entries = decode_liked_playlist_entries(&body)?;
    let liked = assemble_liked_songs(
        entries,
        vec![
            Song { id: 7, name: "Detail returned first".into(), ..Song::default() },
            Song { id: 42, name: "Favorite returned first".into(), ..Song::default() },
        ],
    );

    assert_eq!(liked.iter().map(|entry| entry.song.id).collect::<Vec<_>>(), [42, 7]);
    assert_eq!(liked.iter().map(|entry| entry.favorite_order).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(
        liked.iter().map(|entry| entry.added_at_ms).collect::<Vec<_>>(),
        [Some(1_700_000_000_000), Some(1_600_000_000_000)]
    );
    Ok(())
}

#[test]
fn recent_song_records_keep_netease_order_and_play_timestamps() -> Result<()> {
    let rows = decode_recent_songs(&json!({
        "data": {"list": [
            {
                "resourceId": "42",
                "playTime": 1_700_000_000_123_i64,
                "resourceType": "SONG",
                "data": {"id": 42, "name": "First", "ar": [], "al": {"id": 1, "name": "Album"}, "dt": 1000}
            },
            {
                "resourceId": "7",
                "playTime": 1_600_000_000_456_i64,
                "resourceType": "SONG",
                "data": {"id": 7, "name": "Second", "ar": [], "al": {"id": 2, "name": "Album 2"}, "dt": 2000}
            },
            {
                "resourceId": "9",
                "playTime": 1_500_000_000_000_i64,
                "resourceType": "VIDEO",
                "data": {"id": 9, "name": "Not a song"}
            }
        ]}
    }))?;

    assert_eq!(rows.iter().map(|entry| entry.song.id).collect::<Vec<_>>(), [42, 7]);
    assert_eq!(rows[0].played_at_ms, 1_700_000_000_123);
    assert_eq!(rows[1].played_at_ms, 1_600_000_000_456);
    Ok(())
}

#[tokio::test]
async fn recent_history_uses_the_native_weapi_route() -> Result<()> {
    let client = Client::new();
    let request =
        client.prepare_request(ep::RECENT_SONGS, json!({"limit":100}), &BTreeMap::new()).await?;

    assert_eq!(
        request.build().ok().map(|request| request.url().as_str().to_owned()),
        Some("https://music.163.com/weapi/play-record/song/list".to_owned())
    );
    Ok(())
}

#[test]
fn unavailable_liked_song_does_not_shift_later_server_positions() -> Result<()> {
    let entries = decode_liked_playlist_entries(&json!({
        "playlist": {"trackIds": [
            {"id": 42, "at": 100},
            {"id": 99, "at": 200},
            {"id": 7, "at": 300}
        ]}
    }))?;
    let liked = assemble_liked_songs(
        entries,
        vec![Song { id: 7, name: "Available".into(), ..Song::default() }],
    );

    assert_eq!(liked.len(), 1);
    assert_eq!(liked[0].song.id, 7);
    assert_eq!(liked[0].favorite_order, 3);
    Ok(())
}

#[test]
fn playlist_detail_rejects_a_response_for_another_playlist_id() {
    let body = json!({"playlist":{"id":28_837_265,"name":"A different playlist"}});
    assert!(decode_playlist_header(&body, 4_000_000_000).is_err());
    assert!(
        decode_playlist_header(&body, 28_837_265)
            .is_ok_and(|playlist| playlist.name == "A different playlist")
    );
}
