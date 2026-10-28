//! Whether Windows draws light or dark, under Settings ▸ Personalization ▸ Colors. The default app
//! mode is the reading the selected theme's System variant follows.
//!
//! **Fails light**: a value that won't read answers light, the mode Windows applies to apps until
//! the user picks another.

use super::registry::read_user_dword;

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

/// The default app mode, as `"light"` or `"dark"`, the vocabulary `SystemColorState::theme` is
/// written in.
pub fn app_theme() -> &'static str {
    theme_for(read_user_dword(PERSONALIZE_KEY, "AppsUseLightTheme"))
}

/// The decision, apart from the registry: dark only while the value says so outright.
fn theme_for(uses_light_theme: Option<u32>) -> &'static str {
    if uses_light_theme == Some(0) { "dark" } else { "light" }
}

#[cfg(test)]
#[path = "tests/color_mode_tests.rs"]
mod tests;
