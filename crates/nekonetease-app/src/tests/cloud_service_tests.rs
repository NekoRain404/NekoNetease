use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lru::LruCache;
use nekonetease_core::entities::cloud::{
    Account, CommentThread, Playlist, RecentSong, SearchKind, Song,
};
use nekonetease_core::error::AppError;
use nekonetease_net::netease::{Client, Error};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

use super::cache::{
    PLAYLIST_CACHE_BYTES, PLAYLIST_CACHE_CAP, PLAYLIST_CACHE_TTL, playlist_cache_is_fresh,
};
use super::credential::{SESSION_VERSION, decode_session};
use super::session::{validate_captcha, validate_country_code, validate_phone};
use super::validation::{
    validate_nonempty, validate_playlist_description, validate_playlist_name,
    validate_playlist_tags, validate_track_batch,
};
use super::*;

#[test]
fn netease_cover_requests_use_https_and_a_bounded_size() {
    assert_eq!(
        cover_request_url("http://p2.music.126.net/cover.jpg?token=abc")
            .as_ref()
            .map(reqwest::Url::as_str),
        Some("https://p2.music.126.net/cover.jpg?token=abc&param=512y512")
    );
}

#[test]
fn existing_sizes_and_other_artwork_hosts_are_preserved() {
    assert_eq!(
        cover_request_url("https://p1.music.126.net/cover.jpg?param=224y224")
            .as_ref()
            .map(reqwest::Url::as_str),
        Some("https://p1.music.126.net/cover.jpg?param=224y224")
    );
    assert_eq!(
        cover_request_url("https://example.com/cover.jpg?token=abc")
            .as_ref()
            .map(reqwest::Url::as_str),
        Some("https://example.com/cover.jpg?token=abc")
    );
}

#[test]
fn cover_requests_reject_non_http_urls() {
    assert!(cover_request_url("file:///tmp/cover.jpg").is_none());
    assert!(cover_request_url("https://").is_none());
}

#[test]
fn avatar_requests_use_https_and_a_tile_sized_source() {
    assert_eq!(
        super::covers::avatar_request_url("http://p1.music.126.net/avatar.jpg")
            .as_ref()
            .map(reqwest::Url::as_str),
        Some("https://p1.music.126.net/avatar.jpg?param=160y160")
    );
    assert_eq!(
        super::covers::avatar_request_url("https://p1.music.126.net/avatar.jpg?param=64y64")
            .as_ref()
            .map(reqwest::Url::as_str),
        Some("https://p1.music.126.net/avatar.jpg?param=64y64")
    );
}

const DEVICE_ID: &str = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123";

fn service_for_test() -> CloudService {
    let http = reqwest::Client::new();
    let (status, _) =
        watch::channel(SessionStatus { remember_available: true, ..SessionStatus::default() });
    CloudService {
        client: Client::new(),
        http,
        pending_restore: Arc::new(Mutex::new(None)),
        status,
        transition: Arc::new(Mutex::new(())),
        account_tasks: Arc::new(parking_lot::RwLock::new(CancellationToken::new())),
        guest_initialized: Arc::new(AtomicBool::new(false)),
        liked_ids_cache: Arc::new(parking_lot::RwLock::new(None)),
        liked_ids_revision: Arc::new(AtomicU64::new(0)),
        recent_songs_cache: Arc::new(parking_lot::RwLock::new(None)),
        playlist_cache: Arc::new(parking_lot::Mutex::new(LruCache::new(PLAYLIST_CACHE_CAP))),
    }
}

#[tokio::test]
async fn private_requests_require_login_before_polling_the_network_future() {
    let service = service_for_test();
    let polled = Arc::new(AtomicBool::new(false));
    let future_polled = polled.clone();

    let result = service
        .private_request(async move {
            future_polled.store(true, Ordering::Release);
            Ok::<(), nekonetease_net::netease::Error>(())
        })
        .await;

    assert!(result.is_err());
    assert!(!polled.load(Ordering::Acquire));
}

