use super::playback::{downgrade, served_quality};
use super::queue::playback_candidates;
use super::refusal::{PlaybackDenied, kind_for, refine_refusal, refusal_error};
use super::source::{decoder_extension, open_buffered_source, open_cached_file, track_summary};
use nekonetease_audio::player::source::audio::AudioSource;
use nekonetease_core::entities::cloud::{
    Album, Artist, PlaybackRestriction, Quality, Song, UnavailableReason,
};
use nekonetease_core::utils::toast::ToastKind;
use nekonetease_engine::player::engine::types::RepeatMode;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn serve_media(listener: TcpListener, bytes: Vec<u8>) -> thread::JoinHandle<std::io::Result<()>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut request = Vec::new();
        let mut chunk = [0; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )?;
        stream.write_all(&bytes)
    })
}

#[test]
fn decoder_hint_accepts_known_formats_and_defaults_unknown_values() {
    for (input, expected) in
        [("mp3", "mp3"), ("FLAC", "flac"), ("aac", "aac"), ("m4a", "m4a"), ("unknown", "mp3")]
    {
        assert_eq!(decoder_extension(input), expected);
    }
}

#[test]
fn served_quality_reads_the_reported_tier_and_ignores_a_missing_one() {
    assert_eq!(served_quality(""), None, "the legacy route reports no level");
    assert_eq!(served_quality("standard"), Some(Quality::Standard));
    assert_eq!(served_quality("exhigh"), Some(Quality::Exhigh));
    assert_eq!(served_quality("lossless"), Some(Quality::Lossless));
}

#[test]
fn a_downgrade_is_reported_only_when_the_served_tier_is_lower() {
    assert_eq!(downgrade(Quality::Lossless, Some(Quality::Exhigh)), Some(Quality::Exhigh));
    assert_eq!(downgrade(Quality::Exhigh, Some(Quality::Exhigh)), None);
    assert_eq!(downgrade(Quality::Higher, Some(Quality::Lossless)), None);
    assert_eq!(downgrade(Quality::Lossless, None), None);
}

/// A song detail as the refusal refinement reads it. Only the two classification fields vary.
fn refused_song(no_copyright: Value, privilege: Option<Value>) -> Result<Song, serde_json::Error> {
    let mut row = json!({"id": 7, "name": "Refused"});
    row["noCopyrightRcmd"] = no_copyright;
    if let Some(privilege) = privilege {
        row["privilege"] = privilege;
    }
    serde_json::from_value(row)
}

#[test]
fn a_missing_licence_is_reported_ahead_of_a_generic_refusal() -> Result<(), serde_json::Error> {
    let song = refused_song(
        json!({"type": 1, "typeDesc": "无版权"}),
        Some(json!({"id": 7, "st": 0, "pl": 320_000, "dl": 320_000, "fee": 0})),
    )?;
    assert_eq!(
        refine_refusal(
            PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown },
            Some(&song)
        ),
        PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright }
    );
    Ok(())
}

#[test]
fn a_withdrawn_or_rightless_recording_is_not_available() -> Result<(), serde_json::Error> {
    // A withdrawn recording and an account with no play or download bitrate look the same. So
    // does a regional block, which is why the two are reported as one reason.
    let privileges = [
        json!({"id": 7, "st": -200, "pl": 320_000, "dl": 320_000, "fee": 0}),
        json!({"id": 7, "st": 0, "pl": 0, "dl": 0, "fee": 0}),
    ];
    for privilege in privileges {
        let song = refused_song(json!(null), Some(privilege))?;
        assert_eq!(
            refine_refusal(
                PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown },
                Some(&song)
            ),
            PlaybackRestriction::Unavailable { reason: UnavailableReason::NotAvailable }
        );
    }
    Ok(())
}

#[test]
fn the_detail_fee_restores_a_payable_reason_the_stream_row_omitted() -> Result<(), serde_json::Error>
{
    // The two endpoints disagree on `fee` often enough that the detail's answer is read when the
    // stream row left the refusal unclassified. Fee precedence also outranks the zero bitrates,
    // which a guest's VIP track reports.
    let vip =
        refused_song(json!(null), Some(json!({"id": 7, "st": 0, "pl": 0, "dl": 0, "fee": 1})))?;
    let paid =
        refused_song(json!(null), Some(json!({"id": 7, "st": 0, "pl": 0, "dl": 0, "fee": 4})))?;
    let refused = PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown };
    assert_eq!(refine_refusal(refused, Some(&vip)), PlaybackRestriction::VipRequired);
    assert_eq!(refine_refusal(refused, Some(&paid)), PlaybackRestriction::AlbumPurchaseRequired);
    Ok(())
}

#[test]
fn a_detail_that_grants_playback_leaves_the_refusal_unexplained() -> Result<(), serde_json::Error> {
    let song = refused_song(
        json!(null),
        Some(json!({"id": 7, "st": 0, "pl": 320_000, "dl": 320_000, "fee": 0})),
    )?;
    let refused = PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown };
    assert_eq!(
        refine_refusal(refused, Some(&song)),
        refused,
        "a region block leaves no trace on the detail, so it must not be guessed at"
    );
    assert_eq!(refine_refusal(refused, None), refused);
    Ok(())
}

