use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::netease::{Error, Result};

pub(super) fn number_as_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

pub(super) fn number_as_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

pub(super) fn decode<T: DeserializeOwned>(body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|error| invalid(&error))
}

/// The wire error says which field or shape disagreed, and the typed error says nothing. It costs
/// one `log::debug!` to keep the cause reachable when a response stops matching this tree.
fn invalid(error: &serde_json::Error) -> Error {
    log::debug!("netease decode: {}", nekonetease_core::error::describe(error));
    Error::InvalidResponse
}

// Deserializing a borrowed Value copies only fields the DTO keeps; cloning the DOM first also
// copies every ignored track, privilege and metadata subtree in large API responses.
pub(super) fn decode_ref<T: DeserializeOwned>(body: &Value) -> Result<T> {
    T::deserialize(body).map_err(|error| invalid(&error))
}

pub(super) fn field<T: DeserializeOwned>(mut body: Value, name: &str) -> Result<T> {
    decode(body.get_mut(name).ok_or(Error::InvalidResponse)?.take())
}
