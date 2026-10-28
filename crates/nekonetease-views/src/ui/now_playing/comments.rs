//! The player bar's comment entry: the current song's whole thread, from the transport row.

use slint::ComponentHandle;

use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::CommentThread;
use nekonetease_ui::{AppWindow, Nav, Player};

use crate::ui::comments;

pub(super) fn wire(ui: &AppWindow, state: &AppState) {
    let s = state.clone();
    let weak = ui.as_weak();
    ui.global::<Player>().on_open_comments(move || {
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
        // The thread mounts only while Now Playing is closed, so the button pressed over the
        // full-screen view would otherwise hand the user a page they cannot see.
        ui.global::<Nav>().set_now_playing_open(false);
        comments::open(&ui, &s, CommentThread::Song(id), vm.track.title.as_str());
    });
}
