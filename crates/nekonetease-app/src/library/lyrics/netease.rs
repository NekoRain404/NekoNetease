//! Lyrics returned by `NetEase` for cloud tracks.

use std::collections::HashMap;

use super::lrc;
use crate::state::AppState;
use nekonetease_core::entities::cloud::Lyrics as CloudLyrics;
use nekonetease_core::entities::lyrics::{LyricLine, Lyrics, LyricsOutcome, LyricsSource};
use nekonetease_core::entities::track::TrackSummary;

pub(super) async fn look_up(state: &AppState, track: &TrackSummary) -> LyricsOutcome {
    let response = match state.cloud.lyrics(track.id).await {
        Ok(response) => response,
        Err(error) => {
            log::debug!(
                "netease lyrics for {}: {}",
                track.id,
                nekonetease_core::error::describe(&error)
            );
            return LyricsOutcome::Unavailable;
        }
    };

    parse_response(&response)
}

fn parse_response(response: &CloudLyrics) -> LyricsOutcome {
    let Some(mut sheet) = lrc::parse(&response.original, LyricsSource::Online) else {
        // Some songs have no original text but do have a translation. Showing that is still more
        // useful than an empty panel.
        return lrc::parse(&response.translation, LyricsSource::Online)
            .map_or(LyricsOutcome::Absent, LyricsOutcome::Sheet);
    };

    attach_parallel(&mut sheet.lines, &response.translation, |line, value| {
        line.translation = Some(value);
    });
    attach_parallel(&mut sheet.lines, &response.romanization, |line, value| {
        line.romanization = Some(value);
    });
    Lyrics::new(sheet.lines, LyricsSource::Online)
        .map_or(LyricsOutcome::Absent, LyricsOutcome::Sheet)
}

/// Attach a parallel LRC by timestamp, falling back to row order for plain-text responses. Exact
/// timestamps are important: translations can omit instrumental gaps that remain in the original.
fn attach_parallel(
    target: &mut [LyricLine],
    parallel_text: &str,
    mut attach: impl FnMut(&mut LyricLine, String),
) {
    let Some(parallel) = lrc::parse(parallel_text, LyricsSource::Online) else { return };
    let by_time: HashMap<i64, &str> =
        parallel.lines.iter().filter_map(|line| Some((line.at_ms?, line.text.as_str()))).collect();
    let timed_parallel = !by_time.is_empty();
    let mut index = 0_usize;

    for line in target {
        let value = if timed_parallel {
            line.at_ms.and_then(|at| by_time.get(&at).copied())
        } else {
            let value = parallel.lines.get(index).map(|row| row.text.as_str());
            index = index.saturating_add(1);
            value
        };
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            attach(line, value.to_owned());
        }
    }
}

#[cfg(test)]
#[path = "tests/netease_tests.rs"]
mod tests;
