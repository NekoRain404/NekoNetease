//! Browser-style in-memory back/forward navigation history, tracking the user's path
//! across sidebar-tab switches and detail open/close, walked by Mouse-4 / Mouse-5.
//!
//! [`NavEntry`] is a snapshot of the *visible state* — a section, the tab within it for
//! a section built out of tabs, and the optional detail id open there. [`NavHistory`] is
//! a linear stack with a cursor, where user-initiated navigation truncates the forward
//! portion, as the HTML5 History API does.
//!
//! Recording happens at each user-action call site. The replay path drives the same
//! callbacks, so a single `suppress` flag keeps a replay-triggered callback from
//! re-pushing the entry it just walked to. It is a plain bool rather than a counter, so
//! **the two regions it guards must not nest**: `replay`'s own synchronous step, and the
//! [`PendingNav`] a cross-view walk defers into the destination `open_*`'s hook, which
//! lands a section flip — and so a `record_current` — long after that step returned.
//! [`PendingNav::apply`] and [`PendingNav::apply_deferred`] are that split.
//!
//! **A replay also takes a [`ReplayToken`] before it moves the cursor, and the open it
//! schedules carries it to the write that lands the detail.** The cursor check at the start
//! is not enough: the open crosses a database read and an artwork decode, and a press
//! landing in that gap would otherwise be painted over once that fetch resolves.
//!
//! Not persisted: each launch starts empty. `boot::ui_setup::views::install_views` seeds one
//! entry for the section landed on, and any persisted detail's async open records itself
//! on the way in.

use std::future::Future;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use slint::{ComponentHandle, Weak};

use crate::ui::my_library::{
    self, MyLibraryTab, NAV_MY_LIBRARY, persist_tab, tab_from_index, tab_of_section,
};
use crate::ui::view_tag;
use nekonetease_app::state::AppState;
use nekonetease_core::error::{AppResult, describe};
use nekonetease_ui::{AppWindow, Dialog, MyLibrary, Nav, NavEnterFrom, Queue};

mod replay;

pub use self::replay::ReplayToken;
pub(crate) use self::replay::open_token;
use self::replay::{ReplayOpen, in_replay, land_pending, replay_hook};

mod history;

pub use self::history::{NavEntry, NavHistory};

/// The visible detail id for `(section, tab)`, or `None` if that view has no detail
/// concept or its detail is closed. Past the My Library check everything is
/// [`my_library::detail_id_for`] — which takes the tab rather than reading the mounted one
/// precisely so this caller can ask about a *recorded* entry's tab.
///
fn current_detail_id_for(ui: &AppWindow, section: i32, tab: i32) -> Option<i64> {
    if section != NAV_MY_LIBRARY {
        return None;
    }
    my_library::detail_id_for(ui, tab_from_index(&ui.global::<MyLibrary>(), tab))
}

/// Record an arbitrary snapshot, for the detail open/close hooks where the new state is
/// known at the call site.
pub fn record(entry: NavEntry) {
    nav().history().record(entry);
}

/// Record the current visible state by reading globals, for the hooks that fire once the
/// *new* state is already there.
pub fn record_current(ui: &AppWindow) {
    let section = ui.global::<Nav>().get_selected_index();
    let tab = tab_of_section(ui, section);
    let detail_id = current_detail_id_for(ui, section, tab);
    // Only what the history took: the eleven hooks fire two or three deep for one click,
    // and logging each reads as a stutter rather than a navigation.
    if nav().history().record(NavEntry { section, tab, detail_id }) {
        view_tag::log_current(ui);
    }
}

