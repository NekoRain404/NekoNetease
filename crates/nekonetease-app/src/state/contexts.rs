//! [`PlaybackContext`], the one dependency slice of [`AppState`], taken by `library::playback::*`.
//!
//! No second context exists. Elsewhere in `library` a function reading only `paths` or only `db`
//! takes that field and the rest take the whole state, and a struct between the two would only
//! rename one of them.
//!
//! `PlaybackContext` owns its `Arc`s rather than borrowing from
//! `AppState`. Each field is already `Arc`-backed under the hood, so a
//! `playback_ctx()` call costs six `Arc::clone` increments — cheap
//! enough that callers don't need to think about reuse. Owned fields
//! also dodge the "temporary doesn't live long enough" pattern when the
//! ctx is built inline in an `async move` block.

use std::sync::Arc;

use nekonetease_core::config::Paths;
use nekonetease_engine::player::engine::backend::PlaybackEngine;
use nekonetease_engine::player::engine::event_sink::PlayerSinks;
use nekonetease_engine::player::engine::state::{PlayerAction, PlayerState, PlayerStateHandle};
use nekonetease_store::database::DbPool;

use super::AppState;

/// Dependency slice consumed by `library::playback::*`. Every field is
/// `Arc`-backed; cloning the context is cheap.
#[derive(Clone)]
pub struct PlaybackContext {
    pub player_state: Arc<PlayerStateHandle>,
    pub sinks: Arc<PlayerSinks>,
    pub engine: Arc<PlaybackEngine>,
    pub db: DbPool,
    pub paths: Arc<Paths>,
}

impl PlaybackContext {
    /// Serialized mutate → emit → execute against this context's handles. Thin
    /// forwarder over [`nekonetease_engine::player::engine::actions::emit_and_execute`] so the
    /// `library::playback::*` / `library::queue::*` call sites stay one-liners
    /// while still routing through the shared execution lock.
    pub fn emit_and_execute<F>(&self, f: F)
    where
        F: FnOnce(&mut PlayerState) -> Vec<PlayerAction>,
    {
        nekonetease_engine::player::engine::actions::emit_and_execute(
            &*self.engine,
            &self.player_state,
            &self.sinks,
            f,
        );
    }
}

impl AppState {
    /// Snapshot a [`PlaybackContext`] from the live state. The shared handles are cheap to clone.
    /// Build once per UI callback rather than per
    /// `library::playback::*` invocation if you're firing several in a row.
    pub fn playback_ctx(&self) -> PlaybackContext {
        PlaybackContext {
            player_state: self.player_state.clone(),
            sinks: self.sinks.clone(),
            engine: self.engine.clone(),
            db: self.db.clone(),
            paths: self.paths.clone(),
        }
    }
}
