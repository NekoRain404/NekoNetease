//! The MV player's stream, quality and download handling.
//!
//! Kept apart from the page's detail and list wiring so the quality switch, which restarts the
//! same file at another tier and has to carry the position, pause state and volume across that
//! restart, reads as one concern.
//!
//! Every way of getting the video consults the local cache first. A hit opens the file through
//! the same player the network stream would use; a miss plays the stream and fills the cache once
//! playback is under way, so the first frame is never behind a download the listener did not ask
//! for.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use slint::ComponentHandle;

use crate::ui::util::parse_cloud_id;
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::MvStream;
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_engine::player::engine::state::{amplitude_to_volume, lock_state};
use nekonetease_ui::{AppWindow, MvDetail, MvPlayer};
use nekonetease_video::VideoState;

use super::cloud_mvs::MV_DETAIL_REQUEST;

/// Separates a quality pick from the one before it, so two quick picks cannot land out of
/// order. A close needs no bump — the `MvPlayer.open` guard drops the switch — but a fresh open
/// of the same MV does, since identity alone cannot tell it from the session the pick belongs to.
static MV_STREAM_REQUEST: AtomicU64 = AtomicU64::new(0);

/// The tiers the picker offers, widest first.
const MV_QUALITIES: [u16; 4] = [1080, 720, 480, 240];
const DEFAULT_MV_QUALITY: u16 = 1080;

/// The MV a quality switch restarts, and the tier it is currently playing. Kept beside the
/// player rather than in it, so the video crate stays free of `NetEase`'s tier numbers.
#[derive(Clone)]
struct PlayingMv {
    remote_id: String,
    id: i64,
    quality: u16,
}

/// What a fresh play or a tier switch was resolved to: a file already on disk, or a URL to open.
enum MvSource {
    Cached(PathBuf),
    Network(String),
}

pub(super) fn wire(ui: &AppWindow, state: &AppState) {
    let playing = Arc::new(Mutex::new(None));
    wire_play(ui, state, Arc::clone(&playing));
    wire_quality(ui, state, playing);
    wire_download(ui, state);
}

fn wire_play(ui: &AppWindow, state: &AppState, playing: Arc<Mutex<Option<PlayingMv>>>) {
    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_play(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        if detail.get_resolving() {
            return;
        }
        let remote_id = detail.get_remote_id().to_string();
        let Some(id) = parse_cloud_id(&remote_id) else { return };
        if !s.video.available() {
            toast::notify(
                ToastKind::OperationFailed,
                "Video playback is unavailable on this system.",
            );
            return;
        }

        // On this thread and before the request: the MV's own audio must be the only stream
        // once the player opens it.
        if let Err(error) = library::playback::player_pause(&s.playback_ctx()) {
            log::warn!("cloud_mvs::player_pause: {}", describe(&error));
        }
        // The engine's answer is already 0..=1 and mute-aware, which is what the video player
        // takes.
        let volume = lock_state(&s.player_state).effective_volume();
        let quality = pick_quality(ui.global::<MvPlayer>().get_quality());
        let request = MV_DETAIL_REQUEST.load(Ordering::Relaxed);

        detail.set_resolving(true);
        let s = s.clone();
        let weak = weak.clone();
        let playing = Arc::clone(&playing);
        s.runtime.clone().spawn(async move {
            let generation = s.cloud.status().generation;
            // Before the network: a video already on disk needs no ticket, and the ticket would
            // only be spent re-fetching bytes the cache holds.
            let source = resolve_source(&s, id, quality).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if MV_DETAIL_REQUEST.load(Ordering::Relaxed) != request
                    || s.cloud.status().generation != generation
                    || !ui.global::<MvDetail>().get_open()
                    || ui.global::<MvDetail>().get_remote_id().as_str() != remote_id
                {
                    return;
                }
                ui.global::<MvDetail>().set_resolving(false);
                let session = PlayingMv { remote_id, id, quality };
                match source {
                    Ok(MvSource::Cached(path)) => {
                        let Some(source) = path.to_str() else {
                            report_unopenable();
                            return;
                        };
                        if !begin_playback(&ui, &s, &playing, session, volume, source) {
                            report_unopenable();
                        }
                    }
                    Ok(MvSource::Network(url)) => {
                        if begin_playback(&ui, &s, &playing, session, volume, &url) {
                            cache_after_start(&s, id, quality, url);
                        } else {
                            report_unopenable();
                        }
                    }
                    Err(error) => {
                        let detail = describe(&error);
                        log::warn!("cloud_mvs::stream({id}): {detail}");
                        toast::notify(ToastKind::OperationFailed, detail);
                    }
                }
            });
        });
    });
}

