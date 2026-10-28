use serde_json::json;

use nekonetease_core::entities::dj::DjProgram;

use crate::netease::Error;
use crate::netease::endpoints::Protocol;

use super::{
    DJ_DETAIL, DJ_HOT, DJ_PROGRAM_DETAIL, DJ_PROGRAMS, DJ_RECOMMEND, DJ_SUBSCRIBE, DJ_UNSUBSCRIBE,
    decode_program, decode_program_page, decode_radio, decode_radio_page,
};

#[test]
fn the_dj_routes_are_the_verified_wire_paths() {
    assert_eq!(DJ_RECOMMEND.path, "/api/djradio/recommend/v1");
    assert_eq!(DJ_HOT.path, "/api/djradio/hot/v1");
    assert_eq!(DJ_DETAIL.path, "/api/djradio/v2/get");
    assert_eq!(DJ_PROGRAMS.path, "/api/dj/program/byradio");
    assert_eq!(DJ_PROGRAM_DETAIL.path, "/api/dj/program/detail");
    assert_eq!(DJ_SUBSCRIBE.path, "/api/djradio/sub");
    assert_eq!(DJ_UNSUBSCRIBE.path, "/api/djradio/unsub");

    for endpoint in [DJ_RECOMMEND, DJ_HOT, DJ_DETAIL, DJ_PROGRAMS, DJ_PROGRAM_DETAIL] {
        assert_eq!(endpoint.protocol, Protocol::Weapi);
        assert!(endpoint.read_only);
    }
    for endpoint in [DJ_SUBSCRIBE, DJ_UNSUBSCRIBE] {
        assert_eq!(endpoint.protocol, Protocol::Weapi);
        assert!(!endpoint.read_only);
    }
}

#[test]
fn a_hot_page_decodes_float_counts_and_null_names() -> Result<(), Box<dyn std::error::Error>> {
    let body = json!({
        "djRadios": [{
            "id": 42,
            "name": null,
            "picUrl": "https://p1.music.126.net/cover.jpg",
            "desc": null,
            "programCount": 12.0,
            "subCount": "9",
            "playCount": null,
            "category": "Podcast",
            "subscribed": 1,
            "dj": {"userId": 7, "nickname": "Host", "avatarUrl": null}
        }],
        "more": true
    });

    let page = decode_radio_page(&body, 0, 30)?;
    assert!(page.more);
    assert_eq!(page.radios.len(), 1);
    let radio = page.radios.first().ok_or("the fixture holds one radio")?;
    assert_eq!(radio.id, 42);
    assert_eq!(radio.name, "");
    assert_eq!(radio.description, "");
    assert_eq!(radio.program_count, 12);
    assert_eq!(radio.subscriber_count, 9);
    assert_eq!(radio.play_count, 0);
    assert!(radio.subscribed);
    assert_eq!(radio.host_name(), "Host");
    Ok(())
}

#[test]
fn a_recommended_page_accepts_both_array_envelopes() -> Result<(), Box<dyn std::error::Error>> {
    let direct = json!({"data": [{"id": 1, "name": "One"}]});
    let wrapped = json!({"data": {"djRadios": [{"id": 2, "name": "Two"}]}});

    let direct = decode_radio_page(&direct, 0, 30)?;
    let wrapped = decode_radio_page(&wrapped, 0, 30)?;
    assert_eq!(direct.radios.first().map(|radio| radio.id), Some(1));
    assert_eq!(wrapped.radios.first().map(|radio| radio.id), Some(2));
    Ok(())
}

#[test]
fn a_radio_detail_keeps_the_station_object() -> Result<(), Box<dyn std::error::Error>> {
    let body = json!({
        "data": {
            "id": 88,
            "name": "Night Shift",
            "description": "Late-night stories",
            "subscribed": false,
            "dj": {"userId": 3, "nickname": "Reader"}
        }
    });

    let radio = decode_radio(&body)?;
    assert_eq!(radio.id, 88);
    assert_eq!(radio.name, "Night Shift");
    assert_eq!(radio.description, "Late-night stories");
    assert!(!radio.subscribed);
    assert_eq!(radio.host_name(), "Reader");
    Ok(())
}

