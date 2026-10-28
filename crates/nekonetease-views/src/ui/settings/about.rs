//! Wire the Settings page's About section.
//!
//! Registers the contributor links shown in Settings → About. Each launch goes through
//! [`crate::ui::launcher::open_target`], which owns the hop off the UI thread.
//!
//! The version shown in the About card is *not* set here — it rides on
//! `NekoNeteaseUpdater.current-version`, seeded by [`super::updater_settings`].

use slint::ComponentHandle;

use crate::ui::launcher;
use nekonetease_app::state::AppState;
use nekonetease_ui::{AppWindow, Settings};

const ADAPTER_AUTHOR_URL: &str = "https://github.com/NekoRain";
const FRAMEWORK_AUTHOR_URL: &str = "https://github.com/KenanSalar";

pub fn install(ui: &AppWindow, state: &AppState) {
    let runtime = state.runtime.clone();
    ui.global::<Settings>().on_open_adapter_author({
        let runtime = runtime.clone();
        move || {
            runtime.spawn(launcher::open_target(ADAPTER_AUTHOR_URL, "open-adapter-author"));
        }
    });
    ui.global::<Settings>().on_open_framework_author(move || {
        runtime.spawn(launcher::open_target(FRAMEWORK_AUTHOR_URL, "open-framework-author"));
    });
}
