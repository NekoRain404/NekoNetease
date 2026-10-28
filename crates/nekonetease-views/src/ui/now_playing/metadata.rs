//! Technical-metadata chip row formatter + display helpers.

use crate::ui::util::{format_channels, format_sample_rate, opt_shared};
use nekonetease_core::entities::track::{TrackMeta, TrackSummary};
use nekonetease_ui::TrackMetaRow;
use slint::SharedString;

/// Format the `TrackMeta` projection into the pre-formatted display
/// strings the `TrackMetaRow` chip row reads. `""` for any absent field —
/// the view gates each chip on `field != ""`.
pub(super) fn to_slint_track_meta(t: &TrackMeta) -> TrackMetaRow {
    TrackMetaRow {
        track_id: i32::try_from(t.id).unwrap_or(i32::MAX),
        codec: opt_shared(t.codec.as_deref().map(str::to_uppercase)),
        bitrate: opt_shared(t.bitrate.map(|b| format!("{b} kbps"))),
        sample_rate: opt_shared(t.sample_rate.map(format_sample_rate)),
        bit_depth: opt_shared(t.bit_depth.map(|d| format!("{d}-bit"))),
        channels: opt_shared(t.channels.map(format_channels)),
        year: opt_shared(t.year.filter(|y| *y > 0).map(|y| y.to_string())),
        genre: opt_shared(t.genre.as_deref()),
    }
}

/// The chip row for a cloud stream, built from the endpoint's own answer — there is no file to
/// read one off. Fields the endpoint left unsaid stay `""`, so the row states a format and a
/// measured rate and nothing it would have to guess.
pub(super) fn to_slint_stream_meta(track: &TrackSummary) -> TrackMetaRow {
    let track_id = i32::try_from(track.id).unwrap_or(i32::MAX);
    let Some(stream) = track.stream.as_ref() else {
        return TrackMetaRow { track_id, ..TrackMetaRow::default() };
    };
    TrackMetaRow {
        track_id,
        codec: stream.format.to_uppercase().into(),
        bitrate: average_bitrate(stream.size_bytes, track.duration_ms),
        ..TrackMetaRow::default()
    }
}

/// Payload rate over the track's own length. The endpoint's `level` names a request tier rather
/// than an answer, so the size it reports and the length it reports are the only honest pair;
/// `size * 8 / duration_ms` is already kbit/s, `duration_ms` being milliseconds.
fn average_bitrate(size_bytes: Option<u64>, duration_ms: i64) -> SharedString {
    let (Some(size), duration_ms) = (size_bytes, u64::try_from(duration_ms).unwrap_or(0)) else {
        return SharedString::new();
    };
    if duration_ms == 0 {
        return SharedString::new();
    }
    format!("{} kbps", size.saturating_mul(8) / duration_ms).into()
}

/// Walks `TrackMetaRow` fields in the same order the declarative chip block
/// in `now-playing-view.slint` declared them and returns the non-empty
/// texts. Used both to seed the chip shadow on track-meta change and to
/// re-chunk on width changes without re-reading the global.
pub(super) fn visible_chip_texts(m: &TrackMetaRow) -> Vec<SharedString> {
    let fields =
        [&m.codec, &m.bitrate, &m.sample_rate, &m.bit_depth, &m.channels, &m.year, &m.genre];
    let mut out = Vec::with_capacity(fields.len());
    for s in fields {
        if !s.is_empty() {
            out.push(s.clone());
        }
    }
    out
}

// The wrap itself lives in `crate::ui::chips` — shared with the hero bands, so
// both strips break the same way and only the row cap differs.

#[cfg(test)]
#[path = "tests/metadata_tests.rs"]
mod tests;