#[tokio::test]
async fn public_inputs_are_validated_without_initializing_a_guest_session() {
    let service = service_for_test();

    assert!(matches!(
        service.search("  ", SearchKind::Songs, 0).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(service.search_suggestions("  ").await, Err(AppError::Validation(_))));
    let long_query = "字".repeat(201);
    assert!(matches!(service.search_suggestions(&long_query).await, Err(AppError::Validation(_))));
    assert!(matches!(service.songs(&[1, 0]).await, Err(AppError::Validation(_))));
    assert!(matches!(service.playlist_comments(0, 0).await, Err(AppError::Validation(_))));
    for thread in [CommentThread::Song(0), CommentThread::Album(0), CommentThread::Playlist(0)] {
        assert!(matches!(service.comments(thread, 0).await, Err(AppError::Validation(_))));
    }
    assert!(matches!(
        service.like_comment(CommentThread::Playlist(1), 0, true).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service.like_comment(CommentThread::Playlist(0), 7, true).await,
        Err(AppError::Validation(_))
    ));
    // A valid pair reaches the login gate, which never polls the network future.
    assert!(service.like_comment(CommentThread::Playlist(1), 7, true).await.is_err());
    assert!(matches!(service.category_playlists("  ", 0).await, Err(AppError::Validation(_))));
    assert!(matches!(service.high_quality_playlists("", None).await, Err(AppError::Validation(_))));
    assert!(matches!(
        service.high_quality_playlists("全部", Some(-1)).await,
        Err(AppError::Validation(_))
    ));
    // Playlist writes validate before the account gate, so an empty editor never prompts a
    // login it does not need.
    let long_description = "字".repeat(1001);
    let too_many_tags = vec!["tag".to_owned(); 11];
    assert!(matches!(service.create_playlist(" ").await, Err(AppError::Validation(_))));
    assert!(matches!(service.delete_playlists(&[]).await, Err(AppError::Validation(_))));
    assert!(matches!(service.rename_playlist(1, "").await, Err(AppError::Validation(_))));
    assert!(matches!(
        service.update_playlist_description(1, &long_description).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service.update_playlist_tags(1, &too_many_tags).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(service.add_playlist_tracks(1, &[]).await, Err(AppError::Validation(_))));
    assert!(matches!(service.remove_playlist_tracks(0, &[1]).await, Err(AppError::Validation(_))));
    assert!(matches!(
        service.reorder_playlist_tracks(1, &[1, 0]).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(service.unsubscribe_playlist(0).await, Err(AppError::Validation(_))));
    assert!(!service.guest_initialized.load(Ordering::Acquire));
}

#[tokio::test]
async fn automatic_qr_login_is_refused_once_an_account_is_signed_in() {
    let service = service_for_test();
    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 1, ..Account::default() }),
        ..SessionStatus::default()
    });

    assert!(matches!(service.begin_qr_login_if_signed_out().await, Ok(None)));
}

/// A saved session the network could not redeem must not hide the launch-page QR entry: that is
/// exactly the state a user who comes online later is in, and signing in is their way out. The
/// check is pinned at the source because reaching the guard's far side would open a socket.
#[test]
fn automatic_qr_login_does_not_wait_on_a_pending_restore() {
    let source = include_str!("../cloud/session.rs");
    let body = source
        .split_once("pub async fn begin_qr_login_if_signed_out")
        .and_then(|(_, rest)| rest.split_once("pub async fn check_qr_login"))
        .map_or("", |(body, _)| body);

    assert!(!body.is_empty(), "the automatic QR entry point moved");
    assert!(
        !body.contains("pending_restore"),
        "a pending restore gates the launch-page QR flow again: {body}"
    );
}

/// Seven attempts over 122 seconds. The exact schedule is the contract: a shorter one gives up
/// before a slow captive portal clears, and a longer one keeps a task alive past the point where
/// the user's best move is to sign in by another route.
#[test]
fn the_restore_retry_schedule_covers_about_two_minutes() {
    assert_eq!(
        super::session::RESTORE_RETRY_DELAYS,
        [
            Duration::ZERO,
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(32),
            Duration::from_secs(60),
        ]
    );
    let total: Duration = super::session::RESTORE_RETRY_DELAYS.iter().sum();
    assert_eq!(total, Duration::from_secs(122));
}

