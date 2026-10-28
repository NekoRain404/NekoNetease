//! The playing cloud song's other names, for the overflow menu's version line.
//!
//! What a local file says about its version is already in its title, but a cloud song's aliases
//! arrive only with its detail row, so this fetches one per source change and hands the joined
//! text to `Player.version-line`.

use async_compat::Compat;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::PlayerViewModelLight;
use nekonetease_ui::{AppWindow, Player};
use slint::ComponentHandle;

pub(super) fn spawn_subscriber(
    ui: &AppWindow,
    state: &AppState,
) -> Result<(), slint::EventLoopError> {
    let weak = ui.as_weak();
    let state = state.clone();
    let mut rx = state.sinks.view_model.subscribe();
    slint::spawn_local(Compat::new(async move {
        // Only the id decides when a fetch is owed: every other push (play, pause, volume) moves
        // the view model without changing the answer.
        let mut last_cloud_id: Option<i64> = None;
        loop {
            let cloud_id = rx.borrow_and_update().as_ref().and_then(cloud_id_of);
            if cloud_id != last_cloud_id {
                last_cloud_id = cloud_id;
                let version = match cloud_id {
                    Some(id) => fetch_version(&state, id).await,
                    None => String::new(),
                };
                // A skip during the lookup retires the answer rather than painting it under the
                // next song; the pass this wakes fetches the new id.
                if rx.borrow().as_ref().and_then(cloud_id_of) == cloud_id
                    && let Some(ui) = weak.upgrade()
                {
                    ui.global::<Player>().set_version_line(version.into());
                }
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
        log::debug!("ui::now_playing version subscriber stopped");
    }))?;
    Ok(())
}

/// A cloud stream is the only kind with a song row to look up: a local file's version, if any, is
/// already in its title, and a station has no song row at all.
fn cloud_id_of(vm: &PlayerViewModelLight) -> Option<i64> {
    vm.current_track.as_ref().filter(|track| track.stream.is_some()).map(|track| track.id)
}

async fn fetch_version(state: &AppState, id: i64) -> String {
    match state.cloud.songs(&[id]).await {
        Ok(songs) => songs.first().map_or_else(String::new, |song| {
            version_text(&song.name, &song.alias, &song.translated_names)
        }),
        // The line is decoration on a menu: a failed lookup leaves it empty rather than raising an
        // error over a song that is playing fine.
        Err(error) => {
            log::warn!("now_playing::version({id}): {}", describe(&error));
            String::new()
        }
    }
}

const SEPARATOR: &str = " · ";
/// A menu row is one fixed-height line in a popup measured from its widest row, so the joined list
/// is capped: past this the menu would be sized by an alias rather than by its own controls.
const MAX_VERSION_CHARS: usize = 64;

/// Both naming fields, deduped and stripped of the title the row above already states.
fn version_text(title: &str, aliases: &[String], translated_names: &[String]) -> String {
    let title = title.trim();
    let mut line = String::new();
    let mut accepted: Vec<&str> = Vec::new();
    let mut omitted = false;
    for name in aliases.iter().chain(translated_names) {
        let name = name.trim();
        if name.is_empty() || name == title || accepted.contains(&name) {
            continue;
        }
        let separator = if line.is_empty() { 0 } else { SEPARATOR.chars().count() };
        if line.chars().count() + separator + name.chars().count() > MAX_VERSION_CHARS {
            // A lone alias over the budget is cut rather than dropped, so the line still names it.
            if line.is_empty() {
                line.extend(name.chars().take(MAX_VERSION_CHARS));
            }
            omitted = true;
            break;
        }
        if !line.is_empty() {
            line.push_str(SEPARATOR);
        }
        line.push_str(name);
        accepted.push(name);
    }
    if omitted {
        if !line.is_empty() {
            line.push_str(SEPARATOR);
        }
        line.push('…');
    }
    line
}

#[cfg(test)]
#[path = "tests/version_tests.rs"]
mod tests;
