use super::ep;
use super::favorites::{decode_created_playlists, decode_saved_playlists};
use super::playback::{decode_intelligence_songs, refusal_verdict, trial_restriction};
use crate::netease::Result;
use crate::netease::endpoints::Protocol;
use nekonetease_core::entities::cloud::Quality;
use serde_json::json;

#[tokio::test]
async fn listening_uses_clientlog_eapi_with_the_desktop_reporting_profile() -> Result<()> {
    let client = super::Client::new();
    let cookies =
        std::collections::BTreeMap::from([("MUSIC_U".to_owned(), "test-token".to_owned())]);
    let request = client
        .prepare_request(ep::WEBLOG, super::session::listen_log("play", 42, 123)?, &cookies)
        .await?
        .build()
        .map_err(|_| crate::netease::Error::InvalidResponse)?;
    assert_eq!(request.url().as_str(), "https://clientlog.music.163.com/eapi/feedback/weblog");
    assert_eq!(ep::WEBLOG.protocol, Protocol::Eapi);
    const { assert!(!ep::WEBLOG.read_only) };
    let (_, header, _) = client.request_context(ep::WEBLOG, &cookies, "csrf", 1000);
    assert_eq!(header["os"], "osx");
    assert_eq!(header["MUSIC_U"], "test-token");
    assert_eq!(header["__csrf"], "csrf");
    assert_eq!(header["deviceId"], client.device_id());
    Ok(())
}

#[test]
fn listening_logs_pair_song_identity_with_actual_seconds() -> Result<()> {
    for action in ["startplay", "play"] {
        let body = super::session::listen_log(action, 42, 123)?;
        let logs: serde_json::Value = serde_json::from_str(
            body["logs"].as_str().ok_or(crate::netease::Error::InvalidResponse)?,
        )
        .map_err(|_| crate::netease::Error::InvalidResponse)?;
        assert_eq!(logs[0]["action"], action);
        assert_eq!(logs[0]["json"]["id"], "42");
        assert_eq!(logs[0]["json"]["type"], "song");
        assert_eq!(logs[0]["json"]["content"], "id=0");
        if action == "play" {
            assert_eq!(logs[0]["json"]["time"], 123);
            assert_eq!(logs[0]["json"]["end"], "playend");
        } else {
            assert!(logs[0]["json"]["time"].is_null());
        }
    }
    Ok(())
}

#[tokio::test]
async fn listening_never_sends_guest_or_stale_session_logs() {
    let client = super::Client::new();
    assert!(matches!(
        client.report_listen(42, 60).await,
        Err(crate::netease::Error::Authentication)
    ));
    let generation = client.generation();
    client.clear_session().await;
    assert!(matches!(
        client.request_in_generation(ep::WEBLOG, json!({}), generation).await,
        Err(crate::netease::Error::Cancelled)
    ));
}

#[test]
fn stream_quality_uses_yesplaymusic_bitrates() {
    // The legacy route is the fallback now; `STREAM_V1` is the one playback prefers.
    assert_eq!(ep::STREAM.path, "/api/song/enhance/player/url");
    assert_eq!(ep::STREAM.protocol, Protocol::Eapi);
    for (quality, bitrate) in [
        (Quality::Standard, 128_000),
        (Quality::Higher, 192_000),
        (Quality::Exhigh, 320_000),
        (Quality::Lossless, 350_000),
        (Quality::Hires, 999_000),
    ] {
        assert_eq!(quality.bitrate(), bitrate);
    }
}

#[test]
fn created_and_collected_pages_partition_account_playlists() -> Result<()> {
    let body = json!({"more": true, "playlist": [
        {"id": 3_136_952_023_i64, "name": "Created", "creator": {"userId": 55}},
        {"id": 4_000_000_000_i64, "name": "Collected", "creator": {"userId": 77}},
        {"id": 12, "name": "Unknown owner"}
    ]});
    let (created, created_more) = decode_created_playlists(&body, 55)?;
    let (collected, collected_more) = decode_saved_playlists(&body, 55)?;

    assert_eq!(created.iter().map(|playlist| playlist.id).collect::<Vec<_>>(), [3_136_952_023]);
    assert_eq!(
        collected.iter().map(|playlist| playlist.id).collect::<Vec<_>>(),
        [4_000_000_000, 12]
    );
    assert!(created_more);
    assert!(collected_more);
    Ok(())
}

