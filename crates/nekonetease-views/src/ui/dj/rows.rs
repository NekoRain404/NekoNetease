//! Boundary projections for the podcast page: stations become card rows, programmes become the
//! same track rows every cloud table draws.

use slint::SharedString;

use nekonetease_core::entities::cloud::Song;
use nekonetease_core::entities::dj::{DjProgram, DjRadio};
use nekonetease_core::entities::track::TrackListRow as RsTrackListRow;
use nekonetease_ui::{EntityStripRow as UiEntityStripRow, TrackListRow as UiTrackListRow};

/// The strip's `id` is the row index, not the station id: it is an `int` while the wire's ids
/// are 64-bit, and Rust keeps the ordered station list that index addresses.
pub(super) fn radio_rows(radios: &[DjRadio]) -> Vec<UiEntityStripRow> {
    radios
        .iter()
        .enumerate()
        .map(|(index, radio)| UiEntityStripRow {
            id: i32::try_from(index).unwrap_or(i32::MAX),
            title: radio.name.clone().into(),
            subtitle: radio_subtitle(radio).into(),
            artwork_path: remote_artwork(radio.cover_url.as_deref()).into(),
            play_count: i32::try_from(radio.play_count).unwrap_or(i32::MAX),
            is_favorite: radio.subscribed,
        })
        .collect()
}

fn radio_subtitle(radio: &DjRadio) -> String {
    let category = radio.category.trim();
    let host = radio.host_name().trim();
    match (category.is_empty(), host.is_empty()) {
        (false, false) => format!("{category} · {host}"),
        (false, true) => category.to_owned(),
        (true, false) => host.to_owned(),
        (true, true) => String::new(),
    }
}

/// A programme's playable identity is its `mainSong`. A programme without one keeps its row and
/// stays clickable, because the row opens the detail that explains the absence; only its playable
/// remote id is empty.
pub(super) fn program_rows(programs: &[DjProgram]) -> Vec<UiTrackListRow> {
    let rows: Vec<RsTrackListRow> = programs
        .iter()
        .enumerate()
        .map(|(index, program)| {
            let song_id = program.song_id().unwrap_or(0);
            let artist = program_host(program);
            RsTrackListRow {
                id: song_id,
                file_path: String::new(),
                file_name: program_name(program).to_owned(),
                title: program_name(program).to_owned(),
                artist: (!artist.is_empty()).then(|| artist.to_owned()),
                album_artist: None,
                album: program
                    .radio
                    .as_ref()
                    .map(|radio| radio.name.clone())
                    .filter(|name| !name.is_empty()),
                genre: None,
                track_number: Some(episode_number(program, index)),
                disc_number: Some(1),
                year: None,
                duration_ms: i64::try_from(program_duration_ms(program)).unwrap_or(i64::MAX),
                artwork_path: {
                    let artwork = remote_artwork(program.artwork_url());
                    (!artwork.is_empty()).then_some(artwork)
                },
                is_favorite: false,
                rating: 0,
                album_id: None,
                artist_id: None,
                genre_id: None,
                date_added: String::new(),
                sort_key: None,
                mv_id: program.main_song.as_ref().and_then(Song::mv_id),
            }
        })
        .collect();

    let mut rows = crate::ui::tracks::to_slint_track_list_rows(&rows);
    for (row, program) in rows.iter_mut().zip(programs) {
        if program.song_id().is_none() {
            row.remote_id = SharedString::default();
        }
    }
    rows
}

pub(super) fn program_name(program: &DjProgram) -> &str {
    if !program.name.is_empty() {
        return &program.name;
    }
    program.main_song.as_ref().map_or("", |song| song.name.as_str())
}

pub(super) fn program_duration_ms(program: &DjProgram) -> u64 {
    if program.duration_ms > 0 {
        return program.duration_ms;
    }
    program.main_song.as_ref().map_or(0, |song| song.duration_ms)
}

pub(super) fn program_host(program: &DjProgram) -> &str {
    let host = program.host_name().trim();
    if !host.is_empty() {
        return host;
    }
    program.radio.as_ref().map_or("", |radio| radio.name.as_str())
}

fn episode_number(program: &DjProgram, index: usize) -> i32 {
    let number = if program.serial_num == 0 {
        u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1)
    } else {
        program.serial_num
    };
    i32::try_from(number).unwrap_or(i32::MAX)
}

pub(super) fn remote_artwork(url: Option<&str>) -> String {
    url.filter(|url| crate::ui::remote_covers::is_remote_artwork_url(url))
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn duration_text(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    crate::ui::tracks::format_duration_ms(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// The wire sometimes carries `0` for a publish time the uploader hid.
pub(super) fn publish_date(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|at| at.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}
