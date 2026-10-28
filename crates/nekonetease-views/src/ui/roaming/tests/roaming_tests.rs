use super::{RoamingSession, current_personal_roaming_song, trash_then_skip};
use nekonetease_core::entities::cloud::RoamingMode;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::AppError;
use nekonetease_engine::player::engine::queue::QueueState;
use nekonetease_engine::player::engine::state::PlayerState;
use nekonetease_engine::player::engine::types::PlaybackStatus;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const SONG_ID: i64 = 3_000_000_000;

fn session(mode: RoamingMode) -> RoamingSession {
    RoamingSession { request_id: 7, mode, queue_generation: 11, account_generation: 5 }
}

fn track(id: i64, file_path: &str) -> Arc<TrackSummary> {
    Arc::new(TrackSummary {
        id,
        file_path: file_path.to_owned(),
        file_name: String::new(),
        title: format!("Song {id}"),
        artist: None,
        album: None,
        duration_ms: 180_000,
        artwork_path: None,
        track_number: None,
        disc_number: None,
        last_position: 0,
        is_favorite: false,
        rating: 0,
        replaygain_track_gain: None,
        replaygain_track_peak: None,
        replaygain_album_gain: None,
        replaygain_album_peak: None,
        stream: None,
        mv_id: None,
    })
}

fn player(source: Arc<TrackSummary>, queued: Arc<TrackSummary>) -> PlayerState {
    let mut player = PlayerState {
        status: PlaybackStatus::Playing,
        source: Some(source),
        ..PlayerState::default()
    };
    player.queue = QueueState::default();
    player.queue.add_tracks(vec![queued]);
    let _ = player.queue.skip_to_index(0);
    player
}

fn current_song(
    request: u64,
    roaming: Option<RoamingSession>,
    queue: u64,
    account: u64,
    player: &PlayerState,
) -> Option<i64> {
    current_personal_roaming_song(request, roaming, queue, account, player)
}

#[test]
fn personal_fm_feedback_requires_the_same_live_cloud_track_and_session() {
    let source = track(SONG_ID, "");
    let mut player = player(source.clone(), source.clone());
    assert_eq!(
        current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &player),
        Some(SONG_ID)
    );

    player.status = PlaybackStatus::Paused;
    assert_eq!(
        current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &player),
        Some(SONG_ID),
        "paused playback is still the current song and may be marked not interested",
    );
}

#[test]
fn personal_fm_feedback_rejects_stale_sessions_and_local_or_replaced_tracks() {
    let source = track(SONG_ID, "");
    let queued = track(SONG_ID, "");
    let player_state = player(source.clone(), queued.clone());
    assert_eq!(current_song(6, Some(session(RoamingMode::Personal)), 11, 5, &player_state), None);
    assert_eq!(current_song(7, Some(session(RoamingMode::Personal)), 12, 5, &player_state), None);
    assert_eq!(current_song(7, Some(session(RoamingMode::Personal)), 11, 6, &player_state), None);
    assert_eq!(current_song(7, Some(session(RoamingMode::Heart)), 11, 5, &player_state), None);
    assert_eq!(current_song(7, None, 11, 5, &player_state), None);

    let local_source = player(track(SONG_ID, "/music/song.flac"), queued.clone());
    assert_eq!(current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &local_source), None);
    let local_queue = player(source.clone(), track(SONG_ID, "/music/song.flac"));
    assert_eq!(current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &local_queue), None);
    let replaced_queue = player(source, track(SONG_ID + 1, ""));
    assert_eq!(current_song(7, Some(session(RoamingMode::Personal)), 11, 5, &replaced_queue), None);
}

#[tokio::test]
async fn fm_feedback_skips_only_after_trash_succeeds() {
    let events = RefCell::new(Vec::new());
    let result = trash_then_skip(
        || true,
        async {
            events.borrow_mut().push("trash");
            Ok(())
        },
        || {
            events.borrow_mut().push("next");
            Ok(true)
        },
    )
    .await;

    assert!(matches!(result, Ok(true)));
    assert_eq!(*events.borrow(), ["trash", "next"]);
}

#[tokio::test]
async fn fm_feedback_keeps_the_song_when_trash_fails() {
    let events = RefCell::new(Vec::new());
    let result = trash_then_skip(
        || true,
        async {
            events.borrow_mut().push("trash");
            Err(AppError::network_msg("trash rejected"))
        },
        || {
            events.borrow_mut().push("next");
            Ok(true)
        },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(*events.borrow(), ["trash"]);
}

#[tokio::test]
async fn fm_feedback_does_not_advance_after_the_queue_changes_in_flight() {
    let checks = Cell::new(0);
    let events = RefCell::new(Vec::new());
    let result = trash_then_skip(
        || {
            checks.set(checks.get() + 1);
            checks.get() == 1
        },
        async {
            events.borrow_mut().push("trash");
            Ok(())
        },
        || {
            events.borrow_mut().push("next");
            Ok(true)
        },
    )
    .await;

    assert!(matches!(result, Ok(false)));
    assert_eq!(checks.get(), 2);
    assert_eq!(*events.borrow(), ["trash"]);
}

/// A roam is this page's own session: no My Library global, no tab plumbing and no `RoamingTab`.
#[test]
fn the_page_names_no_my_library_surface() {
    const SOURCES: [(&str, &str); 3] = [
        ("roaming.rs", include_str!("../mod.rs")),
        (
            "roaming-view.slint",
            include_str!("../../../../../nekonetease-ui/ui/views/roaming/roaming-view.slint"),
        ),
        ("roaming.slint", include_str!("../../../../../nekonetease-ui/ui/globals/roaming.slint")),
    ];
    for (name, source) in SOURCES {
        for needle in ["Playlists", "RoamingTab"] {
            assert!(!source.contains(needle), "{name} borrows {needle}");
        }
    }
}
