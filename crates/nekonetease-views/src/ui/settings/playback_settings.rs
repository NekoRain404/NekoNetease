//! Wire the Settings page's Playback section to Rust.
//!
//! Everything here is seeded from one `settings.json` read, and most of it takes the
//! two-phase shape the equalizer and `ReplayGain` callbacks use: apply to the live
//! backend synchronously, then persist on the blocking pool. Three exceptions:
//!
//! - **Play-button animation** needs no runtime mirror — `PlayButton` binds its overlays
//!   to the Slint global, so the chip click repaints reactively.
//! - **The crossfade duration slider** splits `changed` (live, no disk) from `committed`
//!   (drag release, persists), as `set-volume` / `commit-volume` do.
//!
//! Gapless defaults to `true` in two places that already agree — `PlaybackFlags::default()`
//! and `PlayerState::default()` — so a first launch reads no file, falls through to
//! both, and paints the toggle on.

use slint::ComponentHandle;

use crate::ui::settings_bind::toggle_binding;
use nekonetease_app::library;
use nekonetease_app::services::settings;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::Quality;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_playback::player::playback::crossfade;
use nekonetease_ui::{AppWindow, Settings};

/// The on-disk token as a Slint chip index, in lock-step with the inline literal in
/// `views/settings/playback-section.slint` — both sides have to agree on the order or
/// the chip paints the wrong selection. Anything unrecognised lands on index 0, matching
/// `SettingsData::default()`.
fn play_button_anim_idx_from_token(token: &str) -> i32 {
    match token {
        "equalizer" => 1,
        _ => 0,
    }
}

fn play_button_anim_token_from_idx(idx: i32) -> &'static str {
    match idx {
        1 => "equalizer",
        _ => "none",
    }
}

pub fn install_playback_settings(ui: &AppWindow, state: &AppState) {
    // An unreadable file leaves the Slint defaults in place, matching first launch. The
    // slider's range comes from the player's own constants, seeded ahead of the disk
    // read so the value below always lands inside a valid track.
    {
        let g = ui.global::<Settings>();
        g.set_crossfade_min_secs(crossfade::crossfade_ms_to_secs(crossfade::MIN_CROSSFADE_MS));
        g.set_crossfade_max_secs(crossfade::crossfade_ms_to_secs(crossfade::MAX_CROSSFADE_MS));
    }

    if let Ok(s) = settings::read_settings(&state.paths) {
        let g = ui.global::<Settings>();
        g.set_gapless_playback(s.playback.gapless_playback);
        g.set_hardware_video_decoding(s.playback.hwdec_enabled);
        g.set_play_button_animation_idx(play_button_anim_idx_from_token(&s.play_button_animation));
        g.set_cloud_quality(s.playback.cloud_quality.level().into());

        g.set_crossfade_enabled(s.crossfade.crossfade_enabled);
        g.set_crossfade_duration_secs(crossfade::crossfade_ms_to_secs(
            s.crossfade.crossfade_duration_ms,
        ));
        g.set_crossfade_skip_same_album(s.crossfade.crossfade_skip_same_album);
        g.set_crossfade_manual(s.crossfade.crossfade_manual);
        g.set_crossfade_fade_on_pause(s.crossfade.crossfade_fade_on_pause);
    }

    let state_clone = state.clone();
    ui.global::<Settings>().on_gapless_playback_changed(move |on| {
        // Synchronous: `player_set_gapless` publishes a new ViewModel through
        // `with_state_emit`, so the bar's flag and the monitor's gate both refresh
        // before this callback returns.
        if let Err(e) = library::playback::player_set_gapless(&state_clone.playback_ctx(), on) {
            log::warn!("player_set_gapless runtime apply: {}", describe(&e));
        }
        state_clone.persist_blocking("persist gapless_playback", move |paths| {
            library::settings::set_gapless_playback(paths, on)
        });
    });

    let state_hwdec = state.clone();
    ui.global::<Settings>().on_hardware_video_decoding_changed(move |on| {
        // Synchronous: the MV worker sees the new mode on its next command drain, and a file
        // already playing keeps its decoder until the next frame mpv reconfigures.
        state_hwdec.video.set_hwdec(on);
        state_hwdec.persist_blocking("persist hwdec_enabled", move |paths| {
            library::settings::set_hwdec_enabled(paths, on)
        });
    });

    let state_anim = state.clone();
    ui.global::<Settings>().on_play_button_animation_changed(move |idx| {
        // The runtime effect is already reactive off the global, so only the disk
        // write happens here.
        let token = play_button_anim_token_from_idx(idx).to_owned();
        state_anim.persist_blocking("persist play_button_animation", move |paths| {
            library::settings::set_play_button_animation(paths, token)
        });
    });

    let state_quality = state.clone();
    ui.global::<Settings>().on_cloud_quality_changed(move |token| {
        let quality = Quality::from_level(token.as_str());
        // `change_quality` pauses the current cloud deck synchronously, then fetches and opens
        // the replacement source on the runtime. It resumes from the captured position only
        // after the new source has been handed to the engine.
        let change = state_quality.cloud_playback.change_quality(quality);
        state_quality.runtime.spawn(async move {
            if let Err(error) = change.await {
                let detail = format!("音质切换失败，播放已暂停：{}", describe(&error));
                log::warn!("cloud quality change: {detail}");
                toast::notify(ToastKind::OperationFailed, detail);
            }
        });
        state_quality.persist_blocking("persist cloud_quality", move |paths| {
            library::settings::set_cloud_quality(paths, quality)
        });
    });

    install_crossfade_callbacks(ui, state);
}

