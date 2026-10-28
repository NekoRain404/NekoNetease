//! The two threads that own libmpv.
//!
//! The event thread owns the `Mpv` handle and is the only thread that waits on its events or
//! issues commands. The render thread is separate because libmpv requires every
//! `mpv_render_*` call on one thread and forbids them from the update callback: one blocked in
//! `mpv_wait_event` could not also wait on the render condvar without polling.
//!
//! Commands wake the event thread through `mpv_wakeup`; a new frame wakes the render thread
//! through the update callback. Neither path polls or renders on a timer.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::mpsc::error::TryRecvError;

use libmpv2::events::{Event, PropertyData};
use libmpv2::mpv_end_file_reason;
use libmpv2::{Format, Mpv};
use tokio::sync::{mpsc, watch};

use crate::frame::{
    VideoFrame, VideoState, hwdec_option, mpv_error_text, render_size, secs_to_ms, volume_to_mpv,
};
use crate::player::Command;

mod render;

pub(crate) use self::render::RenderControl;

const POSITION_ID: u64 = 1;
const DURATION_ID: u64 = 2;
const HWDEC_ID: u64 = 3;

pub(crate) struct Shared {
    pub state_tx: watch::Sender<VideoState>,
    pub frames_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    pub progress_tx: watch::Sender<(u64, u64)>,
    pub position_ms: AtomicU64,
    pub duration_ms: AtomicU64,
    pub hwdec_enabled: Arc<AtomicBool>,
    pub wake: Arc<WakeHandle>,
}

/// The mpv handle an external thread may wake. Stored as an address because the handle is
/// created before this value is shared and must outlive every wake.
#[derive(Default)]
pub(crate) struct WakeHandle {
    address: Mutex<Option<usize>>,
}

impl WakeHandle {
    fn lock(&self) -> MutexGuard<'_, Option<usize>> {
        self.address.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn install(&self, address: usize) {
        *self.lock() = Some(address);
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    #[allow(
        unsafe_code,
        reason = "libmpv2 registers its wakeup callback in terms of this handle, which has no safe setter on `&Mpv`"
    )]
    pub fn wake(&self) {
        let address = *self.lock();
        let Some(address) = address else { return };
        // SAFETY: installed by the event thread for the life of its `Mpv`, and cleared
        // before that handle drops. libmpv documents `mpv_wakeup` as callable from any
        // thread and safe to race with its own event loop.
        unsafe {
            libmpv2_sys::mpv_wakeup(std::ptr::with_exposed_provenance_mut(address));
        }
    }
}

/// A file the event thread is currently playing, and what the last event left it believing.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the flags describe separate aspects of one file's session"
)]
#[derive(Default)]
struct Session {
    paused: bool,
    active_file: bool,
    size: Option<(u32, u32)>,
    software_logged: bool,
    /// Set between a replacement `loadfile` and the new file's `StartFile`, so the outgoing
    /// file's `Stop` is not mistaken for the end of the session.
    switching: bool,
    shutdown: bool,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the handle must drop on the thread that waited on it, after the render thread is joined"
)]
pub(crate) fn run(mpv: Mpv, mut commands: mpsc::UnboundedReceiver<Command>, shared: Arc<Shared>) {
    shared.wake.install(mpv.ctx.as_ptr().expose_provenance());
    let control = Arc::new(RenderControl::default());
    let render_thread = render::spawn_renderer(
        mpv.ctx.as_ptr().expose_provenance(),
        Arc::clone(&control),
        shared.frames_tx.clone(),
    );
    observe_properties(&mpv);
    let mut session = Session::default();

    'events: loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => break 'events,
                Ok(command) => handle_command(&mpv, &shared, &control, command, &mut session),
                Err(TryRecvError::Empty) => break,
            }
        }
        let Some(event) = mpv.wait_event(-1.0) else {
            continue;
        };
        match event {
            Ok(event) => handle_event(&mpv, &shared, &control, event, &mut session),
            Err(error) => {
                log::warn!("MV playback event failed: {}", mpv_error_text(&error));
                end_session(
                    &shared,
                    &control,
                    &mut session,
                    VideoState::Failed(format!(
                        "Video playback failed: {}",
                        mpv_error_text(&error)
                    )),
                );
            }
        }
        if session.shutdown {
            break;
        }
    }

    control.stop();
    publish_state(&shared, VideoState::Idle);
    shared.frames_tx.send_replace(None);
    shared.progress_tx.send_replace((0, 0));
    shared.wake.clear();
    if let Some(thread) = render_thread
        && thread.join().is_err()
    {
        log::warn!("MV render thread panicked");
    }
}

