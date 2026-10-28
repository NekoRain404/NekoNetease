//! The render thread: the only thread that issues `mpv_render_*` calls.
//!
//! It is separate from the event thread because libmpv requires every render call on one
//! thread and forbids them from the update callback: one blocked in `mpv_wait_event` could
//! not also wait on the render condvar without polling.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use nekonetease_core::error::describe;
use tokio::sync::watch;

use crate::frame::VideoFrame;
use crate::render_sw::SwRenderContext;

/// What the render thread is waiting for.
///
/// `epoch` advances whenever the surface the last frame belongs to is gone (reset) or its size
/// changes, so a render that finishes after the event thread moved on is discarded rather than
/// published over the new state.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent condition the render thread tests"
)]
#[derive(Default)]
struct RenderSignal {
    stop: bool,
    enabled: bool,
    pending: bool,
    reset: bool,
    size: Option<(u32, u32)>,
    epoch: u64,
}

#[derive(Default)]
pub(crate) struct RenderControl {
    state: Mutex<RenderSignal>,
    wake: Condvar,
}

impl RenderControl {
    fn lock(&self) -> MutexGuard<'_, RenderSignal> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&'a self, signal: MutexGuard<'a, RenderSignal>) -> MutexGuard<'a, RenderSignal> {
        self.wake.wait(signal).unwrap_or_else(PoisonError::into_inner)
    }

    /// Called from libmpv's update callback: notification only, no mpv call, no blocking.
    pub fn notify_update(&self) {
        let mut signal = self.lock();
        signal.pending = true;
        drop(signal);
        self.wake.notify_one();
    }

    pub fn enable(&self, size: (u32, u32)) {
        let mut signal = self.lock();
        if signal.size != Some(size) {
            signal.size = Some(size);
            signal.epoch = signal.epoch.wrapping_add(1);
        }
        signal.enabled = true;
        signal.pending = true;
        drop(signal);
        self.wake.notify_all();
    }

    /// Stops rendering but keeps the last frame, so a paused overlay still shows a picture.
    pub fn disable(&self) {
        let mut signal = self.lock();
        signal.enabled = false;
        signal.pending = false;
        drop(signal);
        self.wake.notify_all();
    }

    /// Drops the last frame and every render in flight.
    pub fn reset(&self) {
        let mut signal = self.lock();
        signal.enabled = false;
        signal.pending = false;
        signal.reset = true;
        signal.size = None;
        signal.epoch = signal.epoch.wrapping_add(1);
        drop(signal);
        self.wake.notify_all();
    }

    pub fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }

    fn is_current(&self, epoch: u64) -> bool {
        self.lock().epoch == epoch
    }
}

enum RenderStep {
    Stop,
    Arm,
    Disarm,
    Reset,
    Render { width: u32, height: u32, epoch: u64 },
}

impl RenderControl {
    fn next_step(&self, armed: bool) -> Option<RenderStep> {
        let mut signal = self.lock();
        loop {
            if signal.stop {
                return Some(RenderStep::Stop);
            }
            if signal.reset {
                signal.reset = false;
                return Some(RenderStep::Reset);
            }
            if !signal.enabled {
                signal.pending = false;
                if armed {
                    return Some(RenderStep::Disarm);
                }
            } else if let Some((width, height)) = signal.size {
                if !armed {
                    signal.pending = false;
                    return Some(RenderStep::Arm);
                }
                if signal.pending {
                    signal.pending = false;
                    return Some(RenderStep::Render { width, height, epoch: signal.epoch });
                }
            } else if armed {
                signal.pending = false;
                return Some(RenderStep::Disarm);
            }
            signal = self.wait(signal);
        }
    }
}

pub(super) fn spawn_renderer(
    mpv_address: usize,
    control: Arc<RenderControl>,
    frames: watch::Sender<Option<Arc<VideoFrame>>>,
) -> Option<JoinHandle<()>> {
    let result = std::thread::Builder::new()
        .name("mv-render".to_owned())
        .spawn(move || render_loop(mpv_address, control, frames));
    match result {
        Ok(handle) => Some(handle),
        Err(error) => {
            log::warn!(
                "MV render thread could not start, continuing without frames: {}",
                describe(&error)
            );
            None
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the thread owns both for its whole run; references would not outlive the spawn"
)]
fn render_loop(
    mpv_address: usize,
    control: Arc<RenderControl>,
    frames: watch::Sender<Option<Arc<VideoFrame>>>,
) {
    // SAFETY: the address is the event thread's live handle, and that thread joins this one
    // before its `Mpv` drops.
    let Some(mpv) = std::ptr::NonNull::new(std::ptr::with_exposed_provenance_mut(mpv_address))
    else {
        return;
    };
    let mut context = match SwRenderContext::create(mpv, Arc::clone(&control)) {
        Ok(context) => context,
        Err(message) => {
            log::warn!("MV software renderer could not start: {message}");
            return;
        }
    };

    let mut buffer = Vec::new();
    let mut last: Option<Arc<VideoFrame>> = None;
    let mut armed = false;
    while let Some(step) = control.next_step(armed) {
        match step {
            RenderStep::Stop => break,
            RenderStep::Reset => {
                last = None;
                buffer = Vec::new();
            }
            RenderStep::Arm => {
                context.set_update_callback(true);
                armed = true;
            }
            RenderStep::Disarm => {
                context.set_update_callback(false);
                armed = false;
            }
            RenderStep::Render { width, height, epoch } => {
                // Take the previous frame's buffer back once the UI has dropped its reference,
                // rather than allocating a fresh two megabytes every frame — at 960x540 RGBA
                // and 60 Hz that churn alone is over a hundred megabytes a second. The
                // renderer below sizes whatever it is handed, so a recycled buffer costs one
                // `resize` of the guard bytes rather than a zeroed full frame.
                if buffer.is_empty()
                    && let Some(previous) = last.take()
                {
                    match Arc::try_unwrap(previous) {
                        Ok(frame) => buffer = frame.pixels,
                        Err(still_held) => last = Some(still_held),
                    }
                }
                let ready = match context.render_if_ready(width, height, &mut buffer) {
                    Ok(ready) => ready,
                    Err(message) => {
                        log::warn!("MV frame render failed: {message}");
                        continue;
                    }
                };
                if !ready || !control.is_current(epoch) {
                    continue;
                }
                let unchanged = last.as_ref().is_some_and(|previous| {
                    previous.width == width
                        && previous.height == height
                        && previous.pixels.as_slice() == buffer.as_slice()
                });
                if unchanged {
                    continue;
                }
                let pixels = std::mem::take(&mut buffer);
                let frame = Arc::new(VideoFrame { width, height, pixels });
                last = Some(Arc::clone(&frame));
                let _ = frames.send(Some(frame));
            }
        }
    }
    context.set_update_callback(false);
}
