//! The song wiki's "About this song" page.
//!
//! The page is a block tree rather than a record: one block holds every editorial module, and
//! the prose lives two levels down in a resource's `uiElement.descriptions`. Decoding keeps only
//! what the About panel shows, so a module added upstream costs nothing here.

use serde_json::{Value, json};

use crate::netease::Result;
use crate::netease::endpoints::{self as ep};
use nekonetease_core::entities::song_about::SongAbout;

use super::Client;

/// The block carrying the facts and the review. Other blocks are memories, similar songs and
/// playlists, none of which describe the recording itself.
const ABOUT_BLOCK: &str = "SONG_PLAY_ABOUT_SONG_BASIC";

/// The creative holding review prose. Its `descriptions` are the paragraphs, where every other
/// type's are captions for a tag or a sheet.
const REVIEW_CREATIVE: &str = "songComment";

/// The creative types stating an attribute of the recording. `sheet` links to sheet music and
/// `memoryGrid` holds listeners' memories, so neither belongs in an About panel.
const FACT_CREATIVES: [&str; 6] =
    ["songTag", "songBizTag", "language", "bpm", "songAward", "entertainment"];

impl Client {
    /// The recording's wiki page. Empty rather than an error when the service has no entry.
    pub async fn song_about(&self, song_id: i64) -> Result<SongAbout> {
        let body = self.request(ep::SONG_WIKI_PAGE, json!({ "songId": song_id })).await?;
        Ok(parse_about(&body))
    }
}

pub(super) fn parse_about(body: &Value) -> SongAbout {
    let mut about = SongAbout::default();
    let blocks = body["data"]["blocks"].as_array().into_iter().flatten();
    for block in blocks.filter(|block| block["code"].as_str() == Some(ABOUT_BLOCK)) {
        let creatives = block["creatives"].as_array().into_iter().flatten();
        for creative in creatives {
            let kind = creative["creativeType"].as_str().unwrap_or_default();
            if kind.eq_ignore_ascii_case(REVIEW_CREATIVE) {
                push_descriptions(&creative["uiElement"], &mut about.reviews);
                for resource in resources(creative) {
                    push_descriptions(&resource["uiElement"], &mut about.reviews);
                }
            } else if FACT_CREATIVES.iter().any(|fact| fact.eq_ignore_ascii_case(kind)) {
                push_fact_texts(&creative["uiElement"], &mut about.facts);
                for resource in resources(creative) {
                    push_fact_texts(&resource["uiElement"], &mut about.facts);
                }
            }
        }
    }
    about
}

fn resources(creative: &Value) -> impl Iterator<Item = &Value> {
    creative["resources"].as_array().into_iter().flatten()
}

fn push_descriptions(element: &Value, into: &mut Vec<String>) {
    let entries = element["descriptions"].as_array().into_iter().flatten();
    for entry in entries {
        push_unique(into, entry["description"].as_str());
    }
}

/// A fact's words are spread over three shapes by creative type: a tag's `mainTitle`, a
/// language's `textLinks`, and an award's title with its year in `subTitles`.
fn push_fact_texts(element: &Value, into: &mut Vec<String>) {
    push_unique(into, element["mainTitle"]["title"].as_str());
    let subtitles = element["subTitles"].as_array().into_iter().flatten();
    for subtitle in subtitles {
        push_unique(into, subtitle["title"].as_str());
    }
    let links = element["textLinks"].as_array().into_iter().flatten();
    for link in links {
        push_unique(into, link["text"].as_str());
    }
}

fn push_unique(into: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|text| !text.is_empty()) else { return };
    if !into.iter().any(|held| held == value) {
        into.push(value.to_owned());
    }
}
