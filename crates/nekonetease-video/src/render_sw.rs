//! The software render context.
//!
//! `libmpv2`'s safe `render` module speaks OpenGL only, so the `MPV_RENDER_API_TYPE_SW` calls
//! are its own generated bindings used directly. Everything here runs on the render thread:
//! libmpv requires one thread to create, update, render and free a context, and forbids the
//! update callback from calling any of it.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Arc;

use libmpv2_sys as sys;

use crate::frame::VideoError;
use crate::worker::RenderControl;

/// How many bytes each pixel occupies; the format asked of mpv below must agree.
const BYTES_PER_PIXEL: usize = 4;
/// A scaler may write past the last row; the scratch buffer carries this much slack.
const GUARD: usize = 64;

/// `rgba` is not on libmpv's documented short list, but it is the internal RGB single-plane
/// name it accepts, and it is the only 8-bit format this build renders alpha correctly in.
/// (`rgb24` and `rgba64` are rejected outright by the same build.)
const PIXEL_FORMAT: &[u8] = b"rgba\0";

pub(crate) struct SwRenderContext {
    context: NonNull<sys::mpv_render_context>,
    control: Arc<RenderControl>,
    callback_armed: bool,
}

impl SwRenderContext {
    #[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
    pub(crate) fn create(
        mpv: NonNull<sys::mpv_handle>,
        control: Arc<RenderControl>,
    ) -> Result<Self, VideoError> {
        let api_type = sys::MPV_RENDER_API_TYPE_SW.as_ptr().cast_mut().cast::<c_void>();
        let mut params = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api_type,
            },
            sys::mpv_render_param { type_: 0, data: ptr::null_mut() },
        ];
        let mut raw: *mut sys::mpv_render_context = ptr::null_mut();
        // SAFETY: `mpv` is the live handle the event thread owns for this call; `params` is a
        // NUL-terminated array living on this frame, and libmpv copies what it needs during
        // the call. The returned context is owned by the `NonNull` below from here on.
        let code = unsafe {
            sys::mpv_render_context_create(
                ptr::from_mut(&mut raw),
                mpv.as_ptr(),
                params.as_mut_ptr(),
            )
        };
        if code < 0 {
            return Err(VideoError::Render(error_text(code)));
        }
        let context = NonNull::new(raw)
            .ok_or_else(|| VideoError::Render("libmpv returned no render context".to_owned()))?;
        Ok(Self { context, control, callback_armed: false })
    }

    /// Installs or removes the frame-arrival callback. mpv raises it once immediately on
    /// install, which is what arms the first render without polling.
    #[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
    pub(crate) fn set_update_callback(&mut self, armed: bool) {
        if self.callback_armed == armed {
            return;
        }
        let control = if armed {
            Arc::as_ptr(&self.control).cast_mut().cast::<c_void>()
        } else {
            ptr::null_mut()
        };
        // SAFETY: while armed, `self.control` keeps the `RenderControl` alive, and the
        // callback is disabled before `Drop` frees the context. The callback touches only
        // that control, never mpv.
        unsafe {
            sys::mpv_render_context_set_update_callback(
                self.context.as_ptr(),
                if armed { Some(update_callback) } else { None },
                control,
            );
        }
        self.callback_armed = armed;
    }

    /// Asks mpv whether a new frame is ready, and renders it if so.
    ///
    /// Returns `false` when the wake was for something other than a frame; the caller then
    /// waits for the next callback instead of rendering the same picture again.
    #[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
    pub(crate) fn render_if_ready(
        &mut self,
        width: u32,
        height: u32,
        buffer: &mut Vec<u8>,
    ) -> Result<bool, VideoError> {
        // SAFETY: called on the render thread that created the context, and not from the
        // update callback.
        let flags = unsafe { sys::mpv_render_context_update(self.context.as_ptr()) };
        if flags & u64::from(sys::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME) == 0 {
            return Ok(false);
        }
        self.render(width, height, buffer)?;
        Ok(true)
    }

    #[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
    fn render(&self, width: u32, height: u32, buffer: &mut Vec<u8>) -> Result<(), VideoError> {
        let stride = usize::try_from(width)
            .ok()
            .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
            .ok_or_else(|| format!("video frame width {width} does not fit memory"))?;
        let total = stride
            .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
            .ok_or_else(|| format!("video frame {width}x{height} does not fit memory"))?;
        let capacity = total
            .checked_add(GUARD)
            .ok_or_else(|| format!("video frame {width}x{height} does not fit memory"))?;
        if buffer.len() != capacity {
            // No `clear()` first: growing a `Vec` only initialises the bytes it adds, so a
            // recycled buffer zeroes the guard tail and nothing else. Clearing first rewrote
            // every pixel of a frame the renderer is about to overwrite anyway.
            buffer.resize(capacity, 0);
        }

        let mut size = [
            i32::try_from(width).map_err(|_| format!("video frame width {width} is too large"))?,
            i32::try_from(height)
                .map_err(|_| format!("video frame height {height} is too large"))?,
        ];
        let mut stride = stride;
        let mut params = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr().cast::<c_void>(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                data: PIXEL_FORMAT.as_ptr().cast_mut().cast::<c_void>(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                data: ptr::from_mut(&mut stride).cast::<c_void>(),
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                // The parameter's payload is the surface pointer itself, not its address.
                data: buffer.as_mut_ptr().cast::<c_void>(),
            },
            sys::mpv_render_param { type_: 0, data: ptr::null_mut() },
        ];
        // SAFETY: the buffer holds `stride * height` writable bytes plus the guard, stays
        // borrowed for the whole call, and mpv retains none of these pointers.
        let code =
            unsafe { sys::mpv_render_context_render(self.context.as_ptr(), params.as_mut_ptr()) };
        if code < 0 {
            return Err(VideoError::Render(error_text(code)));
        }
        buffer.truncate(total);
        Ok(())
    }
}

impl Drop for SwRenderContext {
    #[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
    fn drop(&mut self) {
        if self.callback_armed {
            // SAFETY: the same thread that installed the callback; disabling it first means
            // no callback can touch `self.control` once freeing starts.
            unsafe {
                sys::mpv_render_context_set_update_callback(
                    self.context.as_ptr(),
                    None,
                    ptr::null_mut(),
                );
            }
        }
        // SAFETY: this is the single owner of the context, on the thread that created it.
        // `self.control` is still alive here, and drops only after this returns.
        unsafe { sys::mpv_render_context_free(self.context.as_ptr()) };
    }
}

#[allow(unsafe_code, reason = "libmpv2 has no safe binding for the software render context")]
unsafe extern "C" fn update_callback(userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    // SAFETY: installed only while `SwRenderContext::control` holds the `Arc` this pointer
    // came from, and only the callback path reads it. It never calls back into mpv.
    let control = unsafe { &*userdata.cast::<RenderControl>() };
    control.notify_update();
}

fn error_text(code: std::os::raw::c_int) -> String {
    libmpv2_sys::mpv_error_str(code).to_owned()
}