/// Walk one step back or forward and drive the UI into the new state, under
/// `set_suppress(true)` so the transition hooks it triggers don't re-record. Suppress is
/// cleared after the synchronous portion — but a cross-view walk into a detail defers
/// its navigation into the destination `open_*`'s hook, which lands a section flip long
/// after that, so [`PendingNav::apply_deferred`] re-arms the flag around itself. The two
/// regions must not nest; see the module docs.
pub fn replay(state: &AppState, ui: &AppWindow, going_back: bool) {
    let target = {
        let mut hist = nav().history();
        if going_back { hist.back() } else { hist.forward() }
    };
    let Some(target) = target else {
        // **No target, no token.** Beginning one here would retire the replay an in-flight
        // `open_*` is still holding — the user pressed back before history had anywhere to go,
        // so the detail they just asked for would arrive and be discarded, looking like a
        // failed click rather than a no-op.
        return;
    };
    let token = nav().replay_order.begin();

    let current_section = ui.global::<Nav>().get_selected_index();
    let current_tab = tab_of_section(ui, current_section);
    let current_detail = current_detail_id_for(ui, current_section, current_tab);
    let direction = if going_back { NavEnterFrom::Left } else { NavEnterFrom::Right };

    // Its own line because a replay suppresses recording, so this is otherwise
    // the one navigation leaving no trace.
    log::debug!(
        "nav: history {} → section {} tab {} detail {:?}",
        if going_back { "back" } else { "forward" },
        target.section,
        target.tab,
        target.detail_id
    );

    nav().history().set_suppress(true);

    if target.section == current_section && target.tab == current_tab {
        apply_same_view(
            state,
            ui,
            target.section,
            target.tab,
            current_detail,
            target.detail_id,
            direction,
        );
    } else {
        // A different *view* — a My Library tab move, a section switch, or both. Four
        // things are owed either way: tear down whatever the leaving view has open so
        // its release and persist side-effects run, mark the direction, land on the
        // target tab, and put the section where the target names.
        //
        // **Whether those run now or later turns on whether a detail is coming.**
        // `my-library-view.slint`'s body router is a pure function of `(tab-idx, the
        // four ids)` with no third state, and the id lands a DB fetch plus an artwork
        // decode after the tab would — so a tab written here mounts the destination's
        // *grid* for the length of that fetch. Handed to `open_*_with`'s hook instead,
        // both land in one UI-thread tick and the router only ever sees the detail.
        let pending = PendingNav {
            token,
            from: current_detail.is_some().then_some((current_section, current_tab)),
            section: target.section,
            tab: target.tab,
            direction,
        };
        if let Some(id) = target.detail_id {
            spawn_open_detail(state, ui, target.section, target.tab, id, direction, Some(pending));
        } else {
            // Nothing to wait for — the destination view *is* the grid.
            pending.apply(ui);
        }
    }

    nav().history().set_suppress(false);
}

fn apply_same_view(
    state: &AppState,
    ui: &AppWindow,
    section: i32,
    tab: i32,
    current: Option<i64>,
    target: Option<i64>,
    direction: NavEnterFrom,
) {
    match (current, target) {
        (Some(_), None) => invoke_close_detail(ui, section, tab),
        // No navigation to defer — the view the detail belongs to is already the one
        // on screen, so the hook has nothing to land and `open_*` marks for itself.
        (_, Some(id)) => spawn_open_detail(state, ui, section, tab, id, direction, None),
        (None, None) => crate::ui::nav_transition::mark(ui, direction),
    }
}

fn invoke_close_detail(ui: &AppWindow, section: i32, tab: i32) {
    if section != NAV_MY_LIBRARY {
        return;
    }
    let tab = tab_from_index(&ui.global::<MyLibrary>(), tab);
    crate::ui::my_library::close_open_detail(ui, tab);
}

/// The navigation a cross-view replay owes, deferred so it lands in the same UI-thread tick
/// as the destination detail's id — the router has no third state to park it in.
#[derive(Clone, Copy)]
struct PendingNav {
    token: ReplayToken,
    /// The view being left, when it has a detail to close.
    from: Option<(i32, i32)>,
    section: i32,
    tab: i32,
    direction: NavEnterFrom,
}

impl PendingNav {
    /// Land it. For [`replay`]'s own synchronous arm, which is already inside the suppression —
    /// the deferred callers take [`Self::apply_deferred`].
    fn apply(self, ui: &AppWindow) {
        if let Some((section, tab)) = self.from {
            invoke_close_detail(ui, section, tab);
        }
        // **After the close**, which runs `mark_drill_back` and would otherwise clobber the
        // direction `open_*` marked a few statements above this hook.
        crate::ui::nav_transition::mark(ui, self.direction);
        // Before the section flip, so the page mounts on the tab it is meant to be on rather
        // than on whichever one it was left at.
        persist_tab(ui, self.tab);
        // **Read the index live, after the close.** A detail opened by a cross-section drill
        // carries an `origin-nav-index` and closing it *restores that section*, so a
        // same-section walk out of one has already left the page by now. The live read still
        // skips the redundant persist on an ordinary same-section move.
        let nav = ui.global::<Nav>();
        if nav.get_selected_index() != self.section {
            nav.set_selected_index(self.section);
            nav.invoke_persist_selected_index(self.section);
        }
    }