fn observe_properties(mpv: &Mpv) {
    for (name, format, id) in [
        ("time-pos", Format::Double, POSITION_ID),
        ("duration", Format::Double, DURATION_ID),
        ("hwdec-current", Format::String, HWDEC_ID),
    ] {
        if let Err(error) = mpv.observe_property(name, format, id) {
            log::debug!("MV could not observe {name}: {}", mpv_error_text(&error));
        }
    }
}

fn handle_event(
    mpv: &Mpv,
    shared: &Shared,
    control: &RenderControl,
    event: Event<'_>,
    session: &mut Session,
) {
    match event {
        Event::StartFile => {
            session.switching = false;
            publish_state(shared, VideoState::Loading);
        }
        Event::FileLoaded => {
            session.active_file = true;
            session.size = video_size(mpv);
            let state = if session.paused { VideoState::Paused } else { VideoState::Playing };
            publish_state(shared, state);
            // A file loaded paused — a quality switch — has no earlier frame to keep, so the
            // renderer is armed once for the frame that arrived with it. Playing needs the same
            // arm, and neither path polls after this.
            if let Some(size) = session.size {
                control.enable(size);
            }
        }
        Event::VideoReconfig => {
            if let Some(size) = video_size(mpv) {
                session.size = Some(size);
                if session.active_file && !session.paused {
                    control.enable(size);
                }
            }
        }
        Event::PlaybackRestart => {
            if session.active_file && !session.paused {
                publish_state(shared, VideoState::Playing);
                if let Some(size) = session.size {
                    control.enable(size);
                }
            }
        }
        Event::EndFile(reason) => {
            if session.switching && reason == mpv_end_file_reason::Stop {
                return;
            }
            if reason == mpv_end_file_reason::Eof {
                end_session(shared, control, session, VideoState::Ended);
            } else if reason == mpv_end_file_reason::Error {
                end_session(
                    shared,
                    control,
                    session,
                    VideoState::Failed("Video playback failed".to_owned()),
                );
            } else if reason == mpv_end_file_reason::Stop || reason == mpv_end_file_reason::Quit {
                end_session(shared, control, session, VideoState::Idle);
            }
        }
        Event::PropertyChange { name, change, .. } => {
            handle_property(shared, session, name, change);
        }
        Event::Shutdown => session.shutdown = true,
        _ => {}
    }
}

fn handle_property(shared: &Shared, session: &mut Session, name: &str, change: PropertyData<'_>) {
    match (name, change) {
        ("time-pos", PropertyData::Double(seconds)) => {
            shared.position_ms.store(secs_to_ms(seconds), Ordering::Relaxed);
            publish_progress(shared);
        }
        ("duration", PropertyData::Double(seconds)) => {
            shared.duration_ms.store(secs_to_ms(seconds), Ordering::Relaxed);
            publish_progress(shared);
        }
        ("hwdec-current", PropertyData::Str(current)) => {
            check_software(shared, session, current);
        }
        _ => {}
    }
}

fn check_software(shared: &Shared, session: &mut Session, current: &str) {
    if current != "no"
        || session.software_logged
        || session.size.is_none()
        || !shared.hwdec_enabled.load(Ordering::Relaxed)
    {
        return;
    }
    log::info!("MV playback: hardware decoding was requested but mpv selected software decoding");
    session.software_logged = true;
}

fn handle_command(
    mpv: &Mpv,
    shared: &Shared,
    control: &RenderControl,
    command: Command,
    session: &mut Session,
) {
    match command {
        Command::Open { url, volume, start_ms } => {
            open(mpv, shared, control, &url, volume, start_ms, session);
        }
        Command::Seek(ms) => seek(mpv, shared, control, ms, session),
        Command::Pause => pause(mpv, shared, control, session),
        Command::Resume => resume(mpv, shared, control, session),
        Command::Stop => stop(mpv, shared, control, session),
        Command::SetVolume(volume) => {
            if let Err(error) = mpv.set_property("volume", volume_to_mpv(volume)) {
                log::warn!("MV volume could not be set: {}", mpv_error_text(&error));
            }
        }
        Command::SetHwdec(enabled) => switch_hwdec(mpv, shared, control, enabled, session),
        Command::Shutdown => {}
    }
}

fn open(
    mpv: &Mpv,
    shared: &Shared,
    control: &RenderControl,
    url: &str,
    volume: f64,
    start_ms: u64,
    session: &mut Session,
) {
    session.paused = false;
    session.active_file = true;
    session.size = None;
    session.software_logged = false;
    session.switching = true;
    shared.position_ms.store(start_ms, Ordering::Relaxed);
    shared.duration_ms.store(0, Ordering::Relaxed);
    publish_progress(shared);
    publish_state(shared, VideoState::Loading);
    control.reset();
    shared.frames_tx.send_replace(None);

    if let Err(error) = mpv.set_property("volume", volume_to_mpv(volume)) {
        log::warn!("MV volume could not be set: {}", mpv_error_text(&error));
    }
    if let Err(error) = mpv.set_property("pause", false) {
        log::warn!("MV could not leave the paused state: {}", mpv_error_text(&error));
    }
    let start = start_option(start_ms);
    let mut args = vec![url, "replace", "-1"];
    if let Some(start) = start.as_deref() {
        args.push(start);
    }
    if let Err(error) = mpv.command("loadfile", &args) {
        log::warn!("MV could not open {url}: {}", mpv_error_text(&error));
        end_session(
            shared,
            control,
            session,
            VideoState::Failed(format!("Could not open the video: {}", mpv_error_text(&error))),
        );
    }
}