#[tokio::test(start_paused = true)]
async fn the_restore_retry_gives_up_after_the_schedule_is_spent() {
    let shutdown = CancellationToken::new();
    let attempts = Arc::new(AtomicU64::new(0));
    let counter = attempts.clone();

    let last_error = super::session::retry_restore(&shutdown, move || {
        counter.fetch_add(1, Ordering::AcqRel);
        async { Err::<bool, AppError>(AppError::network_msg("offline")) }
    })
    .await;

    assert!(last_error.is_some());
    assert_eq!(
        attempts.load(Ordering::Acquire),
        u64::try_from(super::session::RESTORE_RETRY_DELAYS.len()).unwrap_or(u64::MAX)
    );
}

#[tokio::test(start_paused = true)]
async fn a_successful_restore_stops_the_retry_before_the_schedule_ends() {
    let shutdown = CancellationToken::new();
    let attempts = Arc::new(AtomicU64::new(0));
    let counter = attempts.clone();

    let last_error = super::session::retry_restore(&shutdown, move || {
        let attempt = counter.fetch_add(1, Ordering::AcqRel);
        async move {
            if attempt == 0 {
                Err::<bool, AppError>(AppError::network_msg("offline"))
            } else {
                Ok(true)
            }
        }
    })
    .await;

    assert!(last_error.is_none());
    assert_eq!(attempts.load(Ordering::Acquire), 2);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_shutdown_stops_the_restore_retry() {
    let shutdown = CancellationToken::new();
    let attempts = Arc::new(AtomicU64::new(0));
    let counter = attempts.clone();
    let cancel = shutdown.clone();

    let last_error = super::session::retry_restore(&shutdown, move || {
        let attempt = counter.fetch_add(1, Ordering::AcqRel);
        if attempt == 1 {
            cancel.cancel();
        }
        async { Err::<bool, AppError>(AppError::network_msg("offline")) }
    })
    .await;

    assert!(last_error.is_none());
    assert_eq!(attempts.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn cancelled_account_generation_does_not_poll_a_request() {
    let service = service_for_test();
    service.account_tasks_token().cancel();
    let polled = Arc::new(AtomicBool::new(false));
    let future_polled = polled.clone();

    let result = service
        .account_request(async move {
            future_polled.store(true, Ordering::Release);
            Ok::<(), nekonetease_net::netease::Error>(())
        })
        .await;

    assert!(result.is_err());
    assert!(!polled.load(Ordering::Acquire));
}

#[test]
fn liked_membership_cache_is_scoped_to_account_and_generation() {
    let service = service_for_test();
    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 42, ..Account::default() }),
        generation: 7,
        ..SessionStatus::default()
    });
    *service.liked_ids_cache.write() = Some((42, 7, HashSet::from([1001, 1002])));

    assert_eq!(service.cached_is_liked(1001), Some(true));
    assert_eq!(service.cached_is_liked(1003), Some(false));

    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 43, ..Account::default() }),
        generation: 8,
        ..SessionStatus::default()
    });
    assert_eq!(service.cached_is_liked(1001), None);
}

#[test]
fn recent_history_cache_is_scoped_to_account_and_generation() {
    let service = service_for_test();
    let recent = vec![RecentSong {
        song: Song { id: 42, name: "Cloud history".into(), ..Song::default() },
        played_at_ms: 1_700_000_000_000,
    }];
    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 42, ..Account::default() }),
        generation: 7,
        ..SessionStatus::default()
    });
    *service.recent_songs_cache.write() = Some((42, 7, recent.clone()));

    assert_eq!(service.cached_recent_songs(), Some(recent));
    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 43, ..Account::default() }),
        generation: 8,
        ..SessionStatus::default()
    });
    assert!(service.cached_recent_songs().is_none());
}

#[tokio::test]
async fn an_account_transition_discards_the_previous_liked_snapshot() {
    let service = service_for_test();
    *service.liked_ids_cache.write() = Some((42, 0, HashSet::from([1001])));

    service.begin_transition().await;

    assert!(service.liked_ids_cache.read().is_none());
}

