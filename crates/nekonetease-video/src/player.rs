//! The public face of the crate: a cheap-to-clone owner for the two mpv threads.
//!
//! Every command is fire-and-forget. The worker owns the handle, so a caller never blocks on
//! mpv and a machine without a usable libmpv simply never starts one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use libmpv2::Mpv;
use tokio::sync::{mpsc, watch};

use crate::frame::{
    VideoError, VideoFrame, VideoState, hwdec_option, mpv_error_text, title_from_url,
};
use crate::worker::{self, Shared, WakeHandle};

/// What the UI holds for the process lifetime.
///
/// `spawn` never fails: when libmpv cannot be initialised the player reports
/// [`available`](Self::available) as `false` and every command is a no-op, so the rest of the
/// app has one code path.
pub struct VideoPlayer {
    available: bool,
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<VideoState>,
    frames: watch::Receiver<Option<Arc<VideoFrame>>>,
    progress: watch::Receiver<(u64, u64)>,
    title: Mutex<String>,
    hwdec_enabled: Arc<AtomicBool>,
    wake: Arc<WakeHandle>,
    worker: Mutex<Option<JoinHandle<()>>>,
    // Held so the receivers stay open after the worker thread ends; a dropped sender would
    // turn every `.changed().await` into an immediate `Err` and spin the bridges.
    _state_tx: watch::Sender<VideoState>,
    _frames_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    _progress_tx: watch::Sender<(u64, u64)>,
}

impl VideoPlayer {
    /// Starts the mpv threads. The returned handle is inert rather than absent when the
    /// library is unavailable, which is what keeps the caller free of a second code path.
    #[must_use]
    pub fn spawn() -> Arc<Self> {
        let (state_tx, state) = watch::channel(VideoState::Idle);
        let (frames_tx, frames) = watch::channel(None);
        let (progress_tx, progress) = watch::channel((0, 0));
        let (commands, command_rx) = mpsc::unbounded_channel();
        let hwdec_enabled = Arc::new(AtomicBool::new(true));
        let wake = Arc::new(WakeHandle::default());

        let (available, worker) = start_worker(
            hwdec_enabled.load(Ordering::Relaxed),
            command_rx,
            state_tx.clone(),
            frames_tx.clone(),
            progress_tx.clone(),
            Arc::clone(&hwdec_enabled),
            Arc::clone(&wake),
        );

        Arc::new(Self {
            available,
            commands,
            state,
            frames,
            progress,
            title: Mutex::new(String::new()),
            hwdec_enabled,
            wake,
            worker: Mutex::new(worker),
            _state_tx: state_tx,
            _frames_tx: frames_tx,
            _progress_tx: progress_tx,
        })
    }

    /// Whether libmpv initialised. The only reason a caller might branch.
    #[must_use]
    pub fn available(&self) -> bool {
        self.available
    }

    /// Loads `url`, replacing whatever was playing. `volume` is the app's 0..=1 volume; the
    /// player never reads the app's mixer afterwards, so the two cannot fight over gain.
    ///
    /// # Errors
    ///
    /// Returns [`VideoError::Closed`] once [`shutdown`](Self::shutdown) has joined the worker.
    pub fn open(&self, url: &str, volume: f64) -> Result<(), VideoError> {
        self.open_from(url, volume, 0)
    }

    /// Loads `url` at `start_ms`, which a quality switch uses to keep the position it had.
    /// The same handle replaces the file, so no second mpv instance is created.
    ///
    /// # Errors
    ///
    /// Returns [`VideoError::Closed`] once [`shutdown`](Self::shutdown) has joined the worker.
    pub fn open_from(&self, url: &str, volume: f64, start_ms: u64) -> Result<(), VideoError> {
        self.set_title(title_from_url(url));
        self.send(Command::Open { url: url.to_owned(), volume, start_ms })
    }

    /// Seeks to an absolute position. mpv clamps the target to the file's duration.
    pub fn seek_ms(&self, ms: u64) {
        let _ = self.send(Command::Seek(ms));
    }

    pub fn pause(&self) {
        let _ = self.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.send(Command::Resume);
    }

    pub fn stop(&self) {
        let _ = self.send(Command::Stop);
    }

    pub fn set_volume(&self, volume: f64) {
        let _ = self.send(Command::SetVolume(volume));
    }

    /// Switches hardware decoding at runtime, which mpv applies on the next file or frame
    /// boundary. The mirror moves before the command so a reader in the gap sees the request
    /// the user just made.
    pub fn set_hwdec(&self, enabled: bool) {
        self.hwdec_enabled.store(enabled, Ordering::Relaxed);
        let _ = self.send(Command::SetHwdec(enabled));
    }

