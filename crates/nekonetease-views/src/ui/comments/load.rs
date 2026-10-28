//! The network half of the surface: paging, the reply floor, posting, deleting and liking.
//!
//! Every completion re-checks the request token and the account generation, so a page landing
//! after a sort switch, a close or a sign-out cannot repaint the thread now on screen.

use slint::{ComponentHandle, Model, SharedString};

use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::CommentThread;
use nekonetease_core::entities::comment::{CommentPage, CommentSort};
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, CommentRow, Comments};

use super::{
    comment_thread_of, empty_rows, is_current, like, models, next_replies_request, next_request,
    paging, replace_rows, replies_are_current, replies_token, request_token, rows_of, viewer_id,
};

/// Which model a like patch names. The floor holds its own rows, so one callback serves both.
#[derive(Clone, Copy)]
pub(super) enum Floor {
    Thread,
    Replies,
}

pub(super) fn first(ui: &AppWindow, state: &AppState) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let comments = ui.global::<Comments>();
    let sort = if comments.get_hot() { CommentSort::Hot } else { CommentSort::Time };
    let token = next_request();
    comments.set_loading(true);
    comments.set_loading_more(false);
    comments.set_error(SharedString::default());
    comments.set_more_error(SharedString::default());
    comments.set_rows(empty_rows());
    comments.set_offset(0);
    comments.set_total(0);
    comments.set_has_more(false);

    let state = state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.comment_page(thread, sort, 0).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(token) || !same_session(&state, generation) || !showing(&ui, thread) {
                return;
            }
            let comments = ui.global::<Comments>();
            comments.set_loading(false);
            match result {
                Ok(page) => apply_page(&comments, &page, &state),
                Err(error) => {
                    log::warn!("comments::first: {}", describe(&error));
                    comments.set_error(describe(&error).into());
                }
            }
        });
    });
}

pub(super) fn more(ui: &AppWindow, state: &AppState) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let comments = ui.global::<Comments>();
    if !comments.get_has_more() || comments.get_loading() || comments.get_loading_more() {
        return;
    }
    let Some(offset) = u64::try_from(comments.get_offset()).ok() else { return };
    let sort = if comments.get_hot() { CommentSort::Hot } else { CommentSort::Time };
    // A page reuses the token it was opened under: a later `first` supersedes it, while this
    // page neither invalidates itself nor the rows already on screen.
    let token = request_token();
    comments.set_loading_more(true);

    let state = state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.comment_page(thread, sort, offset).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(token) || !same_session(&state, generation) || !showing(&ui, thread) {
                return;
            }
            let comments = ui.global::<Comments>();
            comments.set_loading_more(false);
            match result {
                Ok(page) => {
                    let mut rows = rows_of(&comments.get_rows());
                    let added = paging::append_unique(&mut rows, page_rows(&page, &state));
                    let returned = page.comments.len();
                    replace_rows(&comments.get_rows(), rows);
                    comments.set_offset(paging::advance_offset(comments.get_offset(), returned));
                    comments.set_total(models::count_to_i32(page.total));
                    // A page whose every row was already held would leave the same cursor and
                    // ask for it forever, so a hot thread stops rather than spinning.
                    comments.set_has_more(page.more && added > 0);
                    comments.set_more_error(SharedString::default());
                }
                Err(error) => {
                    log::warn!("comments::more({offset}): {}", describe(&error));
                    comments.set_more_error(describe(&error).into());
                }
            }
        });
    });
}

/// Selects an ordering. The two routes count offsets independently, so a move drops the rows
/// and starts again rather than continuing the cursor the other route was on.
pub(super) fn pick_sort(ui: &AppWindow, state: &AppState, hot: bool) {
    let comments = ui.global::<Comments>();
    if !paging::sort_changed(comments.get_hot(), hot) {
        return;
    }
    comments.set_hot(hot);
    first(ui, state);
}

pub(super) fn replies(ui: &AppWindow, state: &AppState, parent_id: i64) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let comments = ui.global::<Comments>();
    let token = next_replies_request();
    comments.set_replies_open(true);
    comments.set_replies_parent_id(parent_id.to_string().into());
    comments.set_replies_loading(true);
    comments.set_replies_error(SharedString::default());
    comments.set_replies(empty_rows());

    let state = state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.comment_replies(thread, parent_id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !replies_are_current(token)
                || !same_session(&state, generation)
                || !showing(&ui, thread)
            {
                return;
            }
            let comments = ui.global::<Comments>();
            comments.set_replies_loading(false);
            match result {
                Ok(page) => {
                    let rows = page_rows(&page, &state);
                    replace_rows(&comments.get_replies(), rows);
                }
                Err(error) => {
                    log::warn!("comments::replies({parent_id}): {}", describe(&error));
                    comments.set_replies_error(describe(&error).into());
                }
            }
        });
    });
}