/// The `loadfile` option that resumes a file at `start_ms`; absent at the beginning so the
/// ordinary open keeps mpv's own default.
fn start_option(start_ms: u64) -> Option<String> {
    (start_ms > 0).then(|| format!("start={}", mpv_seconds(start_ms)))
}

/// mpv takes a position in seconds, and the milliseconds it is handed are exact in decimal where
/// they are not in a float — integer division spells the same number without widening anything.
fn mpv_seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn seek(mpv: &Mpv, shared: &Shared, control: &RenderControl, ms: u64, session: &Session) {
    let seconds = mpv_seconds(ms);
    match mpv.command("seek", &[&seconds, "absolute+exact"]) {
        Ok(()) => {
            shared.position_ms.store(ms, Ordering::Relaxed);
            publish_progress(shared);
            // A paused file produces no later frame on its own, so the seek's picture would
            // never replace the one still on screen unless the renderer is armed again.
            if session.paused
                && let Some(size) = session.size
            {
                control.enable(size);
            }
        }
        Err(error) => log::warn!("MV could not seek: {}", mpv_error_text(&error)),
    }
}

fn pause(mpv: &Mpv, shared: &Shared, control: &RenderControl, session: &mut Session) {
    session.paused = true;
    if let Err(error) = mpv.set_property("pause", true) {
        log::warn!("MV could not pause: {}", mpv_error_text(&error));
    }
    if session.active_file {
        publish_state(shared, VideoState::Paused);
    }
    control.disable();
}

fn resume(mpv: &Mpv, shared: &Shared, control: &RenderControl, session: &mut Session) {
    session.paused = false;
    if let Err(error) = mpv.set_property("pause", false) {
        log::warn!("MV could not resume: {}", mpv_error_text(&error));
    }
    if session.active_file {
        publish_state(shared, VideoState::Playing);
        if let Some(size) = session.size {
            control.enable(size);
        }
    }
}

fn stop(mpv: &Mpv, shared: &Shared, control: &RenderControl, session: &mut Session) {
    if let Err(error) = mpv.command("stop", &[]) {
        log::warn!("MV could not stop: {}", mpv_error_text(&error));
    }
    end_session(shared, control, session, VideoState::Idle);
}

fn switch_hwdec(
    mpv: &Mpv,
    shared: &Shared,
    control: &RenderControl,
    enabled: bool,
    session: &mut Session,
) {
    match mpv.set_property("hwdec", hwdec_option(enabled)) {
        Ok(()) => {
            shared.hwdec_enabled.store(enabled, Ordering::Relaxed);
            session.software_logged = false;
        }
        Err(error) => {
            log::warn!("MV hardware decoding could not be changed: {}", mpv_error_text(&error));
            if session.active_file {
                end_session(
                    shared,
                    control,
                    session,
                    VideoState::Failed(format!(
                        "Hardware video decoding could not be changed: {}",
                        mpv_error_text(&error)
                    )),
                );
            }
        }
    }
}

fn end_session(shared: &Shared, control: &RenderControl, session: &mut Session, state: VideoState) {
    session.active_file = false;
    session.paused = false;
    session.switching = false;
    session.size = None;
    publish_state(shared, state);
    control.reset();
    shared.frames_tx.send_replace(None);
    shared.position_ms.store(0, Ordering::Relaxed);
    shared.duration_ms.store(0, Ordering::Relaxed);
    publish_progress(shared);
}

fn publish_state(shared: &Shared, state: VideoState) {
    shared.state_tx.send_if_modified(|current| {
        if *current == state {
            false
        } else {
            *current = state;
            true
        }
    });
}

fn publish_progress(shared: &Shared) {
    let next =
        (shared.position_ms.load(Ordering::Relaxed), shared.duration_ms.load(Ordering::Relaxed));
    shared.progress_tx.send_if_modified(|current| {
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    });
}

fn video_size(mpv: &Mpv) -> Option<(u32, u32)> {
    let width = mpv.get_property::<i64>("dwidth").ok()?;
    let height = mpv.get_property::<i64>("dheight").ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    render_size(u32::try_from(width).ok()?, u32::try_from(height).ok()?)
}

#[cfg(test)]
#[path = "tests/worker_tests.rs"]
mod tests;