    /// The deferred form, for the callers that run after [`replay`] has returned: the `open_*`
    /// hook and [`Self::land`]. Suppression is dropped by then and the section flip inside
    /// reaches the sidebar hook's `record_current`, which would push the entry the walk just
    /// walked *to*.
    fn apply_deferred(self, ui: &AppWindow) {
        nav().history().set_suppress(true);
        self.apply(ui);
        nav().history().set_suppress(false);
    }

    /// Land it on an event-loop hop of its own, for the paths where the open never ran or
    /// failed. Without it a press against a deleted playlist — or one arriving before the view
    /// handles are wired — does nothing at all.
    fn land(self, weak: &Weak<AppWindow>) {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if self.token.is_current() {
                self.apply_deferred(&ui);
            }
        });
    }
}

/// Schedule the tab's `open_*` future, bailing at the start if a newer replay has already
/// advanced the cursor past `expected`.
///
/// That check alone would leave the window open: the open then awaits a DB fetch and an
/// artwork decode, and a press landing in that gap would write its stale `*-id` over the newer
/// one. The token travels into the open through [`in_replay`], so every apply point inside
/// `open_*_with` re-checks it after its awaits.
///
/// `pending` is the cross-view navigation, handed to the `open_*_with` hook so it shares a tick
/// with the id write; `None` when the target is the view already on screen. **Every path that
/// can still open something and doesn't reach the hook has to land it instead**, or the press
/// is silently swallowed — that is the missing-handle bail and each open's `Err` arm. Four
/// paths deliberately land nothing: the newer-replay bail at the start and the opener's own
/// stale-token return, where a later press owns the screen in either case, plus the two guards
/// below — a non-My-Library section and the Songs tab owning no detail at all.
fn spawn_open_detail(
    state: &AppState,
    ui: &AppWindow,
    section: i32,
    tab: i32,
    id: i64,
    direction: NavEnterFrom,
    pending: Option<PendingNav>,
) {
    if section != NAV_MY_LIBRARY {
        return;
    }
    // A cross-view walk carries the token of the replay that built it; a same-view one reads
    // the order here, which is that replay's too — `replay` has not returned yet.
    let token = pending.map_or_else(ReplayToken::current, |pending| pending.token);
    // `weak` is consumed by whichever `open_*_with` runs; `ReplayOpen::fallback` is what is
    // left to land the navigation on if it never gets there.
    let weak: Weak<AppWindow> = ui.as_weak();
    let replay = |label| ReplayOpen {
        token,
        expected: NavEntry { section, tab, detail_id: Some(id) },
        pending,
        fallback: ui.as_weak(),
        label,
    };
    match tab_from_index(&ui.global::<MyLibrary>(), tab) {
        MyLibraryTab::Songs => {}
        MyLibraryTab::Albums => {
            let handle = nav().handles().albums.lock().clone();
            spawn_open(state, handle, replay("open_album"), move |s, au| async move {
                let hook = replay_hook(pending);
                crate::ui::albums::open_album_with(&s, &au, weak, id, direction, hook).await
            });
        }
        MyLibraryTab::Artists => {
            let handle = nav().handles().artists.lock().clone();
            spawn_open(state, handle, replay("open_artist"), move |s, au| async move {
                let hook = replay_hook(pending);
                crate::ui::artists::open_artist_with(&s, &au, weak, id, direction, hook).await
            });
        }
        MyLibraryTab::Genres => {
            let handle = nav().handles().genres.lock().clone();
            spawn_open(state, handle, replay("open_genre"), move |s, gu| async move {
                let hook = replay_hook(pending);
                crate::ui::genres::open_genre_with(&s, &gu, weak, id, direction, hook).await
            });
        }
        MyLibraryTab::Playlists => {
            let handle = nav().handles().playlists.lock().clone();
            spawn_open(state, handle, replay("open_playlist"), move |s, pu| async move {
                let hook = replay_hook(pending);
                crate::ui::playlists::open_playlist_with(&s, &pu, weak, id, direction, hook).await
            });
        }
    }
}

