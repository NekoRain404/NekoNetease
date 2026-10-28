//! Podcast / DJ-radio browsing and playback.
//!
//! The page is two levels: a recommended-station strip whose rows carry an index rather than an
//! exact id, and the programmes of whichever station that index selected. Playback is the cloud
//! queue's, keyed by an episode's `mainSong`, so nothing here opens a second audio path.

mod fetch;
mod rows;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use nekonetease_app::state::AppState;
use nekonetease_core::entities::dj::{DjProgram, DjRadio};
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_ui::{AppWindow, Dj, TrackListRow as UiTrackListRow};

use super::util::parse_cloud_id;
use super::view_ctx::ViewCtx;

static RADIO_LIST_REQUEST: AtomicU64 = AtomicU64::new(0);
static RADIO_DETAIL_REQUEST: AtomicU64 = AtomicU64::new(0);
static PROGRAM_LIST_REQUEST: AtomicU64 = AtomicU64::new(0);
static PROGRAM_DETAIL_REQUEST: AtomicU64 = AtomicU64::new(0);
static SUBSCRIBE_REQUEST: AtomicU64 = AtomicU64::new(0);

/// What the page's models index into: the stations the strip last drew, the station whose
/// programmes are open, and those programmes in row order. `DjRadio` also carries the host and
/// subscription state a reload of the detail needs.
#[derive(Default)]
pub(super) struct DjState {
    pub(super) radios: Vec<DjRadio>,
    pub(super) selected: Option<DjRadio>,
    /// The rows' programme ids, in the order the model indexes them. A row carries the
    /// `mainSong` id it plays, not the programme id the detail endpoint needs.
    pub(super) programs: Vec<DjProgram>,
}

pub(super) type SharedDj = Arc<Mutex<DjState>>;

/// Wire the podcast page. Mounting it is the shell's job; this only registers its callbacks.
pub fn install(cx: ViewCtx<'_>) {
    let ViewCtx { app, state, .. } = cx;
    let data: SharedDj = Arc::new(Mutex::new(DjState::default()));
    wire_radios(app, state, &data);
    wire_radio(app, state, &data);
    wire_programs(app, state, &data);
    wire_detail(app, state, &data);
    wire_playback(app, state);
}

fn wire_radios(ui: &AppWindow, app_state: &AppState, shared: &SharedDj) {
    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_load(move || {
        let Some(ui) = weak.upgrade() else { return };
        load_radios_once(&ui, &state, &data);
    });

    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_refresh(move || {
        let Some(ui) = weak.upgrade() else { return };
        refresh_radios(&ui, &state, &data);
    });
}

/// The first mount asks once; a return to the page keeps the strip it already drew.
fn load_radios_once(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    let global = ui.global::<Dj>();
    if global.get_radios_loading() || global.get_radios().row_count() > 0 {
        return;
    }
    begin_radio_load(ui, state, data);
}

fn refresh_radios(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    if ui.global::<Dj>().get_radios_loading() {
        return;
    }
    begin_radio_load(ui, state, data);
}

fn begin_radio_load(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    let global = ui.global::<Dj>();
    global.set_radios_loading(true);
    global.set_radios_error(SharedString::default());
    fetch::load_radios(state, ui.as_weak(), data.clone());
}

fn wire_radio(ui: &AppWindow, app_state: &AppState, shared: &SharedDj) {
    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_open_radio(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let Ok(index) = usize::try_from(index) else { return };
        let radio = data.lock().radios.get(index).cloned();
        if let Some(radio) = radio {
            open_radio(&ui, &state, &data, radio);
        }
    });

    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_retry_radio(move || {
        let Some(ui) = weak.upgrade() else { return };
        let radio = data.lock().selected.clone();
        let Some(radio) = radio else { return };
        if radio.id <= 0 {
            return;
        }
        let global = ui.global::<Dj>();
        global.set_radio_loading(true);
        global.set_radio_error(SharedString::default());
        fetch::load_radio_detail(&state, ui.as_weak(), radio.id);
    });

    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_toggle_subscribe(move || {
        let Some(ui) = weak.upgrade() else { return };
        toggle_subscribe(&ui, &state, &data);
    });
}

