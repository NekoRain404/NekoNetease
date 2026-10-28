//! UI zoom is multiplied by the monitor DPI rather than replacing it. Slint's runtime
//! scale factor also drives pointer coordinates and artwork resolution, so text, hit
//! targets and images stay aligned without scaling individual design tokens.

use std::rc::Rc;
use std::sync::Arc;

use slint::platform::WindowEvent;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::window::Window as WinitWindow;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::ui::callbacks::index_persist::IndexPersist;
use nekonetease_app::library;
use nekonetease_app::services::settings::{UI_SCALE_PRESETS, ui_scale_preset};
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Settings, WindowChrome};

fn percent_for_index(index: i32) -> i32 {
    usize::try_from(index).ok().and_then(|i| UI_SCALE_PRESETS.get(i)).copied().unwrap_or(100)
}

fn index_for_percent(percent: i32) -> i32 {
    let percent = ui_scale_preset(percent);
    // Six presets fit exactly in i32.
    UI_SCALE_PRESETS
        .iter()
        .position(|p| *p == percent)
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or(1)
}

pub(super) fn install(ui: &AppWindow, state: &AppState, percent: i32) {
    let settings = ui.global::<Settings>();
    settings.set_ui_scale_options(ModelRc::from(Rc::new(VecModel::from(
        UI_SCALE_PRESETS.map(|p| SharedString::from(format!("{p}%"))).to_vec(),
    ))));
    let index = index_for_percent(percent);
    settings.set_ui_scale_index(index);

    let weak = ui.as_weak();
    let state = state.clone();
    let persist = Arc::new(IndexPersist::new(index));
    settings.on_ui_scale_changed(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let percent = percent_for_index(index);
        let index = index_for_percent(percent);
        ui.global::<Settings>().set_ui_scale_index(index);
        // Apply before the next paint, after the dropdown has closed and its input dispatch
        // has unwound; resizing the renderer inside that dispatch can re-enter layout.
        ui.window().request_redraw();
        // The last pick must win even when the blocking pool runs writes out of order.
        persist.publish(index);
        let persist = Arc::clone(&persist);
        state.persist_blocking("persist ui_scale_percent", move |paths| {
            let mut result = Ok(());
            persist.write_if_current(index, || {
                result = library::settings::set_ui_scale_percent(paths, percent);
            });
            result
        });
    });
}

/// Reapply before the first paint (winit seeds the OS scale while creating the window)
/// and on monitor DPI changes. Keep the physical window size; only its layout changes.
/// `os_scale` comes directly from `ScaleFactorChanged` when winit hasn't updated its window yet.
#[allow(clippy::cast_possible_truncation, reason = "OS DPI and bounded zoom fit in f32")]
pub(crate) fn sync(ui: &AppWindow, os_scale: Option<f64>) {
    let window = ui.window();
    let Some(monitor_scale) =
        window.with_winit_window(|w| os_scale.unwrap_or_else(|| w.scale_factor()))
    else {
        return;
    };
    // Preserve Slint's explicit DPI override for development and unusual displays.
    let base = std::env::var("SLINT_SCALE_FACTOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(monitor_scale);
    let scale = (base * f64::from(percent_for_index(ui.global::<Settings>().get_ui_scale_index()))
        / 100.0) as f32;
    if (window.scale_factor() - scale).abs() < f32::EPSILON {
        return;
    }
    let Some(size) = window.with_winit_window(WinitWindow::inner_size) else { return };
    if size.width == 0 || size.height == 0 {
        return;
    }
    let result = window
        .try_dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale })
        .and_then(|()| {
            window.try_dispatch_event(WindowEvent::Resized {
                size: slint::PhysicalSize::new(size.width, size.height).to_logical(scale),
            })
        });
    if let Err(e) = result {
        log::warn!("apply UI scale: {}", describe(&e));
    }
    ui.global::<WindowChrome>().invoke_display_changed();
    window.request_redraw();
}