    /// The decoding mode last requested, not what the decoder negotiated. `hwdec-current`
    /// reports that, and the worker logs the software fallback once per file.
    #[must_use]
    pub fn hwdec_enabled(&self) -> bool {
        self.hwdec_enabled.load(Ordering::Relaxed)
    }

    /// The display title, defaulting to the URL's last path segment on [`open`](Self::open).
    pub fn set_title(&self, title: impl Into<String>) {
        *self.title_lock() = title.into();
    }

    #[must_use]
    pub fn title(&self) -> String {
        self.title_lock().clone()
    }

    /// Latest rendered frame, `None` whenever no picture should be painted.
    #[must_use]
    pub fn frames(&self) -> watch::Receiver<Option<Arc<VideoFrame>>> {
        self.frames.clone()
    }

    #[must_use]
    pub fn state(&self) -> watch::Receiver<VideoState> {
        self.state.clone()
    }

    /// `(position_ms, duration_ms)`; duration is zero until mpv knows it.
    #[must_use]
    pub fn progress(&self) -> watch::Receiver<(u64, u64)> {
        self.progress.clone()
    }

    #[must_use]
    pub fn position_ms(&self) -> u64 {
        self.progress.borrow().0
    }

    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        self.progress.borrow().1
    }

    /// Asks the worker to disable callbacks and release mpv, then waits for both threads.
    /// Idempotent; the app calls it before `process::exit`, where drop would never run.
    pub fn shutdown(&self) {
        if !self.available {
            return;
        }
        let _ = self.commands.send(Command::Shutdown);
        self.wake.wake();
        let Some(thread) = self.worker_lock().take() else {
            return;
        };
        if thread.join().is_err() {
            log::warn!("MV playback thread panicked");
        }
    }

    fn send(&self, command: Command) -> Result<(), VideoError> {
        if !self.available {
            return Ok(());
        }
        self.commands.send(command).map_err(|_| VideoError::Closed)?;
        self.wake.wake();
        Ok(())
    }

    fn title_lock(&self) -> MutexGuard<'_, String> {
        self.title.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn worker_lock(&self) -> MutexGuard<'_, Option<JoinHandle<()>>> {
        self.worker.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn start_worker(
    hwdec: bool,
    commands: mpsc::UnboundedReceiver<Command>,
    state_tx: watch::Sender<VideoState>,
    frames_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    progress_tx: watch::Sender<(u64, u64)>,
    hwdec_enabled: Arc<AtomicBool>,
    wake: Arc<WakeHandle>,
) -> (bool, Option<JoinHandle<()>>) {
    let mpv = match create_mpv(hwdec) {
        Ok(mpv) => mpv,
        Err(error) => {
            log::info!("MV playback is unavailable: {}", mpv_error_text(&error));
            return (false, None);
        }
    };
    let shared = Arc::new(Shared {
        state_tx,
        frames_tx,
        progress_tx,
        position_ms: 0.into(),
        duration_ms: 0.into(),
        hwdec_enabled,
        wake,
    });
    let result = std::thread::Builder::new()
        .name("mv-player".to_owned())
        .spawn(move || worker::run(mpv, commands, shared));
    match result {
        Ok(handle) => (true, Some(handle)),
        Err(error) => {
            log::warn!(
                "MV playback thread could not start: {}",
                nekonetease_core::error::describe(&error)
            );
            (false, None)
        }
    }
}

fn create_mpv(hwdec: bool) -> libmpv2::Result<Mpv> {
    Mpv::with_initializer(|init| {
        // `vo=libmpv` is what hands frames to the software render context; the rest keep the
        // embedded player off the user's mpv configuration and scripts, and `audio-display=no`
        // stops album art being decoded as a video track behind the MV.
        init.set_option("vo", "libmpv")?;
        init.set_option("config", false)?;
        init.set_option("load-scripts", false)?;
        init.set_option("ytdl", "no")?;
        init.set_option("audio-display", "no")?;
        init.set_option("hwdec", hwdec_option(hwdec))?;
        init.set_option("hwdec-software-fallback", "yes")?;
        // The render thread is not a display: without this, mpv paces rendering to the frame's
        // presentation time and `mpv_render_context_update` reports it late.
        init.set_option("video-timing-offset", 0.0)?;
        Ok(())
    })
}

pub(crate) enum Command {
    Open { url: String, volume: f64, start_ms: u64 },
    Seek(u64),
    Pause,
    Resume,
    Stop,
    SetVolume(f64),
    SetHwdec(bool),
    Shutdown,
}

#[cfg(test)]
#[path = "tests/player_tests.rs"]
mod tests;
