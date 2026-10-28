//! The MV player's Slint half: commands out, frames and state in.
//!
//! Three payload loops, each priming on `borrow_and_update` before it waits — `watch` holds the
//! latest value and the first one is real state. `VideoPlayer` publishes at most one frame per mpv
//! update, and a slow UI drops the ones it missed rather than queueing them, which is what keeps a
//! popped-out video from arriving late and playing catch-up.
//!
//! The one copy of a frame on this side is into Slint's pixel buffer: `VideoFrame` owns a
//! `Vec<u8>` for the render thread, and an `Image` needs a `SharedPixelBuffer`, so the two cannot
//! be the same allocation.

use std::sync::Arc;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use nekonetease_app::state::AppState;
use nekonetease_core::error::AppError;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, MvPlayer};
use nekonetease_video::{VideoFrame, VideoState};

/// Wire the overlay's two controls and its three subscriptions.
///
/// Called once, from `boot::ui_setup::install`. A failure to spawn a UI-thread task is reported so
/// a missing overlay is one log line rather than a window that silently does nothing.
pub fn install(app: &AppWindow, state: &AppState) -> Result<(), AppError> {
    install_callbacks(app, state);
    install_state_loop(app, state)?;
    install_frame_loop(app, state)?;
    install_progress_loop(app, state)
}

fn install_callbacks(app: &AppWindow, state: &AppState) {
    let global = app.global::<MvPlayer>();

    let video = Arc::clone(&state.video);
    let weak = app.as_weak();
    global.on_close(move || {
        video.stop();
        // The worker publishes `Idle` a moment later; closing the surface on the press keeps
        // the overlay from lingering over a stopped player when the thread is busy.
        if let Some(ui) = weak.upgrade() {
            let g = ui.global::<MvPlayer>();
            leave_fullscreen(&ui, &g);
            g.set_open(false);
        }
    });

    let video = Arc::clone(&state.video);
    global.on_toggle_play(move || {
        let receiver = video.state();
        match &*receiver.borrow() {
            VideoState::Playing => video.pause(),
            VideoState::Paused => video.resume(),
            _ => {}
        }
    });

    let video = Arc::clone(&state.video);
    global.on_seek(move |ms| {
        // A key or drag can land before mpv knows the duration, where there is nothing to
        // clamp against; mpv clamps the target itself either way.
        let target = u64::try_from(ms).unwrap_or(0);
        let duration = video.duration_ms();
        video.seek_ms(if duration > 0 { target.min(duration) } else { target });
    });

    let video = Arc::clone(&state.video);
    let weak = app.as_weak();
    global.on_set_volume(move |percent| {
        let percent = percent.clamp(0, 100);
        video.set_volume(f64::from(percent) / 100.0);
        // The scale is read back by a quality switch, which hands it to the replacement file.
        if let Some(ui) = weak.upgrade() {
            ui.global::<MvPlayer>().set_volume(percent);
        }
    });

    let weak = app.as_weak();
    global.on_set_fullscreen(move |value| {
        let Some(ui) = weak.upgrade() else { return };
        ui.global::<MvPlayer>().set_fullscreen(value);
        ui.window().set_fullscreen(value);
    });
}

/// Hands the window back its chrome. A closed player that stayed fullscreen would leave the
/// desktop to Escape, which the overlay no longer handles.
fn leave_fullscreen(ui: &AppWindow, g: &MvPlayer<'_>) {
    if g.get_fullscreen() {
        g.set_fullscreen(false);
        ui.window().set_fullscreen(false);
    }
}

fn install_state_loop(app: &AppWindow, state: &AppState) -> Result<(), AppError> {
    let weak = app.as_weak();
    let video = Arc::clone(&state.video);
    let mut states = state.video.state();
    slint::spawn_local(async_compat::Compat::new(async move {
        loop {
            apply_state(&weak, &video, states.borrow_and_update().clone());
            if states.changed().await.is_err() {
                break;
            }
        }
    }))
    .map(|_| ())
    .map_err(|e| AppError::io("MV state subscriber", e))
}