#[test]
fn a_classified_refusal_is_not_second_guessed_by_the_detail() -> Result<(), serde_json::Error> {
    let song = refused_song(
        json!({"type": 1}),
        Some(json!({"id": 7, "st": -200, "pl": 0, "dl": 0, "fee": 0})),
    )?;
    assert_eq!(
        refine_refusal(PlaybackRestriction::VipRequired, Some(&song)),
        PlaybackRestriction::VipRequired
    );
    assert_eq!(
        refine_refusal(PlaybackRestriction::AlbumPurchaseRequired, Some(&song)),
        PlaybackRestriction::AlbumPurchaseRequired
    );
    Ok(())
}

#[test]
fn every_restriction_has_its_own_toast_kind() {
    let cases = [
        (PlaybackRestriction::Trial { seconds: 30 }, ToastKind::TrialPlayback),
        (PlaybackRestriction::VipRequired, ToastKind::VipRequired),
        (PlaybackRestriction::AlbumPurchaseRequired, ToastKind::AlbumPurchaseRequired),
        (
            PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright },
            ToastKind::PlaybackNoCopyright,
        ),
        (
            PlaybackRestriction::Unavailable { reason: UnavailableReason::NotAvailable },
            ToastKind::PlaybackUnavailable,
        ),
        (
            PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown },
            ToastKind::PlaybackUnavailable,
        ),
    ];
    for (restriction, kind) in cases {
        assert_eq!(kind_for(restriction), kind);
    }
}

#[test]
fn a_refusal_carries_its_reason_on_the_error_cause() {
    use std::error::Error;
    let restriction = PlaybackRestriction::Unavailable { reason: UnavailableReason::NoCopyright };
    let error = refusal_error(restriction);
    let denial = error.source().and_then(|cause| cause.downcast_ref::<PlaybackDenied>());
    assert_eq!(denial.map(PlaybackDenied::restriction), Some(restriction));
    assert_eq!(
        nekonetease_core::error::describe(&error),
        "Network error: 该歌曲没有版权，暂时无法播放",
        "the cause repeats the message already printed, so a log line must not say it twice"
    );
}

#[test]
fn unavailable_queue_candidates_follow_repeat_and_shuffle_rules() {
    assert_eq!(
        playback_candidates(4, 2, RepeatMode::Off, false),
        vec![2, 3],
        "repeat-off must not wrap before the selected queue position"
    );
    assert_eq!(playback_candidates(4, 2, RepeatMode::All, false), vec![2, 3, 0, 1]);
    assert_eq!(playback_candidates(4, 2, RepeatMode::One, false), vec![2, 3, 0, 1]);

    let shuffled = playback_candidates(8, 5, RepeatMode::Off, true);
    assert_eq!(shuffled.first(), Some(&5), "shuffle keeps the requested song first");
    let mut sorted = shuffled;
    sorted.sort_unstable();
    assert_eq!(sorted, (0..8).collect::<Vec<_>>(), "shuffle tries each queue row once");
}

#[test]
fn cloud_track_summary_keeps_the_metadata_the_player_bar_renders() {
    let summary = track_summary(
        Song {
            id: 812,
            name: "Cloud song".to_owned(),
            artists: vec![Artist { id: 91, name: "Cloud artist".to_owned(), ..Artist::default() }],
            album: Album { id: 77, name: "Cloud album".to_owned(), ..Album::default() },
            duration_ms: 181_000,
            ..Song::default()
        },
        Some("/cache/cover.jpg".to_owned()),
        true,
    );

    assert_eq!(summary.id, 812);
    assert_eq!(summary.title, "Cloud song");
    assert_eq!(summary.artist.as_deref(), Some("Cloud artist"));
    assert_eq!(summary.album.as_deref(), Some("Cloud album"));
    assert_eq!(summary.duration_ms, 181_000);
    assert_eq!(summary.artwork_path.as_deref(), Some("/cache/cover.jpg"));
    assert_eq!(summary.file_path, "");
    assert!(summary.is_favorite);
}

/// A cache hit must play like a local file: decoded by the same decoder the network path ends
/// on, with no range reader behind it. The entry's name is the cache's, not the fixture's.
#[test]
fn a_cached_entry_opens_as_a_local_file() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::TempDir::new()?;
    let fixture =
        std::fs::read(std::path::Path::new(nekonetease_testkit::ASSETS_DIR).join("silence.flac"))?;
    let entry = tmp.path().join("7-lossless.flac");
    std::fs::write(&entry, fixture)?;

    let mut source = open_cached_file(&entry, Duration::ZERO)?;

    assert!(source.sample_rate().get() > 0);
    assert!(source.next().is_some());
    Ok(())
}

#[test]
fn a_non_range_cdn_response_reaches_the_buffered_decoder() -> Result<(), Box<dyn std::error::Error>>
{
    let bytes =
        std::fs::read(std::path::Path::new(nekonetease_testkit::ASSETS_DIR).join("silence.wav"))?;
    let length = u64::try_from(bytes.len())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/track.wav", listener.local_addr()?);
    let server = serve_media(listener, bytes);

    let mut source = open_buffered_source(&url, "wav", Some(length), Duration::ZERO)?;

    assert_eq!(source.sample_rate().get(), 44_100);
    assert_eq!(source.channels().get(), 1);
    assert_eq!(source.total_duration(), Some(Duration::from_secs(1)));
    assert!(source.next().is_some());
    server.join().map_err(|_| std::io::Error::other("media server panicked"))??;
    Ok(())
}
