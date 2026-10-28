//! The rating write-back switch. Persists to `settings.json`; the in-memory shadow on
//! `AppState` is refreshed by the UI callback before the write, preserving synchronous-shadow
//! ordering.

use crate::services;
use nekonetease_core::config::Paths;
use nekonetease_core::error::AppError;

/// Persist whether a star set in NekoNetease is also written into the file's own tag.
pub fn set_write_ratings_to_tags(paths: &Paths, write: bool) -> Result<(), AppError> {
    services::settings::mutate_settings(paths, move |settings| {
        settings.library.write_ratings_to_tags = write;
    })
}