/// Selecting a station resets both its detail and the programme page in one tick; the strip's
/// index is resolved against the stored list, so the exact 64-bit id survives.
fn open_radio(ui: &AppWindow, state: &AppState, data: &SharedDj, radio: DjRadio) {
    let radio_id = radio.id;
    if radio_id <= 0 {
        return;
    }
    close_program_detail(ui);
    let global = ui.global::<Dj>();
    global.set_radio_remote_id(radio_id.to_string().into());
    global.set_radio_title(radio.name.clone().into());
    global.set_radio_host(radio.host_name().into());
    global.set_radio_artwork_path(rows::remote_artwork(radio.cover_url.as_deref()).into());
    global.set_radio_description(radio.description.clone().into());
    global.set_radio_subscribed(radio.subscribed);
    global.set_radio_subscribe_pending(false);
    global.set_radio_loading(true);
    global.set_radio_error(SharedString::default());
    global.set_programs(empty_programs());
    global.set_programs_loading(true);
    global.set_programs_loading_more(false);
    global.set_programs_has_more(false);
    global.set_programs_error(SharedString::default());
    {
        let mut stored = data.lock();
        stored.selected = Some(radio);
        stored.programs.clear();
    }

    fetch::load_radio_detail(state, ui.as_weak(), radio_id);
    fetch::load_programs(
        state,
        ui.as_weak(),
        fetch::ProgramsRequest { radio_id, page: fetch::ProgramsPage::First },
        data.clone(),
    );
}

/// A flips-before-request toggle: the pill moves on the click and returns on failure. The request
/// token, the rendered id and the account generation keep a late answer off a station opened since.
fn toggle_subscribe(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    let global = ui.global::<Dj>();
    if global.get_radio_subscribe_pending() {
        return;
    }
    let Some(radio_id) = parse_cloud_id(global.get_radio_remote_id().as_str()) else { return };
    let subscribed = global.get_radio_subscribed();
    let request = SUBSCRIBE_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let remote_id = radio_id.to_string();
    global.set_radio_subscribe_pending(true);
    global.set_radio_error(SharedString::default());

    let cloud = state.cloud.clone();
    let state = state.clone();
    let weak = ui.as_weak();
    let data = data.clone();
    state.runtime.clone().spawn(async move {
        let generation = cloud.status().generation;
        let result = cloud.dj_set_subscribed(radio_id, !subscribed).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if SUBSCRIBE_REQUEST.load(Ordering::Relaxed) != request
                || cloud.status().generation != generation
            {
                return;
            }
            let global = ui.global::<Dj>();
            if global.get_radio_remote_id().as_str() != remote_id {
                return;
            }
            global.set_radio_subscribe_pending(false);
            match result {
                Ok(()) => {
                    let next = !subscribed;
                    global.set_radio_subscribed(next);
                    global.set_radio_error(SharedString::default());
                    if let Some(selected) = data.lock().selected.as_mut()
                        && selected.id == radio_id
                    {
                        selected.subscribed = next;
                    }
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!("dj::toggle_subscribe({radio_id}): {message}");
                    global.set_radio_error(message.clone().into());
                    toast::notify(ToastKind::OperationFailed, message);
                }
            }
        });
    });
}

fn wire_programs(ui: &AppWindow, app_state: &AppState, shared: &SharedDj) {
    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_load_more_programs(move || {
        let Some(ui) = weak.upgrade() else { return };
        load_more_programs(&ui, &state, &data);
    });

    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_retry_programs(move || {
        let Some(ui) = weak.upgrade() else { return };
        retry_programs(&ui, &state, &data);
    });
}

fn load_more_programs(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    let global = ui.global::<Dj>();
    if global.get_programs_loading() || global.get_programs_loading_more() {
        return;
    }
    if !global.get_programs_has_more() {
        return;
    }
    let Some(radio_id) = parse_cloud_id(global.get_radio_remote_id().as_str()) else { return };
    let offset = row_count_u64(&global.get_programs());
    global.set_programs_loading_more(true);
    global.set_programs_error(SharedString::default());
    fetch::load_programs(
        state,
        ui.as_weak(),
        fetch::ProgramsRequest { radio_id, page: fetch::ProgramsPage::More { offset } },
        data.clone(),
    );
}

fn retry_programs(ui: &AppWindow, state: &AppState, data: &SharedDj) {
    let global = ui.global::<Dj>();
    if global.get_programs_loading() || global.get_programs_loading_more() {
        return;
    }
    let Some(radio_id) = parse_cloud_id(global.get_radio_remote_id().as_str()) else { return };
    let offset = row_count_u64(&global.get_programs());
    let page = if offset == 0 {
        global.set_programs_loading(true);
        fetch::ProgramsPage::First
    } else {
        global.set_programs_loading_more(true);
        fetch::ProgramsPage::More { offset }
    };
    global.set_programs_error(SharedString::default());
    fetch::load_programs(
        state,
        ui.as_weak(),
        fetch::ProgramsRequest { radio_id, page },
        data.clone(),
    );
}