/// Appends a post the moment the server accepts it. Nothing is shown while the request is in
/// flight, so a refused post leaves the draft in place and the thread untouched.
pub(super) fn post(ui: &AppWindow, state: &AppState) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let comments = ui.global::<Comments>();
    if comments.get_posting() {
        return;
    }
    let content = comments.get_draft().trim().to_owned();
    if content.is_empty() {
        return;
    }
    let reply_to = crate::ui::util::parse_cloud_id(comments.get_reply_to_id().as_str());
    let token = request_token();
    comments.set_posting(true);

    let state = state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.post_comment(thread, &content, reply_to).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(token) || !same_session(&state, generation) || !showing(&ui, thread) {
                return;
            }
            let comments = ui.global::<Comments>();
            comments.set_posting(false);
            let posted = match result {
                Ok(posted) => posted,
                Err(error) => {
                    log::warn!("comments::post: {}", describe(&error));
                    toast::notify(ToastKind::OperationFailed, describe(&error));
                    return;
                }
            };
            let row = models::to_row(&posted, viewer_id(&state));
            comments.set_draft(SharedString::default());
            comments.set_draft_count(0);
            comments.set_reply_to_id(SharedString::default());
            comments.set_reply_to_author(SharedString::default());
            append_posted(&ui, &row, posted.parent_id);
        });
    });
}

pub(super) fn delete(ui: &AppWindow, state: &AppState, remote_id: &str) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let Some(comment_id) = crate::ui::util::parse_cloud_id(remote_id) else { return };
    let token = request_token();
    let opened_floor = replies_token();

    let state = state.clone();
    let weak = ui.as_weak();
    let key = remote_id.to_owned();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.delete_comment(thread, comment_id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !is_current(token) || !same_session(&state, generation) || !showing(&ui, thread) {
                return;
            }
            if let Err(error) = result {
                log::warn!("comments::delete({comment_id}): {}", describe(&error));
                toast::notify(ToastKind::OperationFailed, describe(&error));
                return;
            }
            let comments = ui.global::<Comments>();
            remove_row(&comments.get_rows(), key.as_str());
            comments.set_total(comments.get_total().saturating_sub(1));
            if replies_token() == opened_floor {
                remove_row(&comments.get_replies(), key.as_str());
            }
            if comments.get_reply_to_id().as_str() == key.as_str() {
                comments.set_reply_to_id(SharedString::default());
                comments.set_reply_to_author(SharedString::default());
            }
        });
    });
}

pub(super) fn toggle_like(
    ui: &AppWindow,
    state: &AppState,
    floor: Floor,
    index: i32,
    remote_id: &str,
    next: bool,
) {
    let Some(thread) = comment_thread_of(ui) else { return };
    let Ok(index) = usize::try_from(index) else { return };
    let Some(comment_id) = crate::ui::util::parse_cloud_id(remote_id) else { return };
    let comments = ui.global::<Comments>();
    let Some(snapshot) = like::apply(&model_of(&comments, floor), index, remote_id, next) else {
        return;
    };
    let token = request_token();

    let state = state.clone();
    let weak = ui.as_weak();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        if let Err(error) = state.cloud.like_comment(thread, comment_id, next).await {
            let detail = describe(&error);
            log::warn!("comments::toggle_like({comment_id}): {detail}");
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if !is_current(token) || !same_session(&state, generation) || !showing(&ui, thread)
                {
                    return;
                }
                let comments = ui.global::<Comments>();
                like::rollback(&model_of(&comments, floor), &snapshot);
                toast::notify(ToastKind::OperationFailed, detail);
            });
        }
    });
}

fn apply_page(comments: &Comments, page: &CommentPage, state: &AppState) {
    replace_rows(&comments.get_rows(), page_rows(page, state));
    comments.set_offset(paging::advance_offset(0, page.comments.len()));
    comments.set_total(models::count_to_i32(page.total));
    comments.set_has_more(page.more);
    comments.set_hot(page.hot);
}

fn append_posted(ui: &AppWindow, row: &CommentRow, parent_id: i64) {
    let comments = ui.global::<Comments>();
    if parent_id > 0 {
        let reply_key = parent_id.to_string();
        if comments.get_replies_open() && comments.get_replies_parent_id().as_str() == reply_key {
            let mut rows = rows_of(&comments.get_replies());
            paging::append_unique(&mut rows, vec![row.clone()]);
            replace_rows(&comments.get_replies(), rows);
        }
        bump_reply_count(&comments, &reply_key);
        return;
    }
    let mut rows = rows_of(&comments.get_rows());
    paging::append_unique(&mut rows, vec![row.clone()]);
    replace_rows(&comments.get_rows(), rows);
    comments.set_total(comments.get_total().saturating_add(1));
}

fn bump_reply_count(comments: &Comments, parent_key: &str) {
    let model = comments.get_rows();
    for index in 0..model.row_count() {
        if let Some(row) = model.row_data(index)
            && row.remote_id.as_str() == parent_key
        {
            model.set_row_data(
                index,
                CommentRow { reply_count: row.reply_count.saturating_add(1), ..row },
            );
            return;
        }
    }
}

fn remove_row(model: &slint::ModelRc<CommentRow>, remote_id: &str) {
    let rows: Vec<CommentRow> =
        rows_of(model).into_iter().filter(|row| row.remote_id.as_str() != remote_id).collect();
    replace_rows(model, rows);
}

fn page_rows(page: &CommentPage, state: &AppState) -> Vec<CommentRow> {
    let viewer = viewer_id(state);
    page.comments.iter().map(|comment| models::to_row(comment, viewer)).collect()
}

fn model_of(comments: &Comments, floor: Floor) -> slint::ModelRc<CommentRow> {
    match floor {
        Floor::Thread => comments.get_rows(),
        Floor::Replies => comments.get_replies(),
    }
}

fn showing(ui: &AppWindow, thread: CommentThread) -> bool {
    ui.global::<Comments>().get_open() && comment_thread_of(ui) == Some(thread)
}

fn same_session(state: &AppState, generation: u64) -> bool {
    state.cloud.status().generation == generation
}