#[test]
fn a_playlist_cache_entry_is_scoped_to_account_generation_and_playlist() {
    let service = service_for_test();
    service.status.send_replace(SessionStatus {
        account: Some(Account { id: 42, ..Account::default() }),
        generation: 7,
        ..SessionStatus::default()
    });
    let loaded = (Playlist { id: 500, ..Playlist::default() }, Arc::from([]));
    service.store_playlist(42, 7, 500, &loaded);

    assert!(service.cached_playlist(42, 7, 500).is_some());
    // Another account, another generation, or another playlist: all misses.
    assert!(service.cached_playlist(43, 7, 500).is_none());
    assert!(service.cached_playlist(42, 8, 500).is_none());
    assert!(service.cached_playlist(42, 7, 501).is_none());
}

#[test]
fn a_cached_playlist_stops_being_served_at_the_ttl() {
    let fetched = Instant::now();
    let inside = PLAYLIST_CACHE_TTL.saturating_sub(Duration::from_millis(1));
    assert!(playlist_cache_is_fresh(fetched, fetched));
    assert!(playlist_cache_is_fresh(fetched, fetched + inside));
    assert!(!playlist_cache_is_fresh(fetched, fetched + PLAYLIST_CACHE_TTL));
}

fn playlist_fixture(id: i64, name: String) -> (Playlist, Arc<[Song]>) {
    (Playlist { id, ..Playlist::default() }, vec![Song { id: 1, name, ..Song::default() }].into())
}

#[test]
fn reopening_a_playlist_shares_its_song_storage() -> Result<(), &'static str> {
    let service = service_for_test();
    let loaded = playlist_fixture(500, "Cached song".to_owned());
    service.store_playlist(42, 7, 500, &loaded);

    let reopened = service.cached_playlist(42, 7, 500).ok_or("playlist not cached")?;

    assert!(Arc::ptr_eq(&loaded.1, &reopened.1));
    Ok(())
}

#[test]
fn a_playlist_larger_than_the_byte_budget_is_not_retained() {
    let service = service_for_test();
    let loaded = playlist_fixture(500, "x".repeat(PLAYLIST_CACHE_BYTES));

    service.store_playlist(42, 7, 500, &loaded);

    assert!(service.cached_playlist(42, 7, 500).is_none());
}

#[test]
fn playlist_bytes_evict_the_least_recently_used_entry_before_the_count_cap() {
    let service = service_for_test();
    let first = playlist_fixture(500, "a".repeat(3 * 1024 * 1024));
    let second = playlist_fixture(501, "b".repeat(3 * 1024 * 1024));
    let third = playlist_fixture(502, "c".repeat(3 * 1024 * 1024));
    service.store_playlist(42, 7, 500, &first);
    service.store_playlist(42, 7, 501, &second);
    assert!(service.cached_playlist(42, 7, 500).is_some());

    service.store_playlist(42, 7, 502, &third);

    assert!(service.cached_playlist(42, 7, 500).is_some());
    assert!(service.cached_playlist(42, 7, 501).is_none());
    assert!(service.cached_playlist(42, 7, 502).is_some());
}

#[test]
fn saved_session_round_trips_without_changing_cookie_pairs() -> Result<(), serde_json::Error> {
    let session = StoredSession {
        version: SESSION_VERSION,
        device_id: DEVICE_ID.into(),
        cookie: "MUSIC_U=session-value; __csrf=csrf-value".into(),
    };
    let encoded = serde_json::to_string(&session)?;
    let decoded = decode_session(&encoded);
    assert!(decoded.is_some(), "the serialized session should be valid");
    let decoded = decoded.unwrap_or(StoredSession {
        version: 0,
        device_id: String::new(),
        cookie: String::new(),
    });
    assert_eq!(decoded.version, SESSION_VERSION);
    assert_eq!(decoded.device_id, DEVICE_ID);
    assert_eq!(decoded.cookie, "MUSIC_U=session-value; __csrf=csrf-value");
    Ok(())
}

