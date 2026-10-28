//! The chart catalogue.
//!
//! `/api/toplist/detail` answers every chart in one list, despite the name: it takes no chart id
//! and no offset, so a chart's own tracks come from [`Client::playlist`], which is the resource a
//! chart is.

use serde_json::{Value, json};

use nekonetease_core::entities::cloud::Playlist;

use super::Client;
use super::decode::decode_ref;
use crate::netease::{Error, Result};

const TOPLIST_DETAIL: &str = "/api/toplist/detail";

impl Client {
    /// Returns every published chart in the server's order, tags included.
    pub async fn chart_catalogue(&self) -> Result<Vec<Playlist>> {
        decode_chart_catalogue(&self.request_path(TOPLIST_DETAIL, json!({})).await?)
    }
}

pub(super) fn decode_chart_catalogue(body: &Value) -> Result<Vec<Playlist>> {
    decode_ref(body.get("list").ok_or(Error::InvalidResponse)?)
}
