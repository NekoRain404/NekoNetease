//! The social surface: a user's profile and dynamics, private conversations and notifications.
//!
//! Every list is read-only. Message bodies live in these properties and nowhere else — no store
//! call, no cache, no log line — so signing out or closing the surface drops them with the window.
//! `cloud::social` carries the application-side half of that argument.
//!
//! The request tokens are per endpoint because the three panes load independently: a profile's
//! playlists landing after the user opened somebody else must not repaint the strip, while the
//! same user's events may land a moment later and still belong on screen.

mod fetch;
mod pane;
mod rows;
mod wire;

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use nekonetease_app::state::AppState;
use nekonetease_ui::{AppWindow, Social};

use crate::ui::remote_covers::RemoteCoverThumbs;

/// The handle every callback keeps. It owns no Slint type, so holding it past a closed window is
/// harmless; the weak handle in the closure is what guards the UI writes.
pub struct SocialUi {
    state: AppState,
    covers: Arc<RemoteCoverThumbs>,
    profile_request: AtomicU64,
    events_request: AtomicU64,
    playlists_request: AtomicU64,
    conversations_request: AtomicU64,
    messages_request: AtomicU64,
    notices_request: AtomicU64,
    events_before: Mutex<Option<i64>>,
    playlists_offset: Mutex<u64>,
    conversations_offset: Mutex<u64>,
    messages_before: Mutex<Option<i64>>,
    notices_before: Mutex<Option<i64>>,
}

static INSTALLED: OnceLock<Arc<SocialUi>> = OnceLock::new();

/// The installed handle, for an entry point with no view field to hold it. Boot is the only
/// writer and runs before any callback can ask; a missing handle means the surface is not mounted.
pub fn social() -> Option<&'static Arc<SocialUi>> {
    INSTALLED.get()
}

/// Wires the surface's callbacks and returns the handle its public entry points take.
pub fn install(ui: &AppWindow, state: &AppState) -> Arc<SocialUi> {
    let weak = ui.as_weak();
    let covers = Arc::new(RemoteCoverThumbs::new_for_cards(
        state.http_client().clone(),
        state.runtime.clone(),
        move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                let social = ui.global::<Social>();
                social.set_remote_covers_generation(
                    social.get_remote_covers_generation().wrapping_add(1),
                );
            });
        },
    ));
    let social = Arc::new(SocialUi {
        state: state.clone(),
        covers,
        profile_request: AtomicU64::new(0),
        events_request: AtomicU64::new(0),
        playlists_request: AtomicU64::new(0),
        conversations_request: AtomicU64::new(0),
        messages_request: AtomicU64::new(0),
        notices_request: AtomicU64::new(0),
        events_before: Mutex::new(None),
        playlists_offset: Mutex::new(0),
        conversations_offset: Mutex::new(0),
        messages_before: Mutex::new(None),
        notices_before: Mutex::new(None),
    });
    wire::wire(ui, &social);
    let _ = INSTALLED.set(Arc::clone(&social));
    social
}

impl SocialUi {
    /// Opens one user's page over whatever the content panel was showing.
    pub fn open_profile(self: &Arc<Self>, ui: &AppWindow, user_id: i64) {
        if user_id <= 0 {
            return;
        }
        self.reset(ui);
        let social = ui.global::<Social>();
        social.set_open(true);
        social.set_page("profile".into());
        social.set_profile_user_id(user_id.to_string().into());
        social.set_profile_self(
            self.state.cloud.status().account.is_some_and(|account| account.id == user_id),
        );
        social.set_profile_loading(true);
        fetch::profile(self, ui, user_id);
        fetch::playlists_first(self, ui, user_id);
        fetch::events_first(self, ui, user_id);
    }

    /// Opens the conversation list. The messages pane stays empty until a peer is picked.
    pub fn open_messages(self: &Arc<Self>, ui: &AppWindow) {
        self.reset(ui);
        let social = ui.global::<Social>();
        social.set_open(true);
        social.set_page("messages".into());
        fetch::conversations_first(self, ui);
    }

    /// Opens the notifications surface on the reply tab.
    pub fn open_notifications(self: &Arc<Self>, ui: &AppWindow) {
        self.reset(ui);
        let social = ui.global::<Social>();
        social.set_open(true);
        social.set_page("notifications".into());
        social.set_notice_tab(0);
        fetch::notices_first(self, ui);
    }

    /// Closes the surface and drops every model, so an account's messages are not left resident
    /// behind an invisible card.
    pub fn close(self: &Arc<Self>, ui: &AppWindow) {
        self.invalidate();
        let social = ui.global::<Social>();
        social.set_open(false);
        reset_models(&social);
        reset_cursors(self);
    }

    fn reset(self: &Arc<Self>, ui: &AppWindow) {
        self.invalidate();
        reset_cursors(self);
        reset_models(&ui.global::<Social>());
    }

    fn invalidate(&self) {
        for counter in [
            &self.profile_request,
            &self.events_request,
            &self.playlists_request,
            &self.conversations_request,
            &self.messages_request,
            &self.notices_request,
        ] {
            bump(counter);
        }
    }

    fn open_peer(self: &Arc<Self>, ui: &AppWindow, peer_id: i64) {
        if peer_id <= 0 {
            return;
        }
        let social = ui.global::<Social>();
        social.set_peer_user_id(peer_id.to_string().into());
        let found = conversation_row(ui, peer_id);
        social.set_peer_name(
            found.as_ref().map_or_else(SharedString::default, |row| row.nickname.clone()),
        );
        social.set_peer_avatar_path(
            found.as_ref().map_or_else(SharedString::default, |row| row.avatar_path.clone()),
        );
        social.set_messages(empty_rows());
        social.set_messages_has_more(false);
        social.set_messages_error(SharedString::default());
        *self.messages_before.lock() = None;
        fetch::messages_first(self, ui, peer_id);
    }