#[test]
fn invalid_saved_sessions_are_rejected() {
    for session in [
        format!(r#"{{"version":2,"device_id":"{DEVICE_ID}","cookie":"MUSIC_U=x"}}"#),
        r#"{"version":1,"device_id":"bad","cookie":"MUSIC_U=x"}"#.into(),
        format!(r#"{{"version":1,"device_id":"{DEVICE_ID}","cookie":"MUSIC_U=x\nInjected: y"}}"#),
    ] {
        assert!(decode_session(&session).is_none());
    }
}

#[test]
fn login_fields_reject_blank_values_before_network_access() {
    assert!(validate_nonempty("邮箱", " \t").is_err());
    assert!(validate_nonempty("邮箱", "user@example.com").is_ok());
}

#[test]
fn playlist_write_limits_accept_the_boundary_and_reject_the_next_value() {
    assert!(validate_playlist_name(&"名".repeat(40)).is_ok());
    assert!(validate_playlist_name(&"名".repeat(41)).is_err());
    assert!(validate_playlist_description(&"字".repeat(1000)).is_ok());
    assert!(validate_playlist_description(&"字".repeat(1001)).is_err());
    let ten_tags = vec!["标签".to_owned(); 10];
    let eleven_tags = vec!["标签".to_owned(); 11];
    assert!(validate_playlist_tags(&ten_tags).is_ok());
    assert!(validate_playlist_tags(&eleven_tags).is_err());
    assert!(validate_playlist_tags(&["长".repeat(20)]).is_ok());
    assert!(validate_playlist_tags(&["长".repeat(21)]).is_err());
    let thousand_tracks = vec![1; 1000];
    let thousand_and_one_tracks = vec![1; 1001];
    assert!(validate_track_batch(&thousand_tracks).is_ok());
    assert!(validate_track_batch(&thousand_and_one_tracks).is_err());
    assert!(validate_track_batch(&[]).is_err());
    assert!(validate_track_batch(&[1, 0]).is_err());
}

#[test]
fn playlist_names_and_tags_are_trimmed_before_the_request() {
    assert_eq!(validate_playlist_name("  Mix  ").ok(), Some("Mix"));
    assert_eq!(
        validate_playlist_tags(&[" rock ".to_owned(), "pop".to_owned()]).ok(),
        Some(vec!["rock".to_owned(), "pop".to_owned()])
    );
    assert!(validate_playlist_description("").is_ok());
}

#[tokio::test]
async fn mv_ids_are_validated_before_the_network() {
    let service = service_for_test();

    assert!(matches!(service.mv_detail(0).await, Err(AppError::Validation(_))));
    assert!(matches!(service.mv_url(-1, 1080).await, Err(AppError::Validation(_))));
    assert!(matches!(service.mv_url(1, 999).await, Err(AppError::Validation(_))));
    assert!(matches!(service.artist_mvs_page(0, 0).await, Err(AppError::Validation(_))));
    assert!(matches!(service.similar_mvs(0).await, Err(AppError::Validation(_))));
}

#[test]
fn phone_captcha_inputs_are_checked_before_the_network() {
    assert!(matches!(validate_phone("  "), Err(AppError::Validation(_))));
    assert!(matches!(validate_phone("1234"), Err(AppError::Validation(_))));
    assert!(matches!(validate_phone("1380013800x"), Err(AppError::Validation(_))));
    assert_eq!(validate_phone(" 13800138000 ").ok(), Some("13800138000"));

    assert!(matches!(validate_country_code(""), Err(AppError::Validation(_))));
    assert!(matches!(validate_country_code("86a"), Err(AppError::Validation(_))));
    assert_eq!(validate_country_code(" 86 ").ok(), Some("86"));

    assert!(matches!(validate_captcha(""), Err(AppError::Validation(_))));
    assert!(matches!(validate_captcha("12"), Err(AppError::Validation(_))));
    assert!(matches!(validate_captcha("123456789"), Err(AppError::Validation(_))));
    assert_eq!(validate_captcha(" 1597 ").ok(), Some("1597"));
}

#[test]
fn risk_control_keeps_its_user_readable_message_through_the_app_boundary() {
    let message = AppError::from(Error::RiskControl).to_string();
    assert!(message.contains("风控"), "{message}");
    assert!(AppError::from(Error::CaptchaSend).to_string().contains("验证码发送失败"));
    assert!(AppError::from(Error::CaptchaInvalid).to_string().contains("验证码错误"));
}

#[tokio::test]
async fn an_empty_device_key_is_rejected_before_the_network() {
    let service = service_for_test();

    assert!(matches!(service.revoke_device("  ").await, Err(AppError::Validation(_))));
}
