//! The `Comments` global's callbacks. Every one reads the thread from the global rather than
//! carrying it, so a stale closure cannot act on a surface that has moved.

use std::sync::Arc;

use slint::{ComponentHandle, SharedString};

use nekonetease_app::state::AppState;
use nekonetease_ui::{AppWindow, Comments};

use super::{close, close_replies, load};
use crate::ui::remote_covers::RemoteCoverThumbs;

pub(super) fn wire(ui: &AppWindow, state: &AppState, covers: Arc<RemoteCoverThumbs>) {
    let comments = ui.global::<Comments>();

    comments.on_request_cover(move |source, _generation| covers.get_or_schedule(source.as_str()));

    let weak = ui.as_weak();
    comments.on_close(move || {
        if let Some(ui) = weak.upgrade() {
            close(&ui);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_reload(move || {
        if let Some(ui) = weak.upgrade() {
            load::first(&ui, &s);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_load_more(move || {
        if let Some(ui) = weak.upgrade() {
            load::more(&ui, &s);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_select_hot(move || {
        if let Some(ui) = weak.upgrade() {
            load::pick_sort(&ui, &s, true);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_select_time(move || {
        if let Some(ui) = weak.upgrade() {
            load::pick_sort(&ui, &s, false);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_toggle_like(move |index, remote_id, next| {
        if let Some(ui) = weak.upgrade() {
            load::toggle_like(&ui, &s, load::Floor::Thread, index, remote_id.as_str(), next);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_toggle_reply_like(move |index, remote_id, next| {
        if let Some(ui) = weak.upgrade() {
            load::toggle_like(&ui, &s, load::Floor::Replies, index, remote_id.as_str(), next);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_delete_comment(move |remote_id| {
        if let Some(ui) = weak.upgrade() {
            load::delete(&ui, &s, remote_id.as_str());
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_open_replies(move |remote_id| {
        let Some(parent_id) = crate::ui::util::parse_cloud_id(remote_id.as_str()) else { return };
        if let Some(ui) = weak.upgrade() {
            load::replies(&ui, &s, parent_id);
        }
    });

    let weak = ui.as_weak();
    comments.on_close_replies(move || {
        if let Some(ui) = weak.upgrade() {
            close_replies(&ui);
        }
    });

    let weak = ui.as_weak();
    comments.on_set_reply_target(move |remote_id, author| {
        if let Some(ui) = weak.upgrade() {
            let comments = ui.global::<Comments>();
            comments.set_reply_to_id(remote_id);
            comments.set_reply_to_author(author);
            comments.set_composer_focus_tick(comments.get_composer_focus_tick().wrapping_add(1));
        }
    });

    // Slint strings expose no length, so the composer's counter is answered here on every
    // keystroke; the same character count the Rust gate applies.
    let weak = ui.as_weak();
    comments.on_draft_edited(move || {
        if let Some(ui) = weak.upgrade() {
            let comments = ui.global::<Comments>();
            let count = i32::try_from(comments.get_draft().chars().count()).unwrap_or(i32::MAX);
            comments.set_draft_count(count);
        }
    });

    let weak = ui.as_weak();
    comments.on_cancel_reply(move || {
        if let Some(ui) = weak.upgrade() {
            let comments = ui.global::<Comments>();
            comments.set_reply_to_id(SharedString::default());
            comments.set_reply_to_author(SharedString::default());
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    comments.on_post(move || {
        if let Some(ui) = weak.upgrade() {
            load::post(&ui, &s);
        }
    });
}
