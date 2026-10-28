//! `NetEase` podcast / DJ-radio routes: recommended and hot stations, one station's programmes,
//! one programme's page, and the subscription toggle.
//!
//! The two listing routes answer with different envelopes (`data` for the recommended strip,
//! `djRadios` for the hot page), so each decode accepts both rather than pinning one fork's body.

use serde_json::{Value, json};

use nekonetease_core::entities::dj::{DjProgram, DjProgramPage, DjRadio, DjRadioPage};

use crate::netease::endpoints::{Endpoint, Protocol};
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, number_as_u64};

pub(super) const DJ_RECOMMEND: Endpoint<'static> =
    Endpoint::read("/api/djradio/recommend/v1", Protocol::Weapi);
pub(super) const DJ_HOT: Endpoint<'static> = Endpoint::read("/api/djradio/hot/v1", Protocol::Weapi);
pub(super) const DJ_DETAIL: Endpoint<'static> =
    Endpoint::read("/api/djradio/v2/get", Protocol::Weapi);
pub(super) const DJ_PROGRAMS: Endpoint<'static> =
    Endpoint::read("/api/dj/program/byradio", Protocol::Weapi);
pub(super) const DJ_PROGRAM_DETAIL: Endpoint<'static> =
    Endpoint::read("/api/dj/program/detail", Protocol::Weapi);
pub(super) const DJ_SUBSCRIBE: Endpoint<'static> =
    Endpoint::write("/api/djradio/sub", Protocol::Weapi);
pub(super) const DJ_UNSUBSCRIBE: Endpoint<'static> =
    Endpoint::write("/api/djradio/unsub", Protocol::Weapi);

impl Client {
    /// The stations the discovery strip shows. A missing `data` is a malformed response; an empty
    /// list is a valid one.
    pub async fn dj_recommend(&self) -> Result<Vec<DjRadio>> {
        let body = self.request(DJ_RECOMMEND, json!({})).await?;
        let radios = radio_array(&body).ok_or(Error::InvalidResponse)?;
        decode_ref(radios)
    }

    pub async fn dj_hot(&self, limit: u64, offset: u64) -> Result<DjRadioPage> {
        let body = self.request(DJ_HOT, json!({"limit": limit, "offset": offset})).await?;
        decode_radio_page(&body, offset, limit)
    }

    pub async fn dj_radio_detail(&self, id: i64) -> Result<DjRadio> {
        let body = self.request(DJ_DETAIL, json!({"id": id})).await?;
        decode_radio(&body)
    }

    pub async fn dj_programs(
        &self,
        radio_id: i64,
        limit: u64,
        offset: u64,
        asc: bool,
    ) -> Result<DjProgramPage> {
        let body = self
            .request(
                DJ_PROGRAMS,
                json!({
                    "radioId": radio_id,
                    "limit": limit,
                    "offset": offset,
                    "asc": asc,
                }),
            )
            .await?;
        decode_program_page(&body, offset, limit)
    }

    pub async fn dj_program_detail(&self, id: i64) -> Result<DjProgram> {
        let body = self.request(DJ_PROGRAM_DETAIL, json!({"id": id})).await?;
        decode_program(&body)
    }

    pub async fn dj_set_subscribed(&self, id: i64, subscribed: bool) -> Result<()> {
        let endpoint = if subscribed { DJ_SUBSCRIBE } else { DJ_UNSUBSCRIBE };
        self.request(endpoint, json!({"id": id})).await?;
        Ok(())
    }
}

/// The recommended route answers `data: [...]`, while some forks wrap the same array in a
/// `djRadios` key.
fn radio_array(body: &Value) -> Option<&Value> {
    if let Some(data) = body.get("data") {
        if data.is_array() {
            return Some(data);
        }
        if let Some(radios) = data.get("djRadios").filter(|value| value.is_array()) {
            return Some(radios);
        }
    }
    body.get("djRadios")
        .filter(|value| value.is_array())
        .or_else(|| body.get("radios").filter(|value| value.is_array()))
}

fn decode_radio_page(body: &Value, offset: u64, limit: u64) -> Result<DjRadioPage> {
    let radios = radio_array(body).ok_or(Error::InvalidResponse)?;
    let radios: Vec<DjRadio> = decode_ref(radios)?;
    let more = page_has_more(body, offset, limit, radios.len());
    Ok(DjRadioPage { radios, more })
}

fn decode_radio(body: &Value) -> Result<DjRadio> {
    let radio = body
        .get("data")
        .filter(|value| value.is_object())
        .or_else(|| body.get("djRadio").filter(|value| value.is_object()))
        .ok_or(Error::InvalidResponse)?;
    // `data` is the station itself on `v2/get`; a fork that wraps it keeps the same object one
    // level down.
    let radio = radio.get("djRadio").filter(|value| value.is_object()).unwrap_or(radio);
    decode_ref(radio)
}

fn decode_program_page(body: &Value, offset: u64, limit: u64) -> Result<DjProgramPage> {
    let programs = body
        .get("programs")
        .or_else(|| body.get("data").and_then(|data| data.get("programs")))
        .filter(|value| value.is_array())
        .ok_or(Error::InvalidResponse)?;
    let programs: Vec<DjProgram> = decode_ref(programs)?;
    let more = program_page_has_more(body, offset, limit, programs.len());
    Ok(DjProgramPage { programs, more })
}

fn decode_program(body: &Value) -> Result<DjProgram> {
    let program = body
        .get("program")
        .filter(|value| value.is_object())
        .or_else(|| body.get("data").and_then(|data| data.get("program")))
        .or_else(|| body.get("data").filter(|value| value.is_object()))
        .ok_or(Error::InvalidResponse)?;
    decode_ref(program)
}

fn page_has_more(body: &Value, offset: u64, limit: u64, len: usize) -> bool {
    if let Some(more) = flag(body, "more").or_else(|| flag(body, "hasMore")) {
        return more;
    }
    total_exceeds(body, offset, len)
        .or_else(|| {
            let total = number_as_u64(body.get("total")?)?;
            Some(offset.saturating_add(u64_of(len)) < total)
        })
        .unwrap_or_else(|| u64_of(len) >= limit)
}

fn program_page_has_more(body: &Value, offset: u64, limit: u64, len: usize) -> bool {
    if let Some(more) = flag(body, "more").or_else(|| flag(body, "hasMore")) {
        return more;
    }
    let data = body.get("data").unwrap_or(body);
    total_exceeds(data, offset, len)
        .or_else(|| {
            let total = number_as_u64(data.get("count")?)?;
            Some(offset.saturating_add(u64_of(len)) < total)
        })
        .unwrap_or_else(|| u64_of(len) >= limit)
}

fn total_exceeds(container: &Value, offset: u64, len: usize) -> Option<bool> {
    let total = number_as_u64(container.get("count")?)?;
    Some(offset.saturating_add(u64_of(len)) < total)
}

fn flag(container: &Value, key: &str) -> Option<bool> {
    container.get(key).and_then(Value::as_bool)
}

fn u64_of(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "../tests/dj_tests.rs"]
mod tests;
