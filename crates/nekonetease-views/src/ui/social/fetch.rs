//! The network half of the surface.
//!
//! Each completion re-checks its own token, the account generation and the page still on screen,
//! so a page landing after a close, a sign-out or a jump to another user cannot repaint what the
//! user is now looking at. Nothing here logs: a failure is reported to the pane, never described
//! into the rolling log beside the message text it was about.

use std::sync::Arc;

use nekonetease_core::error::describe;
use nekonetease_ui::AppWindow;
use slint::{ComponentHandle, SharedString};

use super::rows;
use super::{
    SocialUi, append_unique, empty_rows, is_current, next, page_is, replace_rows, rows_of,
    same_session,
};

const EVENT_PAGE: u32 = 20;
const PLAYLIST_PAGE: u32 = 30;
const CONVERSATION_PAGE: u32 = 30;
const MESSAGE_PAGE: u32 = 50;
const NOTICE_PAGE: u32 = 30;

pub(super) fn profile(social: &Arc<SocialUi>, ui: &AppWindow, user_id: i64) {
    let token = next(&social.profile_request);
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_profile(user_id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.profile_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "profile")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            social_ui.set_profile_loading(false);
            match result {
                Ok(profile) => {
                    social_ui.set_profile_name(profile.nickname.into());
                    social_ui.set_profile_signature(profile.signature.into());
                    social_ui.set_profile_avatar_path(rows::remote_artwork(
                        profile.avatar_url.as_deref(),
                    ));
                    social_ui.set_profile_followers(profile.followers.to_string().into());
                    social_ui.set_profile_following(profile.follows.to_string().into());
                    social_ui.set_profile_playlists(profile.playlist_count.to_string().into());
                    social_ui.set_profile_events(profile.event_count.to_string().into());
                    social_ui.set_profile_followed(profile.followed);
                    social_ui.set_profile_error(SharedString::default());
                }
                Err(error) => social_ui.set_profile_error(describe(&error).into()),
            }
        });
    });
}

pub(super) fn playlists_first(social: &Arc<SocialUi>, ui: &AppWindow, user_id: i64) {
    let token = next(&social.playlists_request);
    apply_playlists(social, ui, user_id, 0, token, false);
}

pub(super) fn playlists_more(social: &Arc<SocialUi>, ui: &AppWindow) {
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if !social_ui.get_playlists_has_more() || social_ui.get_playlists_loading() {
        return;
    }
    let Ok(user_id) = social_ui.get_profile_user_id().as_str().parse::<i64>() else {
        return;
    };
    let offset = *social.playlists_offset.lock();
    let token = next(&social.playlists_request);
    apply_playlists(social, ui, user_id, offset, token, true);
}

fn apply_playlists(
    social: &Arc<SocialUi>,
    ui: &AppWindow,
    user_id: i64,
    offset: u64,
    token: u64,
    more: bool,
) {
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    let social_ui = ui.global::<nekonetease_ui::Social>();
    social_ui.set_playlists_loading(true);
    social_ui.set_playlists_error(SharedString::default());
    if !more {
        social_ui.set_playlists(empty_rows());
        social_ui.set_playlists_has_more(false);
    }
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_playlists(user_id, offset, PLAYLIST_PAGE).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.playlists_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "profile")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            social_ui.set_playlists_loading(false);
            match result {
                Ok(page) => {
                    let incoming: Vec<_> = page.playlists.iter().map(rows::playlist_row).collect();
                    let mut existing =
                        if more { rows_of(&social_ui.get_playlists()) } else { Vec::new() };
                    let returned = incoming.len();
                    let added = append_unique(&mut existing, incoming, |row| row.id.to_string());
                    replace_rows(&social_ui.get_playlists(), existing);
                    *social.playlists_offset.lock() = offset + u64::try_from(returned).unwrap_or(0);
                    social_ui.set_playlists_has_more(page.has_more && added > 0);
                }
                Err(error) => social_ui.set_playlists_error(describe(&error).into()),
            }
        });
    });
}

pub(super) fn events_first(social: &Arc<SocialUi>, ui: &AppWindow, user_id: i64) {
    let token = next(&social.events_request);
    apply_events(social, ui, user_id, None, token, false);
}