fn row_count_u64<T: Clone>(model: &ModelRc<T>) -> u64 {
    u64::try_from(model.row_count()).unwrap_or(u64::MAX)
}

fn wire_detail(ui: &AppWindow, app_state: &AppState, shared: &SharedDj) {
    let weak = ui.as_weak();
    let state = app_state.clone();
    let data = shared.clone();
    ui.global::<Dj>().on_open_program_row(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        open_program_row(&ui, &state, &data, index);
    });

    let weak = ui.as_weak();
    ui.global::<Dj>().on_close_detail(move || {
        let Some(ui) = weak.upgrade() else { return };
        close_program_detail(&ui);
    });

    let weak = ui.as_weak();
    let state = app_state.clone();
    ui.global::<Dj>().on_retry_program(move || {
        let Some(ui) = weak.upgrade() else { return };
        let remote_id = ui.global::<Dj>().get_detail_remote_id().to_string();
        let Some(program_id) = parse_cloud_id(&remote_id) else { return };
        open_program(&ui, &state, program_id);
    });
}

fn open_program_row(ui: &AppWindow, state: &AppState, data: &SharedDj, index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let program_id = data.lock().programs.get(index).map(|program| program.id);
    let Some(program_id) = program_id.filter(|id| *id > 0) else { return };
    open_program(ui, state, program_id);
}

fn open_program(ui: &AppWindow, state: &AppState, program_id: i64) {
    let global = ui.global::<Dj>();
    global.set_detail_open(true);
    global.set_detail_remote_id(program_id.to_string().into());
    global.set_detail_title(SharedString::default());
    global.set_detail_host(SharedString::default());
    global.set_detail_artwork_path(SharedString::default());
    global.set_detail_duration(SharedString::default());
    global.set_detail_publish_date(SharedString::default());
    global.set_detail_listeners(SharedString::default());
    global.set_detail_description(SharedString::default());
    global.set_detail_song_id(SharedString::default());
    global.set_detail_loading(true);
    global.set_detail_error(SharedString::default());
    fetch::load_program_detail(state, ui.as_weak(), program_id);
}

/// Cancels any detail request in flight as well as closing the page, so its answer cannot reopen
/// a programme the user already left.
fn close_program_detail(ui: &AppWindow) {
    PROGRAM_DETAIL_REQUEST.fetch_add(1, Ordering::Relaxed);
    let global = ui.global::<Dj>();
    global.set_detail_open(false);
    global.set_detail_remote_id(SharedString::default());
    global.set_detail_title(SharedString::default());
    global.set_detail_host(SharedString::default());
    global.set_detail_artwork_path(SharedString::default());
    global.set_detail_duration(SharedString::default());
    global.set_detail_publish_date(SharedString::default());
    global.set_detail_listeners(SharedString::default());
    global.set_detail_description(SharedString::default());
    global.set_detail_song_id(SharedString::default());
    global.set_detail_loading(false);
    global.set_detail_error(SharedString::default());
}

fn wire_playback(ui: &AppWindow, app_state: &AppState) {
    let weak = ui.as_weak();
    let state = app_state.clone();
    ui.global::<Dj>().on_play_program(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(song_id) = parse_cloud_id(ui.global::<Dj>().get_detail_song_id().as_str()) else {
            return;
        };
        play_song(&ui, &state, song_id);
    });
}

/// Queues the programme list around the chosen episode, the same call every cloud surface makes.
/// A selection the visible list does not contain still plays, as a queue of one.
fn play_song(ui: &AppWindow, state: &AppState, song_id: i64) {
    if song_id <= 0 {
        return;
    }
    let rows = ui.global::<Dj>().get_programs();
    let mut ids: Vec<i64> = (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .filter_map(|row| parse_cloud_id(row.remote_id.as_str()))
        .collect();
    if !ids.contains(&song_id) {
        ids.clear();
        ids.push(song_id);
    }
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let (volume, speed) = {
            let player = lock_state(&state.player_state);
            (player.effective_volume(), player.playback_speed)
        };
        if let Err(error) =
            state.cloud_playback.play_queue(&ids, song_id, false, volume, speed).await
        {
            log::warn!("dj::play_song({song_id}): {}", describe(&error));
        }
    });
}

fn empty_programs() -> ModelRc<UiTrackListRow> {
    ModelRc::from(Rc::new(VecModel::default()))
}
