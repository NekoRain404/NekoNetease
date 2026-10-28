//! The four requests behind the podcast page. Each publishes only while its generation and its
//! rendered remote id still stand, so a late answer cannot paint over a page opened since.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Dj, TrackListRow as UiTrackListRow};

use super::rows;
use super::{
    PROGRAM_DETAIL_REQUEST, PROGRAM_LIST_REQUEST, RADIO_DETAIL_REQUEST, RADIO_LIST_REQUEST,
    SharedDj,
};

/// Which slice of a station's programmes the caller is asking for.
#[derive(Clone, Copy)]
pub(super) enum ProgramsPage {
    First,
    More { offset: u64 },
}

#[derive(Clone, Copy)]
pub(super) struct ProgramsRequest {
    pub radio_id: i64,
    pub page: ProgramsPage,
}

pub(super) fn load_radios(state: &AppState, weak: slint::Weak<AppWindow>, data: SharedDj) {
    let request = RADIO_LIST_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.dj_recommend().await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if RADIO_LIST_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let global = ui.global::<Dj>();
            global.set_radios_loading(false);
            match result {
                Ok(radios) => {
                    let rows = rows::radio_rows(&radios);
                    global.set_radios(ModelRc::from(Rc::new(VecModel::from(rows))));
                    global.set_radios_error(SharedString::default());
                    data.lock().radios = radios;
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!("dj::load_radios: {message}");
                    global.set_radios_error(message.into());
                }
            }
        });
    });
}

pub(super) fn load_radio_detail(state: &AppState, weak: slint::Weak<AppWindow>, radio_id: i64) {
    let request = RADIO_DETAIL_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let remote_id = radio_id.to_string();
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.dj_radio(radio_id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if RADIO_DETAIL_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let global = ui.global::<Dj>();
            if global.get_radio_remote_id().as_str() != remote_id {
                return;
            }
            global.set_radio_loading(false);
            match result {
                Ok(radio) => {
                    global.set_radio_title(radio.name.clone().into());
                    global.set_radio_host(radio.host_name().into());
                    global.set_radio_artwork_path(
                        rows::remote_artwork(radio.cover_url.as_deref()).into(),
                    );
                    global.set_radio_description(radio.description.clone().into());
                    global.set_radio_subscribed(radio.subscribed);
                    global.set_radio_error(SharedString::default());
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!("dj::load_radio_detail({radio_id}): {message}");
                    global.set_radio_error(message.into());
                }
            }
        });
    });
}

pub(super) fn load_programs(
    state: &AppState,
    weak: slint::Weak<AppWindow>,
    request: ProgramsRequest,
    data: SharedDj,
) {
    let request_id = PROGRAM_LIST_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let remote_id = request.radio_id.to_string();
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let offset = match request.page {
            ProgramsPage::First => 0,
            ProgramsPage::More { offset } => offset,
        };
        let generation = state.cloud.status().generation;
        let result = state.cloud.dj_programs_page(request.radio_id, offset).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if PROGRAM_LIST_REQUEST.load(Ordering::Relaxed) != request_id
                || state.cloud.status().generation != generation
            {
                return;
            }
            let global = ui.global::<Dj>();
            if global.get_radio_remote_id().as_str() != remote_id {
                return;
            }
            global.set_programs_loading(false);
            global.set_programs_loading_more(false);
            match result {
                Ok(page) => {
                    let mut rows = match request.page {
                        ProgramsPage::First => Vec::new(),
                        ProgramsPage::More { .. } => programs_rows(&global.get_programs()),
                    };
                    rows.extend(rows::program_rows(&page.programs));
                    global.set_programs(ModelRc::from(Rc::new(VecModel::from(rows))));
                    // A response that drew nothing cannot advance the offset, so asking again
                    // would fetch the same page forever.
                    global.set_programs_has_more(page.more && !page.programs.is_empty());
                    global.set_programs_error(SharedString::default());
                    // The row carries the playable `mainSong` id, so the detail endpoint's
                    // programme id has to be kept beside the model in the same order.
                    let mut stored = data.lock();
                    if matches!(request.page, ProgramsPage::First) {
                        stored.programs.clear();
                    }
                    stored.programs.extend(page.programs);
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!(
                        "dj::load_programs({}, offset {offset}): {message}",
                        request.radio_id
                    );
                    global.set_programs_error(message.into());
                }
            }
        });
    });
}

fn programs_rows(model: &ModelRc<UiTrackListRow>) -> Vec<UiTrackListRow> {
    (0..model.row_count()).filter_map(|index| model.row_data(index)).collect()
}

pub(super) fn load_program_detail(state: &AppState, weak: slint::Weak<AppWindow>, program_id: i64) {
    let request = PROGRAM_DETAIL_REQUEST.fetch_add(1, Ordering::Relaxed) + 1;
    let remote_id = program_id.to_string();
    let state = state.clone();
    state.runtime.clone().spawn(async move {
        let generation = state.cloud.status().generation;
        let result = state.cloud.dj_program(program_id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if PROGRAM_DETAIL_REQUEST.load(Ordering::Relaxed) != request
                || state.cloud.status().generation != generation
            {
                return;
            }
            let global = ui.global::<Dj>();
            if !global.get_detail_open() || global.get_detail_remote_id().as_str() != remote_id {
                return;
            }
            global.set_detail_loading(false);
            match result {
                Ok(program) => {
                    global.set_detail_title(rows::program_name(&program).into());
                    global.set_detail_host(rows::program_host(&program).into());
                    global.set_detail_artwork_path(
                        rows::remote_artwork(program.artwork_url()).into(),
                    );
                    global.set_detail_duration(
                        rows::duration_text(rows::program_duration_ms(&program)).into(),
                    );
                    global
                        .set_detail_publish_date(rows::publish_date(program.create_time_ms).into());
                    let listeners = if program.listener_count > 0 {
                        program.listener_count.to_string()
                    } else {
                        String::new()
                    };
                    global.set_detail_listeners(listeners.into());
                    global.set_detail_description(program.description.clone().into());
                    global.set_detail_song_id(
                        program.song_id().map(|id| id.to_string()).unwrap_or_default().into(),
                    );
                    global.set_detail_error(SharedString::default());
                }
                Err(error) => {
                    let message = describe(&error);
                    log::warn!("dj::load_program_detail({program_id}): {message}");
                    global.set_detail_error(message.into());
                }
            }
        });
    });
}
