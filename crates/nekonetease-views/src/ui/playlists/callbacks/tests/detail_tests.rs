use super::{PlaylistPlaybackAction as Action, playback_action};
use nekonetease_ui::{PlayerVm, TrackListRow, TrackSummaryRow};
use slint::{ModelRc, SharedString, VecModel};

fn player(id: &str, status: &str, cloud: bool) -> PlayerVm {
    PlayerVm {
        has_track: true,
        status: status.into(),
        is_playing: status == "playing",
        track: TrackSummaryRow {
            remote_id: id.into(),
            file_path: if cloud { SharedString::default() } else { "local.mp3".into() },
            ..TrackSummaryRow::default()
        },
        ..PlayerVm::default()
    }
}

fn rows(id: &str) -> ModelRc<TrackListRow> {
    ModelRc::new(VecModel::from(vec![TrackListRow {
        remote_id: id.into(),
        ..TrackListRow::default()
    }]))
}

#[test]
fn playlist_controls_follow_the_source_and_transport_transitions() {
    assert_eq!(
        playback_action(&player("42", "playing", true), &rows("42"), true, true),
        Action::Pause
    );
    assert_eq!(
        playback_action(&player("42", "paused", true), &rows("42"), true, true),
        Action::Resume
    );
    assert_eq!(
        playback_action(&player("42", "stopped", true), &rows("42"), true, true),
        Action::Start
    );
    assert_eq!(
        playback_action(&player("42", "playing", true), &rows("43"), true, true),
        Action::Start
    );
    assert_eq!(
        playback_action(&player("42", "paused", true), &rows("43"), true, true),
        Action::Start
    );
    assert_eq!(playback_action(&PlayerVm::default(), &rows("42"), true, true), Action::Start);
}

#[test]
fn pending_or_empty_playlists_cannot_control_the_previous_source() {
    assert_eq!(
        playback_action(&player("42", "playing", true), &rows("42"), true, false),
        Action::Unavailable
    );
    assert_eq!(
        playback_action(&player("42", "playing", true), &ModelRc::default(), true, true),
        Action::Unavailable
    );
}

#[test]
fn source_matching_keeps_cloud_ids_exact_and_separate_from_local_ids() {
    assert_eq!(
        playback_action(&player("3000000000", "playing", true), &rows("3000000001"), true, true),
        Action::Start
    );
    assert_eq!(
        playback_action(&player("3000000000", "playing", true), &rows("3000000000"), true, true),
        Action::Pause
    );
    assert_eq!(
        playback_action(&player("42", "playing", false), &rows("42"), true, true),
        Action::Start
    );
    assert_eq!(
        playback_action(&player("42", "playing", true), &rows("42"), false, true),
        Action::Start
    );
    assert_eq!(
        playback_action(&player("42", "playing", false), &rows("42"), false, true),
        Action::Pause
    );
}

/// Exercises the compiled bindings too: model and player changes must invalidate the decision
/// read by the click handler, including the pending header introduced by asynchronous opens.
#[cfg(target_os = "windows")]
#[test]
fn compiled_playlist_controls_react_to_player_and_playlist_changes()
-> Result<(), slint::PlatformError> {
    use nekonetease_ui::{AppWindow, Player, PlaylistDetail};
    use slint::{ComponentHandle, Model};

    let ui = AppWindow::new()?;
    let detail = ui.global::<PlaylistDetail>();
    detail.on_resolve_playback_action(|player, rows, cloud, ready| {
        playback_action(&player, &rows, cloud, ready)
    });
    detail.set_playlist_id(1);
    detail.set_cloud_collection(true);
    detail.set_tracks(rows("3000000000"));
    let transport = ui.global::<Player>();

    transport.set_vm(player("3000000001", "playing", true));
    assert_eq!(detail.get_playback_action(), Action::Start);
    transport.set_vm(player("3000000000", "playing", true));
    assert_eq!(detail.get_playback_action(), Action::Pause);
    transport.set_vm(player("3000000000", "paused", true));
    assert_eq!(detail.get_playback_action(), Action::Resume);
    transport.set_vm(player("3000000000", "stopped", true));
    assert_eq!(detail.get_playback_action(), Action::Start);

    transport.set_vm(player("3000000000", "playing", true));
    detail.set_loading(true);
    assert_eq!(detail.get_playback_action(), Action::Unavailable);
    detail.set_tracks(rows("3000000001"));
    detail.set_playlist_id(2);
    detail.set_loading(false);
    assert_eq!(detail.get_playback_action(), Action::Start);
    let live_rows = std::rc::Rc::new(VecModel::from(vec![TrackListRow {
        remote_id: "3000000000".into(),
        ..TrackListRow::default()
    }]));
    detail.set_tracks(ModelRc::from(live_rows.clone()));
    assert_eq!(detail.get_playback_action(), Action::Pause);
    live_rows.set_row_data(
        0,
        TrackListRow { remote_id: "3000000001".into(), ..TrackListRow::default() },
    );
    assert_eq!(detail.get_playback_action(), Action::Start);
    live_rows.set_vec(Vec::new());
    assert_eq!(detail.get_playback_action(), Action::Unavailable);
    live_rows.push(TrackListRow { remote_id: "3000000000".into(), ..TrackListRow::default() });
    assert_eq!(detail.get_playback_action(), Action::Pause);
    detail.set_playlist_id(-1);
    assert_eq!(detail.get_playback_action(), Action::Unavailable);
    Ok(())
}