/// The five crossfade callbacks. The four toggles take [`toggle_binding`]'s shared
/// apply-then-persist shape; only the duration slider needs its own pair, splitting the
/// live drag from the release.
fn install_crossfade_callbacks(ui: &AppWindow, state: &AppState) {
    let g = ui.global::<Settings>();

    g.on_crossfade_enabled_changed(toggle_binding(
        state,
        "persist crossfade_enabled",
        library::playback::player_set_crossfade_enabled,
        library::settings::set_crossfade_enabled,
    ));
    g.on_crossfade_skip_same_album_changed(toggle_binding(
        state,
        "persist crossfade_skip_same_album",
        library::playback::player_set_crossfade_skip_same_album,
        library::settings::set_crossfade_skip_same_album,
    ));
    g.on_crossfade_manual_changed(toggle_binding(
        state,
        "persist crossfade_manual",
        library::playback::player_set_crossfade_manual,
        library::settings::set_crossfade_manual,
    ));
    g.on_crossfade_fade_on_pause_changed(toggle_binding(
        state,
        "persist crossfade_fade_on_pause",
        library::playback::player_set_crossfade_fade_on_pause,
        library::settings::set_crossfade_fade_on_pause,
    ));

    // Live drag: apply to the backend and write the clamped value back, so the readout
    // tracks the thumb. No disk write — that is `committed`'s.
    let state_drag = state.clone();
    let weak = ui.as_weak();
    g.on_crossfade_duration_changed(move |secs| {
        let ms = crossfade::secs_to_crossfade_ms(secs);
        library::playback::player_set_crossfade_duration_ms(&state_drag.playback_ctx(), ms);
        if let Some(ui) = weak.upgrade() {
            ui.global::<Settings>()
                .set_crossfade_duration_secs(crossfade::crossfade_ms_to_secs(ms));
        }
    });

    let state_commit = state.clone();
    g.on_crossfade_duration_committed(move |secs| {
        let ms = crossfade::secs_to_crossfade_ms(secs);
        state_commit.persist_blocking("persist crossfade_duration_ms", move |paths| {
            library::settings::set_crossfade_duration_ms(paths, ms)
        });
    });
}