#[test]
fn an_empty_filtered_page_does_not_hide_later_created_playlists() -> Result<()> {
    let body = json!({"more": true, "playlist": [{"id": 11, "name": "Collected", "creator": {"userId": 77}}]});
    let (created, more) = decode_created_playlists(&body, 55)?;
    assert_eq!(created, [] as [nekonetease_core::entities::cloud::Playlist; 0]);
    assert!(more);
    Ok(())
}

#[test]
fn intelligence_decodes_nested_songs_in_server_order() -> Result<()> {
    let songs = decode_intelligence_songs(&json!({"data": [
        {"id": 3_000_000_000_i64, "songInfo": {"id": 3_000_000_000_i64, "name": "First", "artists": [{"id": 8, "name": "Artist"}]}},
        {"id": 7, "songInfo": {"id": 7, "name": "Second"}}
    ]}))?;
    assert_eq!(songs.iter().map(|song| song.id).collect::<Vec<_>>(), [3_000_000_000, 7]);
    assert_eq!(songs[0].artist_names(), "Artist");
    Ok(())
}

#[test]
fn intelligence_rejects_an_entry_without_song_info() {
    assert!(decode_intelligence_songs(&json!({"data": [{"id": 7}]})).is_err());
}

#[test]
fn a_preview_is_a_trial_and_a_paid_track_with_a_url_is_not_restricted() {
    use nekonetease_core::entities::cloud::PlaybackRestriction;
    assert_eq!(
        trial_restriction(&json!({"freeTrialInfo": {"start": 30, "end": 60}, "fee": 1})),
        Some(PlaybackRestriction::Trial { seconds: 30 })
    );
    // A paying account's VIP track reports `fee: 1` and still hands out a full URL.
    assert_eq!(trial_restriction(&json!({"fee": 1, "url": "https://example/a.mp3"})), None);
}

#[test]
fn a_fee_one_refusal_reports_a_membership_requirement() {
    use nekonetease_core::entities::cloud::PlaybackRestriction;
    // A recorded refusal from `/api/song/enhance/player/url/v1`: `url` is null, the row still
    // carries its identity, and `fee: 1` is the only reason it states.
    let row = json!({
        "id": 33_894_312, "url": null, "br": 0, "size": 0, "md5": null, "code": 404,
        "expi": 1200, "type": null, "gain": 0, "fee": 1, "payed": 0, "flag": 4,
        "level": null, "freeTrialInfo": null,
        "freeTrialPrivilege": {"resConsumable": false, "userConsumable": false}
    });
    assert_eq!(refusal_verdict(&row), PlaybackRestriction::VipRequired);
}

#[test]
fn a_fee_four_refusal_reports_an_album_purchase_requirement() {
    use nekonetease_core::entities::cloud::PlaybackRestriction;
    let row = json!({
        "id": 1_901_371_647, "url": null, "br": 0, "size": 0, "md5": null, "code": 404,
        "expi": 1200, "type": null, "gain": 0, "fee": 4, "payed": 0, "flag": 4,
        "level": null, "freeTrialInfo": null
    });
    assert_eq!(refusal_verdict(&row), PlaybackRestriction::AlbumPurchaseRequired);
}

#[test]
fn an_unclassified_refusal_leaves_the_reason_to_the_song_detail() {
    use nekonetease_core::entities::cloud::{PlaybackRestriction, UnavailableReason};
    // Fee 0 or 8 states nothing a listener can act on, and the row has no `privilege` or
    // `noCopyrightRcmd` — only the song detail does. A regional block reports this same shape.
    let row = json!({
        "id": 1_974_443_814, "url": null, "br": 0, "size": 0, "md5": null, "code": 404,
        "expi": 1200, "type": null, "gain": 0, "fee": 0, "payed": 0, "flag": 0,
        "level": null, "freeTrialInfo": null
    });
    assert_eq!(
        refusal_verdict(&row),
        PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown }
    );
}

#[test]
fn playback_prefers_v1_and_keeps_the_legacy_route_registered() {
    assert_eq!(ep::STREAM_V1.path, "/api/song/enhance/player/url/v1");
    assert_eq!(ep::STREAM_V1.protocol, Protocol::Xeapi);
    assert_eq!(ep::STREAM.path, "/api/song/enhance/player/url");
    assert_eq!(ep::STREAM.protocol, Protocol::Eapi);
}
