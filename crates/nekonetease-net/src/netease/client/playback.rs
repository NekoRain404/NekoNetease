use serde_json::{Value, json};

use nekonetease_core::entities::cloud::{
    Lyrics, PlaybackRestriction, Playlist, Quality, Song, StreamTicket, UnavailableReason,
};

use crate::netease::endpoints::{self as ep};
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, field};

impl Client {
    pub async fn stream(&self, id: i64, quality: Quality) -> Result<StreamTicket> {
        // `STREAM_V1` is what the current clients call; the legacy route answers with the same
        // `data[]` row shape, so the decoder below is shared. A session XEAPI refuses (a guest
        // without its bootstrap, say) still gets a ticket through the fallback.
        let body = match self
            .request(
                ep::STREAM_V1,
                json!({"ids":format!("[{id}]"),"level":quality.level(),"encodeType":"flac"}),
            )
            .await
        {
            Ok(body) => body,
            Err(
                Error::Unavailable | Error::Transport | Error::Timeout | Error::InvalidResponse,
            ) => {
                self.request(ep::STREAM, json!({"ids":format!("[{id}]"),"br":quality.bitrate()}))
                    .await?
            }
            Err(error) => return Err(error),
        };
        let row = body["data"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"].as_i64() == Some(id)))
            .ok_or(Error::Unavailable)?;
        // A preview still has a URL, so the trial verdict rides beside playback. A refused
        // recording has none, and `fee` is the only reason its row carries — which is why the fee
        // is read only on that arm: a paying account's VIP track also reports `fee: 1` while
        // handing out a perfectly ordinary URL. The refusal is a ticket rather than an error so
        // the caller can still classify it against the song detail, which is where a missing
        // licence is visible.
        let restriction = trial_restriction(row);
        let Some(url) = row["url"].as_str().filter(|u| !u.is_empty()) else {
            return Ok(refused_ticket(id, row));
        };
        let url = crate::services::net::http_url(url).ok_or(Error::InvalidResponse)?.to_string();
        Ok(StreamTicket {
            song_id: id,
            url,
            format: row["type"].as_str().unwrap_or("mp3").to_owned(),
            expires_after_seconds: row["expi"].as_u64().unwrap_or(600),
            trial: matches!(restriction, Some(PlaybackRestriction::Trial { .. })),
            size: row["size"].as_u64(),
            level: row["level"].as_str().unwrap_or_default().to_owned(),
            restriction,
        })
    }

    pub async fn lyrics(&self, id: i64) -> Result<Lyrics> {
        let body = self
            .request(ep::LYRIC, json!({"id":id,"tv":-1,"lv":-1,"rv":-1,"kv":-1,"_nmclfl":1}))
            .await?;
        Ok(Lyrics {
            original: body["lrc"]["lyric"].as_str().unwrap_or_default().to_owned(),
            translation: body["tlyric"]["lyric"].as_str().unwrap_or_default().to_owned(),
            romanization: body["romalrc"]["lyric"].as_str().unwrap_or_default().to_owned(),
        })
    }

    pub async fn daily(&self) -> Result<Vec<Song>> {
        let body = self.request(ep::DAILY, json!({"afresh":false})).await?;
        decode_ref(&body["data"]["dailySongs"])
    }

    pub async fn recommendations(&self) -> Result<Vec<Playlist>> {
        field(self.request(ep::RECOMMENDATIONS, json!({})).await?, "recommend")
    }

    pub async fn intelligence(
        &self,
        song_id: i64,
        playlist_id: i64,
        start_music_id: i64,
    ) -> Result<Vec<Song>> {
        let body = self
            .request(
                ep::INTELLIGENCE,
                json!({
                    "songId": song_id, "playlistId": playlist_id, "startMusicId": start_music_id,
                    "type": "fromPlayOne", "count": 20
                }),
            )
            .await?;
        decode_intelligence_songs(&body)
    }

    pub async fn similar_songs(&self, song_id: i64) -> Result<Vec<Song>> {
        field(
            self.request(ep::SIMILAR_SONGS, json!({"songid": song_id, "limit": 30, "offset": 0}))
                .await?,
            "songs",
        )
    }
}

/// The preview window a stream row announces, if any. `freeTrialInfo` is the only field that
/// describes a playable restriction; `fee` classifies a refusal instead.
pub(super) fn trial_restriction(row: &Value) -> Option<PlaybackRestriction> {
    let info = row.get("freeTrialInfo").filter(|value| !value.is_null())?;
    let start = info["start"].as_u64().unwrap_or_default();
    let end = info["end"].as_u64().unwrap_or_default();
    Some(PlaybackRestriction::Trial { seconds: end.saturating_sub(start) })
}

/// A refused row, kept in the ticket's shape so the caller can refine the verdict. The format,
/// expiry and size fields are still the row's own where it carries them.
fn refused_ticket(id: i64, row: &Value) -> StreamTicket {
    StreamTicket {
        song_id: id,
        url: String::new(),
        format: row["type"].as_str().unwrap_or("mp3").to_owned(),
        expires_after_seconds: row["expi"].as_u64().unwrap_or(600),
        trial: false,
        size: row["size"].as_u64(),
        level: row["level"].as_str().unwrap_or_default().to_owned(),
        restriction: Some(refusal_verdict(row)),
    }
}

/// Why a stream row carried no URL. Fee codes 1 (membership) and 4 (album purchase) are the two
/// the service documents; every other refusal is left unclassified, the song detail being the only
/// place a missing licence or a withdrawn recording shows.
pub(super) fn refusal_verdict(row: &Value) -> PlaybackRestriction {
    match row["fee"].as_i64().unwrap_or_default() {
        1 => PlaybackRestriction::VipRequired,
        4 => PlaybackRestriction::AlbumPurchaseRequired,
        _ => PlaybackRestriction::Unavailable { reason: UnavailableReason::Unknown },
    }
}

pub(super) fn decode_intelligence_songs(body: &Value) -> Result<Vec<Song>> {
    let entries = body["data"].as_array().ok_or(Error::InvalidResponse)?;
    entries.iter().map(|entry| decode_ref(&entry["songInfo"])).collect()
}
