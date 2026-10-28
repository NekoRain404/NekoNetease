//! The Motion card's one persisted setting.

use crate::services;
use nekonetease_core::config::Paths;
use nekonetease_core::error::AppError;

/// Persist the Skip Startup Animation switch.
///
/// A pure disk write: nothing applies it in the running session, since the only mount it
/// speaks for is the one `boot::ui_setup` sets up before the window is shown.
pub fn set_skip_startup_animation(paths: &Paths, on: bool) -> Result<(), AppError> {
    services::settings::mutate_settings(paths, move |settings| {
        settings.motion.skip_startup_animation = on;
    })
}