pub(super) fn events_more(social: &Arc<SocialUi>, ui: &AppWindow) {
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if !social_ui.get_events_has_more() || social_ui.get_events_loading_more() {
        return;
    }
    let Ok(user_id) = social_ui.get_profile_user_id().as_str().parse::<i64>() else {
        return;
    };
    let Some(before) = *social.events_before.lock() else {
        return;
    };
    let token = next(&social.events_request);
    apply_events(social, ui, user_id, Some(before), token, true);
}

fn apply_events(
    social: &Arc<SocialUi>,
    ui: &AppWindow,
    user_id: i64,
    before: Option<i64>,
    token: u64,
    more: bool,
) {
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if more {
        social_ui.set_events_loading_more(true);
    } else {
        social_ui.set_events_loading(true);
        social_ui.set_events(empty_rows());
        social_ui.set_events_has_more(false);
        social_ui.set_events_error(SharedString::default());
        social_ui.set_events_more_error(SharedString::default());
    }
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_events(user_id, before, EVENT_PAGE).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.events_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "profile")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            social_ui.set_events_loading(false);
            social_ui.set_events_loading_more(false);
            match result {
                Ok(page) => {
                    let incoming: Vec<_> = page.events.iter().map(rows::event_row).collect();
                    let mut existing =
                        if more { rows_of(&social_ui.get_events()) } else { Vec::new() };
                    let added = append_unique(&mut existing, incoming, |row| row.id.to_string());
                    replace_rows(&social_ui.get_events(), existing);
                    *social.events_before.lock() = page.next_before;
                    social_ui.set_events_has_more(page.next_before.is_some() && added > 0);
                    social_ui.set_events_more_error(SharedString::default());
                }
                Err(error) => {
                    let message: SharedString = describe(&error).into();
                    if more {
                        social_ui.set_events_more_error(message);
                    } else {
                        social_ui.set_events_error(message);
                    }
                }
            }
        });
    });
}

pub(super) fn conversations_first(social: &Arc<SocialUi>, ui: &AppWindow) {
    let token = next(&social.conversations_request);
    apply_conversations(social, ui, 0, token, false);
}

pub(super) fn conversations_more(social: &Arc<SocialUi>, ui: &AppWindow) {
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if !social_ui.get_conversations_has_more() || social_ui.get_conversations_loading_more() {
        return;
    }
    let offset = *social.conversations_offset.lock();
    let token = next(&social.conversations_request);
    apply_conversations(social, ui, offset, token, true);
}

fn apply_conversations(
    social: &Arc<SocialUi>,
    ui: &AppWindow,
    offset: u64,
    token: u64,
    more: bool,
) {
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if more {
        social_ui.set_conversations_loading_more(true);
    } else {
        social_ui.set_conversations_loading(true);
        social_ui.set_conversations(empty_rows());
        social_ui.set_conversations_has_more(false);
    }
    social_ui.set_conversations_error(SharedString::default());
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_conversations(offset, CONVERSATION_PAGE).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.conversations_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "messages")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            social_ui.set_conversations_loading(false);
            social_ui.set_conversations_loading_more(false);
            match result {
                Ok(page) => {
                    let incoming: Vec<_> =
                        page.conversations.iter().map(rows::conversation_row).collect();
                    let returned = incoming.len();
                    let mut existing =
                        if more { rows_of(&social_ui.get_conversations()) } else { Vec::new() };
                    let added =
                        append_unique(&mut existing, incoming, |row| row.peer_id.to_string());
                    replace_rows(&social_ui.get_conversations(), existing);
                    *social.conversations_offset.lock() =
                        offset + u64::try_from(returned).unwrap_or(0);
                    social_ui.set_conversations_has_more(page.has_more && added > 0);
                }
                Err(error) => social_ui.set_conversations_error(describe(&error).into()),
            }
        });
    });
}

pub(super) fn messages_first(social: &Arc<SocialUi>, ui: &AppWindow, peer_id: i64) {
    let token = next(&social.messages_request);
    apply_messages(social, ui, peer_id, None, token);
}

pub(super) fn messages_more(social: &Arc<SocialUi>, ui: &AppWindow) {
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if !social_ui.get_messages_has_more() || social_ui.get_messages_loading_more() {
        return;
    }
    let Ok(peer_id) = social_ui.get_peer_user_id().as_str().parse::<i64>() else {
        return;
    };
    let Some(before) = *social.messages_before.lock() else {
        return;
    };
    let token = next(&social.messages_request);
    apply_messages(social, ui, peer_id, Some(before), token);
}