#[test]
fn a_radio_detail_accepts_the_older_dj_radio_key() -> Result<(), Box<dyn std::error::Error>> {
    let body = json!({"djRadio": {"id": 5, "name": "Old route"}});
    assert_eq!(decode_radio(&body)?.id, 5);
    Ok(())
}

#[test]
fn a_program_page_decodes_programs_and_count_paging() -> Result<(), Box<dyn std::error::Error>> {
    let body = json!({
        "programs": [{
            "id": 300,
            "name": "Episode 1",
            "coverUrl": "https://p1.music.126.net/program.jpg",
            "description": null,
            "duration": 1_800_000.0,
            "listenerCount": "1200",
            "serialNum": 1,
            "createTime": 1_700_000_000_000i64,
            "mainSong": {
                "id": 900,
                "name": "Episode 1",
                "duration": 1_800_000,
                "artists": [{"id": 1, "name": "Reader"}],
                "album": {"id": 2, "name": "Night Shift", "picUrl": null}
            },
            "radio": {"id": 88, "name": "Night Shift", "picUrl": null,
                      "dj": {"userId": 3, "nickname": "Reader"}}
        }],
        "count": 31,
        "more": true
    });

    let page = decode_program_page(&body, 0, 30)?;
    assert!(page.more);
    assert_eq!(page.programs.len(), 1);
    let program = page.programs.first().ok_or("the fixture holds one program")?;
    assert_eq!(program.id, 300);
    assert_eq!(program.description, "");
    assert_eq!(program.duration_ms, 1_800_000);
    assert_eq!(program.listener_count, 1200);
    assert_eq!(program.song_id(), Some(900));
    assert_eq!(program.artwork_url(), Some("https://p1.music.126.net/program.jpg"));
    assert_eq!(program.host_name(), "Reader");
    Ok(())
}

#[test]
fn a_program_without_audio_is_listed_but_has_no_song_id() -> Result<(), Box<dyn std::error::Error>>
{
    let body = json!({
        "programs": [{"id": 301, "name": "No audio", "mainSong": null}],
        "more": false
    });
    let page = decode_program_page(&body, 0, 30)?;
    assert_eq!(page.programs.first().and_then(DjProgram::song_id), None);
    assert!(!page.more);
    Ok(())
}

#[test]
fn a_program_detail_decodes_the_playable_main_song() -> Result<(), Box<dyn std::error::Error>> {
    let body = json!({
        "data": {
            "program": {
                "id": 400,
                "name": "Episode 2",
                "description": "The story continues",
                "mainSong": {"id": 901, "name": "Episode 2", "artists": [], "album": {"id": 0, "name": ""}},
                "radio": {"id": 88, "name": "Night Shift"}
            }
        }
    });

    let program = decode_program(&body)?;
    assert_eq!(program.id, 400);
    assert_eq!(program.song_id(), Some(901));
    assert_eq!(program.description, "The story continues");
    assert_eq!(program.artwork_url(), None);
    Ok(())
}

#[test]
fn a_missing_envelope_is_an_invalid_response() {
    assert!(matches!(decode_radio(&json!({"data": null})), Err(Error::InvalidResponse)));
    assert!(matches!(decode_program(&json!({})), Err(Error::InvalidResponse)));
    assert!(matches!(decode_program_page(&json!({}), 0, 30), Err(Error::InvalidResponse)));
    assert!(matches!(decode_radio_page(&json!({}), 0, 30), Err(Error::InvalidResponse)));
}

#[test]
fn inferred_paging_respects_the_count_and_the_page_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let short = json!({"programs": [{"id": 1}], "count": 31});
    assert!(decode_program_page(&short, 0, 30)?.more);

    let exact = json!({"programs": [{"id": 1}], "count": 1});
    assert!(!decode_program_page(&exact, 0, 30)?.more);

    let full_page = json!({"programs": [{"id": 1}]});
    assert!(decode_program_page(&full_page, 0, 1)?.more);
    assert!(!decode_program_page(&full_page, 0, 30)?.more);

    let hot = json!({"djRadios": [{"id": 1}], "total": 2});
    assert!(decode_radio_page(&hot, 0, 30)?.more);
    Ok(())
}