fn wire_quality(ui: &AppWindow, state: &AppState, playing: Arc<Mutex<Option<PlayingMv>>>) {
    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvPlayer>().on_set_quality(move |picked| {
        let Some(ui) = weak.upgrade() else { return };
        let quality = pick_quality(picked);
        let player = ui.global::<MvPlayer>();
        player.set_quality(i32::from(quality));
        // Every pick retires the switch before it, including a pick back to the tier already
        // playing — otherwise the abandoned request would still land and restart the stream at
        // the tier the viewer just left.
        let request = MV_STREAM_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
        // With no stream loaded the pick is only the mark the next open starts from.
        let Some(session) = playing.lock().clone() else { return };
        if session.quality == quality {
            return;
        }

        let s = s.clone();
        let weak = weak.clone();
        let playing = Arc::clone(&playing);
        s.runtime.clone().spawn(async move {
            let generation = s.cloud.status().generation;
            let source = resolve_source(&s, session.id, quality).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if MV_STREAM_REQUEST.load(Ordering::Relaxed) != request
                    || s.cloud.status().generation != generation
                    || !ui.global::<MvPlayer>().get_open()
                    || playing.lock().as_ref().map(|live| live.remote_id.as_str())
                        != Some(session.remote_id.as_str())
                {
                    return;
                }
                let player = ui.global::<MvPlayer>();
                // The restart lands a request later, so what it hands over is measured here
                // rather than before the await: a tier switch must not rewind the viewer by the
                // fetch's own latency, nor undo a pause or a volume drag made while it ran.
                let volume = volume_amplitude(player.get_volume());
                let position = s.video.position_ms();
                let title = s.video.title();
                let video_state = s.video.state();
                let paused = matches!(&*video_state.borrow(), VideoState::Paused);
                let from_network = matches!(&source, Ok(MvSource::Network(_)));
                let Some(source) = (match source {
                    Ok(MvSource::Cached(path)) => path.to_str().map(str::to_owned),
                    Ok(MvSource::Network(url)) => Some(url),
                    Err(error) => {
                        player.set_quality(i32::from(session.quality));
                        log::warn!(
                            "cloud_mvs::stream({}, {quality}): {}",
                            session.id,
                            describe(&error)
                        );
                        toast::notify(ToastKind::OperationFailed, describe(&error));
                        return;
                    }
                }) else {
                    player.set_quality(i32::from(session.quality));
                    log::warn!("cloud_mvs: the MV cache entry is not a UTF-8 path");
                    return;
                };
                // The same handle replaces the file, so nothing of the previous tier survives
                // the switch beyond the position it hands over.
                if s.video.open_from(&source, volume, position).is_err() {
                    log::warn!("cloud_mvs: the video player refused the new MV stream");
                    player.set_quality(i32::from(session.quality));
                    return;
                }
                // A restart is the same MV at another tier, so it keeps the name the open gave
                // it; `open_from` would re-derive one from the new source.
                s.video.set_title(title);
                if paused {
                    s.video.pause();
                }
                if let Some(live) = playing.lock().as_mut() {
                    live.quality = quality;
                }
                if from_network {
                    cache_after_start(&s, session.id, quality, source);
                }
            });
        });
    });
}

/// Saves the tier the page is offering. Fire-and-forget: the file can take minutes, so the only
/// report is the toast when it lands or fails, and the Downloads folder opening on success.
fn wire_download(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    let s = state.clone();
    ui.global::<MvDetail>().on_download(move || {
        let Some(ui) = weak.upgrade() else { return };
        let detail = ui.global::<MvDetail>();
        let remote_id = detail.get_remote_id().to_string();
        let Some(id) = parse_cloud_id(&remote_id) else { return };
        let title = detail.get_title().to_string();
        let quality = pick_quality(ui.global::<MvPlayer>().get_quality());

        let s = s.clone();
        let weak = weak.clone();
        s.runtime.clone().spawn(async move {
            let result = save_video(&s, id, quality, &title).await;
            let _ = weak.upgrade_in_event_loop(move |_ui| match result {
                Ok(path) => {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_owned();
                    toast::notify(ToastKind::DownloadSaved, name);
                    if let Some(folder) = path.parent().map(std::path::Path::to_path_buf) {
                        let handle = s.runtime.clone();
                        handle.spawn(async move {
                            if let Err(error) = library::tracks::reveal_folder(folder).await {
                                log::warn!(
                                    "Could not reveal the downloaded video: {}",
                                    describe(&error)
                                );
                            }
                        });
                    }
                }
                Err(error) => {
                    let detail = describe(&error);
                    log::warn!("cloud_mvs::download({id}): {detail}");
                    toast::notify(ToastKind::OperationFailed, detail);
                }
            });
        });
    });
}

