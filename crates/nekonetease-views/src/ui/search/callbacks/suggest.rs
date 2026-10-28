//! `Search.*` suggestion callbacks: the box's throttle asks, a row pick commits the same way a
//! recent chip does. See [`super::wire`].

use std::sync::Arc;
use std::sync::atomic::Ordering;

use slint::ComponentHandle;

use crate::ui::search::{SearchUi, suggest};
use nekonetease_app::state::AppState;
use nekonetease_ui::{AppWindow, Search};

pub(super) fn wire(ui: &AppWindow, state: &AppState, search_ui: &Arc<SearchUi>) {
    let g = ui.global::<Search>();
    let weak = ui.as_weak();

    {
        let s = state.clone();
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_request_suggestions(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            // Claimed here, on the UI thread, so a second keystroke's request cannot race the
            // worker for the token. `pending` is what holds the commit back until the answer.
            let token = su.suggestion_token.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
            ui.global::<Search>().set_suggestions_pending(true);
            let s = s.clone();
            let su = su.clone();
            let weak = weak.clone();
            let query = query.to_string();
            let runtime = s.runtime.clone();
            runtime.spawn(async move {
                suggest::fetch_suggestions(&s, &su, &weak, token, query).await;
            });
        });
    }

    {
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_suggestion_pick(move |term| {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Search>();
            // The commit lands 300 ms out, and a label equal to what was typed would leave the
            // popup up over the old rows until then.
            su.suggestion_token.fetch_add(1, Ordering::AcqRel);
            g.set_suggestions_pending(false);
            suggest::clear_model(&g);
            g.set_query(term.clone());
            // The pick decides the search, so it is committed at once: the 300 ms debounce still
            // carries it to `commit-search`, and a suggestion tick already in flight reads this
            // rather than asking for the term the pick just chose.
            g.set_committed_query(term);
            g.set_debounce_tick_pending(g.get_debounce_tick_pending() + 1);
        });
    }

    {
        let su = search_ui.clone();
        let weak = weak.clone();
        g.on_dismiss_suggestions(move || {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<Search>();
            // A click outside is the user passing on the rows: settling the exchange is what
            // lets the commit the timer deferred run.
            su.suggestion_token.fetch_add(1, Ordering::AcqRel);
            g.set_suggestions_pending(false);
            suggest::clear_model(&g);
        });
    }
}
