//! The full comment thread surface: paging, ordering, replies and posting.
//!
//! `detail_comments` keeps the capped preview the playlist and album details share. This
//! module is the surface behind the `Comments` global, and it owns the request tokens that
//! keep a page landing after a sort switch or a close out of the thread now on screen.

mod callbacks;
mod like;
#[cfg(test)]
#[path = "tests/like_tests.rs"]
mod like_tests;
mod load;
mod models;
#[cfg(test)]
#[path = "tests/models_tests.rs"]
mod models_tests;
#[cfg(test)]
#[path = "tests/page_failure_tests.rs"]
mod page_failure_tests;
mod paging;
#[cfg(test)]
#[path = "tests/paging_tests.rs"]
mod paging_tests;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::CommentThread;
use nekonetease_ui::{AppWindow, CommentRow, Comments};

use crate::ui::remote_covers::RemoteCoverThumbs;

static REQUEST: AtomicU64 = AtomicU64::new(0);
static REPLIES_REQUEST: AtomicU64 = AtomicU64::new(0);

/// Wires the surface's callbacks. Safe to call once per window; repeated calls replace the
/// handlers rather than stacking them.
pub fn install(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    // The row tier, not the card one: an avatar draws at 28 px, and `new`'s 72 px covers it at
    // every scale the UI offers without holding a card-sized bitmap per commenter.
    let covers = Arc::new(RemoteCoverThumbs::new(
        state.http_client().clone(),
        state.runtime.clone(),
        move || {
            let _ = weak.upgrade_in_event_loop(|ui| {
                let comments = ui.global::<Comments>();
                comments.set_remote_covers_generation(
                    comments.get_remote_covers_generation().wrapping_add(1),
                );
            });
        },
    ));
    callbacks::wire(ui, state, covers);
}

/// Opens the thread over whatever page was there. The caller sets the token it owns.
pub fn open(ui: &AppWindow, state: &AppState, thread: CommentThread, title: &str) {
    let comments = ui.global::<Comments>();
    reset(&ui.global::<Comments>());
    comments.set_open(true);
    comments.set_title(SharedString::from(title));
    comments.set_thread_kind(thread_kind(thread).into());
    comments.set_thread_id(thread.id().to_string().into());
    comments.set_hot(true);
    next_request();
    load::first(ui, state);
}

/// Closes the surface and drops every model, so a reopen starts from the first page.
pub fn close(ui: &AppWindow) {
    next_request();
    next_replies_request();
    let comments = ui.global::<Comments>();
    comments.set_open(false);
    comments.set_title(SharedString::default());
    comments.set_thread_kind(SharedString::default());
    comments.set_thread_id(SharedString::default());
    comments.set_draft(SharedString::default());
    comments.set_draft_count(0);
    comments.set_reply_to_id(SharedString::default());
    comments.set_reply_to_author(SharedString::default());
    comments.set_posting(false);
    reset(&comments);
    close_replies(ui);
}

/// Dismisses the reply floor and drops its rows.
pub(super) fn close_replies(ui: &AppWindow) {
    next_replies_request();
    let comments = ui.global::<Comments>();
    comments.set_replies_open(false);
    comments.set_replies_loading(false);
    comments.set_replies_error(SharedString::default());
    comments.set_replies(empty_rows());
    comments.set_replies_parent_id(SharedString::default());
}

/// The thread the surface is pointed at. A close or a stale global between two opens answers
/// `None` rather than a thread id of zero.
pub(super) fn comment_thread_of(ui: &AppWindow) -> Option<CommentThread> {
    let comments = ui.global::<Comments>();
    let id = crate::ui::util::parse_cloud_id(comments.get_thread_id().as_str())?;
    match comments.get_thread_kind().as_str() {
        "song" => Some(CommentThread::Song(id)),
        "album" => Some(CommentThread::Album(id)),
        "playlist" => Some(CommentThread::Playlist(id)),
        "mv" => Some(CommentThread::Mv(id)),
        _ => None,
    }
}

/// The signed-in account's id, which decides which rows may be deleted. Anonymous threads
/// render with every delete action closed.
pub(super) fn viewer_id(state: &AppState) -> i64 {
    state.cloud.status().account.map(|account| account.id).unwrap_or_default()
}

pub(super) fn empty_rows() -> ModelRc<CommentRow> {
    ModelRc::from(Rc::new(VecModel::default()))
}

pub(super) fn rows_of(model: &ModelRc<CommentRow>) -> Vec<CommentRow> {
    (0..model.row_count()).filter_map(|index| model.row_data(index)).collect()
}

/// Replaces the rows in place. Recreating the `ModelRc` would reset the list's scroll position
/// on every page, and the model is always the `VecModel` [`empty_rows`] built.
pub(super) fn replace_rows(model: &ModelRc<CommentRow>, rows: Vec<CommentRow>) {
    if let Some(vec) = model.as_any().downcast_ref::<VecModel<CommentRow>>() {
        vec.set_vec(rows);
    }
}

pub(super) fn next_request() -> u64 {
    REQUEST.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) fn request_token() -> u64 {
    REQUEST.load(Ordering::Relaxed)
}

pub(super) fn is_current(token: u64) -> bool {
    REQUEST.load(Ordering::Relaxed) == token
}

pub(super) fn next_replies_request() -> u64 {
    REPLIES_REQUEST.fetch_add(1, Ordering::Relaxed) + 1
}

pub(super) fn replies_token() -> u64 {
    REPLIES_REQUEST.load(Ordering::Relaxed)
}

pub(super) fn replies_are_current(token: u64) -> bool {
    REPLIES_REQUEST.load(Ordering::Relaxed) == token
}

fn reset(comments: &Comments) {
    comments.set_draft_count(0);
    comments.set_rows(empty_rows());
    comments.set_replies(empty_rows());
    comments.set_offset(0);
    comments.set_total(0);
    comments.set_has_more(false);
    comments.set_loading(false);
    comments.set_loading_more(false);
    comments.set_error(SharedString::default());
    comments.set_more_error(SharedString::default());
    comments.set_replies_open(false);
    comments.set_replies_loading(false);
    comments.set_replies_error(SharedString::default());
    comments.set_replies_parent_id(SharedString::default());
}

fn thread_kind(thread: CommentThread) -> &'static str {
    match thread {
        CommentThread::Song(_) => "song",
        CommentThread::Album(_) => "album",
        CommentThread::Playlist(_) => "playlist",
        CommentThread::Mv(_) => "mv",
    }
}
