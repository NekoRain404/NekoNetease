//! Callback wiring. Every handler upgrades a weak window before touching a global, so a callback
//! outliving its window is inert rather than a panic on the UI thread.

use std::sync::Arc;

use slint::ComponentHandle;

use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, Social};

use super::fetch;
use super::pane;
use super::rows;
use super::{SocialUi, same_session};
use crate::ui::util::parse_cloud_id;

pub(super) fn wire(ui: &AppWindow, social: &Arc<SocialUi>) {
    let global = ui.global::<Social>();

    let covers = Arc::clone(&social.covers);
    global.on_request_cover(move |source, _generation| covers.get_or_schedule(source.as_str()));

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_close(move || {
        if let Some(ui) = weak.upgrade() {
            handle.close(&ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_retry(move || {
        if let Some(ui) = weak.upgrade() {
            handle.retry(&ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_toggle_follow(move || {
        if let Some(ui) = weak.upgrade() {
            toggle_follow(&ui, &handle);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_load_more_playlists(move || {
        if let Some(ui) = weak.upgrade() {
            fetch::playlists_more(&handle, &ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_load_more_events(move || {
        if let Some(ui) = weak.upgrade() {
            fetch::events_more(&handle, &ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_open_conversation(move |peer_id| {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(peer_id) = parse_cloud_id(peer_id.as_str()) {
            handle.open_peer(&ui, peer_id);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_load_more_conversations(move || {
        if let Some(ui) = weak.upgrade() {
            fetch::conversations_more(&handle, &ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_load_more_messages(move || {
        if let Some(ui) = weak.upgrade() {
            fetch::messages_more(&handle, &ui);
        }
    });

    // Pure, so Slint evaluates it during layout rather than inside a callback.
    global.on_messages_thread_front(pane::thread_front);

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_select_notice_tab(move |tab| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.global::<Social>().get_notice_tab() == tab {
            return;
        }
        handle.clear_notices(&ui, tab);
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_load_more_notices(move || {
        if let Some(ui) = weak.upgrade() {
            fetch::notices_more(&handle, &ui);
        }
    });

    let weak = ui.as_weak();
    let handle = Arc::clone(social);
    global.on_open_user(move |user_id| {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(user_id) = parse_cloud_id(user_id.as_str()) {
            handle.open_profile(&ui, user_id);
        }
    });
}

/// Optimistic like the comment heart: the state flips now, and a refused request puts back both
/// the flag and the count it moved. The busy flag makes a second press wait rather than stack two
/// requests whose rollbacks would race.
fn toggle_follow(ui: &AppWindow, social: &Arc<SocialUi>) {
    let global = ui.global::<Social>();
    if global.get_profile_self() || global.get_profile_busy() {
        return;
    }
    let Some(user_id) = parse_cloud_id(global.get_profile_user_id().as_str()) else {
        return;
    };
    let was_followed = global.get_profile_followed();
    let previous_count = global.get_profile_followers();
    let next = !was_followed;
    let delta = if next { 1 } else { -1 };
    global.set_profile_followed(next);
    global.set_profile_followers(rows::adjust_follow_count(previous_count.as_str(), delta).into());
    global.set_profile_busy(true);

    let state = social.state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_set_followed(user_id, next).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let global = ui.global::<Social>();
            let still_showing = global.get_page().as_str() == "profile"
                && global.get_profile_user_id().as_str() == user_id.to_string();
            if !same_session(&state, generation) || !still_showing {
                return;
            }
            global.set_profile_busy(false);
            if let Err(error) = result {
                global.set_profile_followed(was_followed);
                global.set_profile_followers(previous_count);
                toast::notify(ToastKind::OperationFailed, describe(&error));
            }
        });
    });
}
