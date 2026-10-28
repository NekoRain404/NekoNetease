//! Pairs a cloud-disk song with the local track that is the same recording.
//!
//! The identity is deliberately strict: a normalised title, a normalised artist set, and a duration
//! within [`DURATION_TOLERANCE_MS`]. A looser match is worse than none here — a playlist sync
//! writes the local row it is handed, so pairing two different recordings puts the wrong track in
//! the user's playlist and the mistake is silent. A miss costs one song the sync reports as
//! unavailable.

use std::collections::HashMap;

use nekonetease_core::entities::cloud::Song;
use nekonetease_core::error::AppResult;
use nekonetease_store::database::DbPool;
use nekonetease_store::database::queries;
use unicode_normalization::UnicodeNormalization;

/// Encoders, masters and taggers disagree about a track's length by a second or two: gapless
/// pregap, encoder delay, a trimmed silence on a compilation. Three seconds absorbs those while
/// still separating a studio cut from a live version of the same song, which is the pair a wrong
/// match would confuse.
pub const DURATION_TOLERANCE_MS: u64 = 3_000;

/// The separator families tags use for several artists. The join NekoNetease itself renders is
/// `" / "`, but a file usually carries whatever its tagger chose.
const ARTIST_SEPARATORS: [char; 4] = ['/', ';', ',', '&'];

/// Written as the forms that appear in tags. A bare `ft` would also cut `fthm`, a bare `feat`
/// would cut `feather`, hence the surrounding spaces.
const FEAT_MARKERS: [&str; 5] = [" feat. ", " feat ", " ft. ", " ft ", " featuring "];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchCandidate {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub duration_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackMatch {
    pub cloud_song_id: i64,
    pub local_track_id: i64,
    pub duration_delta_ms: u64,
}

/// A [`TrackMatch`] plus the local row's text, for a host that names the found copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedMatch {
    pub cloud_song_id: i64,
    pub local_track_id: i64,
    pub title: String,
    pub artist: Option<String>,
}

/// Pairs every cloud song that has a candidate, in the order the songs were given.
pub fn match_songs(songs: &[Song], candidates: &[MatchCandidate]) -> Vec<TrackMatch> {
    let index = CandidateIndex::new(candidates);
    songs.iter().filter_map(|song| index.best(song)).collect()
}

/// [`match_songs`] over the whole local library, which is read in one narrow projection.
pub async fn match_cloud_disk(db: &DbPool, songs: &[Song]) -> AppResult<Vec<TrackMatch>> {
    let candidates = match_candidates(db).await?;
    Ok(match_songs(songs, &candidates))
}

/// [`match_songs_named`] over the whole local library.
pub async fn match_cloud_disk_named(db: &DbPool, songs: &[Song]) -> AppResult<Vec<NamedMatch>> {
    let candidates = match_candidates(db).await?;
    Ok(match_songs_named(songs, &candidates))
}

async fn match_candidates(db: &DbPool) -> AppResult<Vec<MatchCandidate>> {
    Ok(queries::cloud::match_candidates(db)
        .await?
        .into_iter()
        .map(|row| MatchCandidate {
            id: row.id,
            title: row.title,
            artist: row.artist,
            duration_ms: row.duration_ms,
        })
        .collect())
}

/// [`match_songs`], keeping the matched candidate's own title and artist so a surface can name the
/// local copy rather than only its id. The matcher already holds those fields; this adds no read.
pub fn match_songs_named(songs: &[Song], candidates: &[MatchCandidate]) -> Vec<NamedMatch> {
    let by_id: HashMap<i64, &MatchCandidate> =
        candidates.iter().map(|candidate| (candidate.id, candidate)).collect();
    match_songs(songs, candidates)
        .into_iter()
        .filter_map(|found| {
            let candidate = by_id.get(&found.local_track_id)?;
            Some(NamedMatch {
                cloud_song_id: found.cloud_song_id,
                local_track_id: found.local_track_id,
                title: candidate.title.clone(),
                artist: candidate.artist.clone(),
            })
        })
        .collect()
}

struct CandidateIndex<'a> {
    buckets: HashMap<(String, String), Vec<&'a MatchCandidate>>,
}

impl<'a> CandidateIndex<'a> {
    fn new(candidates: &'a [MatchCandidate]) -> Self {
        let mut buckets: HashMap<(String, String), Vec<&'a MatchCandidate>> = HashMap::new();
        for candidate in candidates {
            let key = key_of(&candidate.title, candidate.artist.as_deref().unwrap_or(""));
            buckets.entry(key).or_default().push(candidate);
        }
        Self { buckets }
    }

    /// The closest duration among the bucket, with the lower id breaking a tie so two runs over the
    /// same library answer the same pairing.
    fn best(&self, song: &Song) -> Option<TrackMatch> {
        let key = key_of(&song.name, &song.artist_names());
        let duration_ms = i64::try_from(song.duration_ms).unwrap_or(i64::MAX);
        self.buckets
            .get(&key)?
            .iter()
            .filter_map(|candidate| {
                let delta = candidate.duration_ms.abs_diff(duration_ms);
                (delta <= DURATION_TOLERANCE_MS).then_some((delta, candidate.id))
            })
            .min()
            .map(|(duration_delta_ms, local_track_id)| TrackMatch {
                cloud_song_id: song.id,
                local_track_id,
                duration_delta_ms,
            })
    }
}

fn key_of(title: &str, artists: &str) -> (String, String) {
    (fold(title), artist_key(artists))
}

/// Width, case and runs of whitespace are the three differences that carry no meaning: a
/// full-width parenthesis from a Japanese tag, a title shouted in capitals, a stray double space.
fn fold(value: &str) -> String {
    value.nfkc().collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reduces an artist line to the set of names it names, so `"B / A"`, `"A; B"` and `"A feat. B"`
/// all compare equal. A name that only the separator differs on is not a different artist.
fn artist_key(artists: &str) -> String {
    let mut folded = fold(artists);
    for marker in FEAT_MARKERS {
        folded = folded.replace(marker, " / ");
    }
    let mut names = folded
        .split(ARTIST_SEPARATORS)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    names.join("/")
}

#[cfg(test)]
#[path = "tests/cloud_match_tests.rs"]
mod tests;