fn apply_messages(
    social: &Arc<SocialUi>,
    ui: &AppWindow,
    peer_id: i64,
    before: Option<i64>,
    token: u64,
) {
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    let social_ui = ui.global::<nekonetease_ui::Social>();
    let more = before.is_some();
    if more {
        social_ui.set_messages_loading_more(true);
    } else {
        social_ui.set_messages_loading(true);
        social_ui.set_messages(empty_rows());
        social_ui.set_messages_has_more(false);
    }
    social_ui.set_messages_error(SharedString::default());
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.social_messages(peer_id, before, MESSAGE_PAGE).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.messages_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "messages")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            if social_ui.get_peer_user_id().as_str() != peer_id.to_string() {
                return;
            }
            social_ui.set_messages_loading(false);
            social_ui.set_messages_loading_more(false);
            match result {
                Ok(page) => {
                    let incoming: Vec<_> = page.messages.iter().map(rows::message_row).collect();
                    let mut existing =
                        if more { rows_of(&social_ui.get_messages()) } else { Vec::new() };
                    let added = append_unique(&mut existing, incoming, |row| row.id.to_string());
                    replace_rows(&social_ui.get_messages(), existing);
                    let cursor = page
                        .messages
                        .last()
                        .map(|message| message.time_ms)
                        .filter(|time| *time > 0)
                        .filter(|time| before.is_none_or(|previous| *time != previous));
                    *social.messages_before.lock() = cursor;
                    social_ui.set_messages_has_more(page.has_more && cursor.is_some() && added > 0);
                }
                Err(error) => social_ui.set_messages_error(describe(&error).into()),
            }
        });
    });
}

pub(super) fn notices_first(social: &Arc<SocialUi>, ui: &AppWindow) {
    let token = next(&social.notices_request);
    apply_notices(social, ui, None, token);
}

pub(super) fn notices_more(social: &Arc<SocialUi>, ui: &AppWindow) {
    let social_ui = ui.global::<nekonetease_ui::Social>();
    if !social_ui.get_notices_has_more() || social_ui.get_notices_loading_more() {
        return;
    }
    let Some(before) = *social.notices_before.lock() else {
        return;
    };
    let token = next(&social.notices_request);
    apply_notices(social, ui, Some(before), token);
}

fn apply_notices(social: &Arc<SocialUi>, ui: &AppWindow, before: Option<i64>, token: u64) {
    let state = social.state.clone();
    let social = Arc::clone(social);
    let weak = ui.as_weak();
    let social_ui = ui.global::<nekonetease_ui::Social>();
    let more = before.is_some();
    let replies = social_ui.get_notice_tab() == 0;
    if more {
        social_ui.set_notices_loading_more(true);
    } else {
        social_ui.set_notices_loading(true);
        social_ui.set_notices(empty_rows());
        social_ui.set_notices_has_more(false);
        social_ui.set_notices_error(SharedString::default());
        social_ui.set_notices_more_error(SharedString::default());
    }
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = if replies {
            state.cloud.social_comment_notifications(before, NOTICE_PAGE).await
        } else {
            state.cloud.social_notices(before, NOTICE_PAGE).await
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(&social.notices_request, token)
                || !same_session(&state, generation)
                || !page_is(&ui, "notifications")
            {
                return;
            }
            let social_ui = ui.global::<nekonetease_ui::Social>();
            if (social_ui.get_notice_tab() == 0) != replies {
                return;
            }
            social_ui.set_notices_loading(false);
            social_ui.set_notices_loading_more(false);
            match result {
                Ok(page) => {
                    let incoming: Vec<_> = page.notices.iter().map(rows::notice_row).collect();
                    let mut existing =
                        if more { rows_of(&social_ui.get_notices()) } else { Vec::new() };
                    let added = append_unique(&mut existing, incoming, |row| row.id.to_string());
                    replace_rows(&social_ui.get_notices(), existing);
                    let cursor = page
                        .notices
                        .last()
                        .map(|notice| notice.time_ms)
                        .filter(|time| *time > 0)
                        .filter(|time| before.is_none_or(|previous| *time != previous));
                    *social.notices_before.lock() = cursor;
                    social_ui.set_notices_has_more(page.has_more && cursor.is_some() && added > 0);
                    social_ui.set_notices_more_error(SharedString::default());
                }
                Err(error) => {
                    let message: SharedString = describe(&error).into();
                    if more {
                        social_ui.set_notices_more_error(message);
                    } else {
                        social_ui.set_notices_error(message);
                    }
                }
            }
        });
    });
}