fn install_frame_loop(app: &AppWindow, state: &AppState) -> Result<(), AppError> {
    let weak = app.as_weak();
    let mut frames = state.video.frames();
    slint::spawn_local(async_compat::Compat::new(async move {
        loop {
            let frame = frames.borrow_and_update().clone();
            publish_frame(&weak, frame.as_deref());
            if frames.changed().await.is_err() {
                break;
            }
        }
    }))
    .map(|_| ())
    .map_err(|e| AppError::io("MV frame subscriber", e))
}

fn install_progress_loop(app: &AppWindow, state: &AppState) -> Result<(), AppError> {
    let weak = app.as_weak();
    let mut progress = state.video.progress();
    slint::spawn_local(async_compat::Compat::new(async move {
        loop {
            let (position_ms, duration_ms) = *progress.borrow_and_update();
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<MvPlayer>();
                g.set_position_ms(ms_to_i32(position_ms));
                g.set_duration_ms(ms_to_i32(duration_ms));
            }
            if progress.changed().await.is_err() {
                break;
            }
        }
    }))
    .map(|_| ())
    .map_err(|e| AppError::io("MV progress subscriber", e))
}

/// Mirror one engine state onto the overlay.
///
/// `Failed` closes the surface rather than keeping a dead player on screen; the reason goes to the
/// toast stack, which is the surface that already reports a track that would not open.
fn apply_state(
    weak: &slint::Weak<AppWindow>,
    video: &nekonetease_video::VideoPlayer,
    state: VideoState,
) {
    let Some(ui) = weak.upgrade() else { return };
    let g = ui.global::<MvPlayer>();
    match state {
        VideoState::Idle => {
            leave_fullscreen(&ui, &g);
            g.set_open(false);
            g.set_loading(false);
            g.set_playing(false);
            g.set_error(slint::SharedString::default());
            g.set_frame(Image::default());
            g.set_has_frame(false);
        }
        VideoState::Loading => {
            g.set_title(video.title().into());
            g.set_open(true);
            g.set_loading(true);
            g.set_playing(false);
            g.set_error(slint::SharedString::default());
        }
        VideoState::Playing => {
            g.set_open(true);
            g.set_loading(false);
            g.set_playing(true);
        }
        VideoState::Paused => {
            g.set_open(true);
            g.set_loading(false);
            g.set_playing(false);
        }
        VideoState::Ended => {
            leave_fullscreen(&ui, &g);
            g.set_open(false);
            g.set_playing(false);
            g.set_frame(Image::default());
            g.set_has_frame(false);
        }
        VideoState::Failed(reason) => {
            leave_fullscreen(&ui, &g);
            g.set_open(false);
            g.set_playing(false);
            g.set_error(slint::SharedString::default());
            g.set_frame(Image::default());
            g.set_has_frame(false);
            toast::notify(ToastKind::PlaybackFailed, reason);
        }
    }
}

fn publish_frame(weak: &slint::Weak<AppWindow>, frame: Option<&VideoFrame>) {
    let Some(ui) = weak.upgrade() else { return };
    let g = ui.global::<MvPlayer>();
    if let Some(image) = frame.and_then(image_of) {
        g.set_frame(image);
        g.set_has_frame(true);
    } else {
        g.set_frame(Image::default());
        g.set_has_frame(false);
    }
}

/// Copy an RGBA8 frame into Slint's image buffer. `None` for a frame whose pixel vector is
/// shorter than its dimensions claim, which is the only way the copy below could overrun.
fn image_of(frame: &VideoFrame) -> Option<Image> {
    if frame.width == 0 || frame.height == 0 || frame.pixels.len() < frame.byte_len() {
        return None;
    }
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(frame.width, frame.height);
    buffer.make_mut_bytes().copy_from_slice(&frame.pixels[..frame.byte_len()]);
    Some(Image::from_rgba8(buffer))
}

fn ms_to_i32(ms: u64) -> i32 {
    i32::try_from(ms).unwrap_or(i32::MAX)
}

#[cfg(test)]
#[path = "tests/mv_player_tests.rs"]
mod tests;