    fn clear_notices(self: &Arc<Self>, ui: &AppWindow, tab: i32) {
        let social = ui.global::<Social>();
        social.set_notice_tab(tab);
        social.set_notices(empty_rows());
        social.set_notices_has_more(false);
        social.set_notices_error(SharedString::default());
        social.set_notices_more_error(SharedString::default());
        *self.notices_before.lock() = None;
        fetch::notices_first(self, ui);
    }

    fn retry(self: &Arc<Self>, ui: &AppWindow) {
        let social = ui.global::<Social>();
        match social.get_page().as_str() {
            "profile" => {
                let Ok(user_id) = social.get_profile_user_id().as_str().parse::<i64>() else {
                    return;
                };
                self.open_profile(ui, user_id);
            }
            "messages" => {
                if let Ok(peer_id) = social.get_peer_user_id().as_str().parse::<i64>() {
                    *self.messages_before.lock() = None;
                    social.set_messages(empty_rows());
                    social.set_messages_has_more(false);
                    fetch::messages_first(self, ui, peer_id);
                } else {
                    fetch::conversations_first(self, ui);
                }
            }
            "notifications" => {
                *self.notices_before.lock() = None;
                fetch::notices_first(self, ui);
            }
            _ => {}
        }
    }
}

fn conversation_row(ui: &AppWindow, peer_id: i64) -> Option<nekonetease_ui::SocialConversationRow> {
    let model = ui.global::<Social>().get_conversations();
    (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .find(|row| row.peer_id.as_str() == peer_id.to_string())
}

fn reset_models(social: &Social) {
    social.set_profile_name(SharedString::default());
    social.set_profile_signature(SharedString::default());
    social.set_profile_avatar_path(SharedString::default());
    social.set_profile_followers("0".into());
    social.set_profile_following("0".into());
    social.set_profile_playlists("0".into());
    social.set_profile_events("0".into());
    social.set_profile_followed(false);
    social.set_profile_self(false);
    social.set_profile_loading(false);
    social.set_profile_busy(false);
    social.set_profile_error(SharedString::default());
    social.set_profile_user_id(SharedString::default());

    social.set_playlists(empty_rows());
    social.set_playlists_has_more(false);
    social.set_playlists_loading(false);
    social.set_playlists_error(SharedString::default());

    social.set_events(empty_rows());
    social.set_events_has_more(false);
    social.set_events_loading(false);
    social.set_events_loading_more(false);
    social.set_events_error(SharedString::default());
    social.set_events_more_error(SharedString::default());

    social.set_conversations(empty_rows());
    social.set_conversations_has_more(false);
    social.set_conversations_loading(false);
    social.set_conversations_loading_more(false);
    social.set_conversations_error(SharedString::default());

    social.set_peer_user_id(SharedString::default());
    social.set_peer_name(SharedString::default());
    social.set_peer_avatar_path(SharedString::default());
    social.set_messages(empty_rows());
    social.set_messages_has_more(false);
    social.set_messages_loading(false);
    social.set_messages_loading_more(false);
    social.set_messages_error(SharedString::default());

    social.set_notices(empty_rows());
    social.set_notices_has_more(false);
    social.set_notices_loading(false);
    social.set_notices_loading_more(false);
    social.set_notices_error(SharedString::default());
    social.set_notices_more_error(SharedString::default());
}

fn reset_cursors(social: &SocialUi) {
    *social.events_before.lock() = None;
    *social.playlists_offset.lock() = 0;
    *social.conversations_offset.lock() = 0;
    *social.messages_before.lock() = None;
    *social.notices_before.lock() = None;
}

/// The token a request carries. A later open or close bumps it, which is what keeps a page from
/// the previous user off the surface now on screen.
pub(super) fn next(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) fn is_current(counter: &AtomicU64, token: u64) -> bool {
    counter.load(Ordering::Relaxed) == token
}

pub(super) fn bump(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) fn same_session(state: &AppState, generation: u64) -> bool {
    state.cloud.status().generation == generation
}

pub(super) fn page_is(ui: &AppWindow, page: &str) -> bool {
    let social = ui.global::<Social>();
    social.get_open() && social.get_page().as_str() == page
}

pub(super) fn empty_rows<T: Clone + Default + 'static>() -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::default()))
}

pub(super) fn replace_rows<T: Clone + Default + 'static>(model: &ModelRc<T>, rows: Vec<T>) {
    if let Some(vec) = model.as_any().downcast_ref::<VecModel<T>>() {
        vec.set_vec(rows);
    }
}

pub(super) fn rows_of<T: Clone + Default + 'static>(model: &ModelRc<T>) -> Vec<T> {
    (0..model.row_count()).filter_map(|index| model.row_data(index)).collect()
}

/// Appends what the surface does not hold yet. The service repeats an entry across adjacent pages
/// often enough that a plain append doubles rows, and a page made only of repeats must not keep
/// the paging footer alive.
pub(super) fn append_unique<T: Clone>(
    existing: &mut Vec<T>,
    incoming: Vec<T>,
    id: impl Fn(&T) -> String,
) -> usize {
    let mut seen: HashSet<String> = existing.iter().map(&id).collect();
    let before = existing.len();
    existing.extend(incoming.into_iter().filter(|row| seen.insert(id(row))));
    existing.len() - before
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/page_failure_tests.rs"]
mod page_failure_tests;
