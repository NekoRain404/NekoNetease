//! The signed-in account's uploaded cloud-disk songs.

use nekonetease_core::entities::cloud::{CloudDiskPage, Song};

use serde_json::{Value, json};

use crate::netease::endpoints as ep;
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, number_as_u64};

impl Client {
    /// Fetches one page of the account's cloud disk. The endpoint pages by offset and reports the
    /// whole library's count, so a caller walks the pages until their sum reaches `total`.
    pub async fn cloud_disk_songs(&self, offset: u64, limit: u64) -> Result<CloudDiskPage> {
        let body = self.request(ep::CLOUD_DISK, json!({"limit": limit, "offset": offset})).await?;
        decode_cloud_disk_page(&body)
    }
}

/// Each entry wraps the recording under `simpleSong`; the outer object carries the upload's own
/// metadata, none of which a track row draws.
pub(super) fn decode_cloud_disk_page(body: &Value) -> Result<CloudDiskPage> {
    let entries = body["data"].as_array().ok_or(Error::InvalidResponse)?;
    let mut songs = Vec::with_capacity(entries.len());
    for entry in entries {
        let song = entry.get("simpleSong").unwrap_or(entry);
        songs.push(decode_ref::<Song>(song)?);
    }
    let total = number_as_u64(&body["count"]).unwrap_or(songs.len() as u64);
    Ok(CloudDiskPage { songs, total })
}
