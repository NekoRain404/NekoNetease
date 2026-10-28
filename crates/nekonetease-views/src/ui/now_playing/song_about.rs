//! The playing cloud song's wiki page, from the overflow menu's About row.
//!
//! The lookup is per click rather than per source change: most recordings are never asked about,
//! and the page is a request of its own.

use nekonetease_app::state::AppState;
use nekonetease_core::entities::song_about::SongAbout;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, Dialog, Player};
use slint::ComponentHandle;

use crate::ui::callbacks::DialogClaim;

const FACT_SEPARATOR: &str = " · ";
const PARAGRAPH_SEPARATOR: &str = "\n\n";
/// A page runs a few hundred characters; this cap keeps a long one inside a card that has no
/// scroller of its own.
const MAX_ABOUT_CHARS: usize = 800;
const ELLIPSIS: char = '…';

pub(super) fn wire(ui: &AppWindow, state: &AppState) {
    let s = state.clone();
    let weak = ui.as_weak();
    ui.global::<Player>().on_open_song_about(move || {
        let Some(ui) = weak.upgrade() else { return };
        let vm = ui.global::<Player>().get_vm();
        // A local track's `remote_id` is only its database row, so the empty path is what says
        // this source is a cloud song; the exact NetEase id rides beside it as text.
        if !vm.has_track || !vm.track.file_path.is_empty() {
            return;
        }
        let Some(id) = crate::ui::util::parse_cloud_id(vm.track.remote_id.as_str()) else {
            return;
        };
        let Some(claim) = DialogClaim::take_from(&weak) else { return };
        let title = vm.track.title.to_string();
        let state = s.clone();
        let weak = weak.clone();
        state.runtime.clone().spawn(async move {
            let result = state.cloud.song_about(id).await;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                // A menu that raised a dialog of its own while the page was in flight owns the
                // global; painting under it would hand the page another dialog's heading.
                if !claim.holds(&ui) {
                    return;
                }
                match result {
                    Ok(about) => {
                        let message = about_message(&about);
                        ui.global::<Dialog>().invoke_open_song_about(title.into(), message.into());
                    }
                    Err(error) => {
                        let detail = describe(&error);
                        log::warn!("now_playing::song_about({id}): {detail}");
                        toast::notify(ToastKind::OperationFailed, detail);
                    }
                }
            });
        });
    });
}

/// The facts line and the review are one Text in the dialog, so they are joined here rather than
/// through a second property the card has no slot for.
pub(super) fn about_message(about: &SongAbout) -> String {
    let mut message = about.facts.join(FACT_SEPARATOR);
    for review in &about.reviews {
        if !message.is_empty() {
            message.push_str(PARAGRAPH_SEPARATOR);
        }
        message.push_str(review);
    }
    capped(message)
}

fn capped(text: String) -> String {
    if text.chars().count() <= MAX_ABOUT_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_ABOUT_CHARS - 1).collect();
    cut.push(ELLIPSIS);
    cut
}

#[cfg(test)]
#[path = "tests/song_about_tests.rs"]
mod tests;