/// A fresh cache entry wins over the network. The entry is the same stream already on disk, so a
/// hit costs no ticket request and no bytes.
async fn resolve_source(state: &AppState, mv_id: i64, quality: u16) -> AppResult<MvSource> {
    if let Some(path) = library::mv_cache::cached_video(&state.paths, mv_id, quality) {
        if let Err(error) = library::mv_cache::mark_used(&path) {
            // A lease that could not be extended costs a re-download later, not this playback.
            log::warn!("Could not extend the MV cache lease: {}", describe(&error));
        }
        return Ok(MvSource::Cached(path));
    }
    match state.cloud.mv_url(mv_id, quality).await? {
        MvStream { url: Some(url) } if !url.is_empty() => Ok(MvSource::Network(url)),
        _ => Err(AppError::network_msg("No video stream was available for this music video.")),
    }
}

/// Opens `source` and, once it took, makes it the session a tier switch restarts.
fn begin_playback(
    ui: &AppWindow,
    state: &AppState,
    playing: &Arc<Mutex<Option<PlayingMv>>>,
    session: PlayingMv,
    volume: f64,
    source: &str,
) -> bool {
    // A failed open leaves no session behind: the stale one would answer a later tier pick with
    // the video this page has just replaced.
    *playing.lock() = None;
    if state.video.open(source, volume).is_err() {
        return false;
    }
    // An empty detail has not loaded yet; the source's own name beats a blank title bar.
    let title = ui.global::<MvDetail>().get_title();
    if !title.is_empty() {
        state.video.set_title(title);
    }
    let player = ui.global::<MvPlayer>();
    // The same value as a percentage for the overlay's slider. `amplitude_to_volume` lands in
    // 0..=100, so the conversion cannot fail; the fallback is the loudest the slider can go.
    player.set_volume(i32::try_from(amplitude_to_volume(volume)).unwrap_or(100));
    player.set_quality(i32::from(session.quality));
    // An earlier pick that has not landed yet belongs to the session this open just replaced,
    // whatever MV it names.
    MV_STREAM_REQUEST.fetch_add(1, Ordering::Relaxed);
    *playing.lock() = Some(session);
    true
}

/// Fills the cache once playback is under way. The listener is already watching, so a large video
/// must not delay the first frame behind its download.
fn cache_after_start(state: &AppState, mv_id: i64, quality: u16, url: String) {
    let paths = Arc::clone(&state.paths);
    state.runtime.spawn(async move {
        match library::mv_cache::cache_video(&paths, mv_id, quality, &url).await {
            Ok(Some(path)) => log::debug!("MV stream cached at {}", path.display()),
            Ok(None) => {}
            Err(error) => log::warn!("MV stream cache failed: {}", describe(&error)),
        }
    });
}

/// Resolves the chosen tier's URL and streams it to disk. Nothing about the URL is logged: it
/// carries a signed token.
async fn save_video(state: &AppState, mv_id: i64, quality: u16, title: &str) -> AppResult<PathBuf> {
    let url = match state.cloud.mv_url(mv_id, quality).await? {
        MvStream { url: Some(url) } if !url.is_empty() => url,
        _ => {
            return Err(AppError::network_msg(
                "No video stream was available for this music video.",
            ));
        }
    };
    library::mv_cache::download(&state.paths, title, &url).await
}

fn report_unopenable() {
    log::warn!("cloud_mvs: the video player could not open the MV");
    toast::notify(ToastKind::OperationFailed, "The music video could not be opened.");
}

/// Slint hands the pick across as an `int`; anything the picker cannot offer is the default
/// rather than a tier `mv_url` would reject.
pub(super) fn pick_quality(quality: i32) -> u16 {
    u16::try_from(quality)
        .ok()
        .filter(|quality| MV_QUALITIES.contains(quality))
        .unwrap_or(DEFAULT_MV_QUALITY)
}

/// The overlay's slider percentage as the amplitude the player takes.
pub(super) fn volume_amplitude(percent: i32) -> f64 {
    f64::from(percent.clamp(0, 100)) / 100.0
}