/// Run `open` behind the newer-replay bail. The missing-handle bail and a failed open each land
/// the pending navigation, being the two paths that can still open something and don't reach
/// the hook.
fn spawn_open<H, Fut>(
    state: &AppState,
    handle: Option<Arc<H>>,
    replay: ReplayOpen,
    open: impl FnOnce(AppState, Arc<H>) -> Fut + Send + 'static,
) where
    H: Send + Sync + 'static,
    Fut: Future<Output = AppResult<()>> + Send + 'static,
{
    let ReplayOpen { token, expected, pending, fallback, label } = replay;
    let Some(handle) = handle else {
        land_pending(pending, &fallback);
        return;
    };
    let s = state.clone();
    state.runtime.spawn(async move {
        // The cursor catches a user navigation that moved the history since the press; the
        // token catches a later replay, including one to the same entry.
        if !token.is_current() || nav().history().current() != Some(expected) {
            return;
        }
        if let Err(e) = in_replay(token, open(s, handle)).await {
            let id = expected.detail_id.unwrap_or_default();
            log::warn!("nav_history::replay {label}({id}): {}", describe(&e));
            land_pending(pending, &fallback);
        }
    });
}

/// Whether Now Playing, the queue sheet or a modal dialog is up. Mouse-4/5 gate on it:
/// overlays have their own input contexts, Esc and F closing them, and absorbing
/// browser-style nav while one is up would surprise the user.
pub fn overlay_open(ui: &AppWindow) -> bool {
    ui.global::<Nav>().get_now_playing_open()
        || ui.global::<Queue>().get_open()
        || ui.global::<Dialog>().get_open()
}

/// Per-section UI-handle registry, filled at startup by each `wire_*` once its
/// `Arc<*Ui>` exists and read by the replay's `open_*` dispatch.
/// `Mutex<Option<…>>` so it can be populated lazily during boot rather than threading
/// the handles through every callback.
///
/// Named for the *sections* it registers rather than for the handles, `boot::ui_setup::views`
/// having its own unrelated `UiHandles` that this used to collide with.
#[derive(Default)]
pub struct SectionHandles {
    pub albums: Mutex<Option<Arc<crate::ui::albums::AlbumsUi>>>,
    pub artists: Mutex<Option<Arc<crate::ui::artists::ArtistsUi>>>,
    pub genres: Mutex<Option<Arc<crate::ui::genres::GenresUi>>>,
    pub playlists: Mutex<Option<Arc<crate::ui::playlists::PlaylistsUi>>>,
}

/// Everything browser-style navigation needs, owned by the view layer.
///
/// Both halves sat on `AppState` and were the only reason it named anything under `ui::`. That
/// edge is the one no dependency direction survives, `src/ui/` reaching `state` from 138 places.
#[derive(Default)]
pub struct Navigation {
    history: Mutex<NavHistory>,
    handles: SectionHandles,
    replay_order: replay::ReplayOrder,
}

impl Navigation {
    /// The back/forward stack. In-memory only — reset on each launch.
    pub fn history(&self) -> parking_lot::MutexGuard<'_, NavHistory> {
        self.history.lock()
    }

    pub fn handles(&self) -> &SectionHandles {
        &self.handles
    }
}

/// The one navigation state this process has.
///
/// A static rather than a value threaded from `boot`, because reaching it from the twelve
/// `'static` callbacks that record and the five `open_*_with` futures that land would mean a
/// parameter on about fifty functions to arrive at a value there is exactly one of — the same
/// bargain `utils::toast`'s bridge and `window_chrome`'s geometry statics already take. It
/// carries no `AppState`, so nothing here can grow a second way to reach the app layer.
static NAVIGATION: LazyLock<Navigation> = LazyLock::new(Navigation::default);

/// The process's navigation state. Fresh on first use, so there is no install to order.
pub fn nav() -> &'static Navigation {
    &NAVIGATION
}

#[cfg(test)]
#[path = "tests/nav_history_tests.rs"]
mod tests;
