//! The suggestion popup's fetch. It never touches the result models: a response paints only while
//! its query is still the box's text and still uncommitted, and a failure only logs.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::ui::search::SearchUi;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Search};

pub(super) async fn fetch_suggestions(
    state: &AppState,
    search_ui: &Arc<SearchUi>,
    weak: &Weak<AppWindow>,
    token: u64,
    box_text: String,
) {
    let needle = box_text.trim().to_owned();
    // The committed query is already answered by the result page underneath, and this request
    // still owes its settle: `pending` is what holds the commit back.
    if needle.is_empty() || search_ui.state().last_query.lock().as_str() == needle {
        clear_on_ui(search_ui, token, weak);
        return;
    }

    match state.cloud.search_suggestions(&needle).await {
        Ok(labels) => {
            apply_suggestions(search_ui, weak, SuggestionReply { token, box_text, needle, labels });
        }
        Err(error) => {
            log::warn!("search::suggest: {}", describe(&error));
            clear_on_ui(search_ui, token, weak);
        }
    }
}

struct SuggestionReply {
    token: u64,
    box_text: String,
    needle: String,
    labels: Vec<String>,
}

fn apply_suggestions(search_ui: &Arc<SearchUi>, weak: &Weak<AppWindow>, reply: SuggestionReply) {
    let search_ui = search_ui.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.suggestion_token.load(Ordering::Acquire) != reply.token {
            return;
        }
        let g = ui.global::<Search>();
        g.set_suggestions_pending(false);
        // The box may have moved on without asking again (a chip fills it directly), and a
        // commit between the request and this write makes the rows stale.
        if g.get_query().as_str() != reply.box_text
            || g.get_committed_query().trim() == reply.needle.as_str()
        {
            return;
        }
        let rows = reply.labels.into_iter().map(SharedString::from).collect::<Vec<_>>();
        g.set_suggestions(ModelRc::from(Rc::new(VecModel::from(rows))));
        g.set_suggestions_query(SharedString::from(reply.box_text));
    });
}

fn clear_on_ui(search_ui: &Arc<SearchUi>, token: u64, weak: &Weak<AppWindow>) {
    let search_ui = search_ui.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if search_ui.suggestion_token.load(Ordering::Acquire) != token {
            return;
        }
        let g = ui.global::<Search>();
        g.set_suggestions_pending(false);
        clear_model(&g);
    });
}

pub(super) fn clear_model(g: &Search) {
    g.set_suggestions(ModelRc::from(Rc::new(VecModel::default())));
    g.set_suggestions_query(SharedString::default());
}
