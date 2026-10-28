//! 云贝 balance, the daily check-in, and the account's signed-in devices.
//!
//! None of these routes is in `docs/netease-api.md`. The two 云贝 routes are pinned from
//! api-enhanced's `module/yunbei_info.js` and `module/yunbei_sign.js`, and chaunsin's Go SDK
//! (`YunBeiUserInfo`, `YunBeiSignIn`); the SDK also names the check-in's success contract — a
//! repeated claim answers `code: 200` with `data.sign: false`.
//!
//! The device routes are pinned from api-enhanced's `module/device_list.js` and
//! `module/device_kickoff.js`, and independently from oceanus's `docs/NCM_API.md`. Scopify's
//! `types/api/devices.ts` supplies the response shape: `data.userDeviceList[]` with
//! `deviceKey`, `deviceName`, `os`, `ifLoginDevice` and `lastActiveTime`, plus
//! `data.userDeviceLimitInfoDTO` for the seat count this card does not draw.

use serde::Deserialize;
use serde_json::{Value, json};

use nekonetease_core::entities::cloud::CloudBeans;

use crate::netease::Result;
use crate::netease::endpoints::{self as ep, Protocol};

use super::{Client, decode};

/// The device list is EAPI and carries the login state in `ifLoginDevice`; the key is a string
/// on current clients and a number on older rows, so both are accepted.
pub(super) const DEVICE_LIST: &str = "/api/middle/user/device/list";
/// Revoking a device takes the same key the list returns. The route also accepts a safety
/// captcha; this client sends the empty value the community clients use and reports any
/// verification the server asks for as a typed error.
pub(super) const DEVICE_KICKOFF: &str = "/api/middle/user/security/device/kickoff";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub key: String,
    pub name: String,
    pub os: String,
    pub place: String,
    pub last_active_ms: i64,
    pub current: bool,
}

#[derive(Deserialize, Default)]
struct UserInfo {
    #[serde(default, rename = "userPoint")]
    user_point: UserPoint,
    #[serde(default, rename = "mobileSign")]
    mobile_sign: bool,
    #[serde(default, rename = "pcSign")]
    pc_sign: bool,
}

#[derive(Deserialize, Default)]
struct UserPoint {
    #[serde(default)]
    balance: i64,
}

#[derive(Deserialize)]
struct CheckIn {
    #[serde(default)]
    sign: bool,
}

impl Client {
    /// Reads the signed-in account's 云贝 balance and whether today's check-in is already claimed.
    pub async fn cloud_beans(&self) -> Result<CloudBeans> {
        let body = self.request(ep::CLOUD_BEANS, json!({})).await?;
        decode_beans(body)
    }

    /// Claims today's 云贝 check-in. `false` means the server reports it was already claimed.
    pub async fn check_in(&self) -> Result<bool> {
        let body = self.request(ep::CLOUD_CHECK_IN, json!({})).await?;
        decode_check_in(body)
    }

    /// Lists the account's signed-in devices, including the one this client is using.
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let body = self.request_at(DEVICE_LIST, Protocol::Eapi, json!({"excStatus": "9"})).await?;
        Ok(decode_devices(&body))
    }

    /// Signs the device named by `key` out of the account.
    pub async fn revoke_device(&self, key: &str) -> Result<()> {
        self.request_at_write(DEVICE_KICKOFF, Protocol::Eapi, json!({"key": key, "captcha": ""}))
            .await?;
        Ok(())
    }
}

pub(super) fn decode_beans(body: Value) -> Result<CloudBeans> {
    let info: UserInfo = decode::decode(body)?;
    Ok(CloudBeans {
        // The mobile and desktop clients each keep their own flag; either means the day is done.
        signed_today: info.mobile_sign || info.pc_sign,
        balance: u64::try_from(info.user_point.balance).unwrap_or_default(),
    })
}

pub(super) fn decode_check_in(body: Value) -> Result<bool> {
    Ok(decode::field::<CheckIn>(body, "data")?.sign)
}

pub(super) fn decode_devices(body: &Value) -> Vec<DeviceInfo> {
    let Some(rows) =
        body.get("data").and_then(|data| data.get("userDeviceList")).and_then(Value::as_array)
    else {
        return Vec::new();
    };
    rows.iter().filter_map(device_info).collect()
}

fn device_info(row: &Value) -> Option<DeviceInfo> {
    let key = match row.get("deviceKey")? {
        Value::String(key) => key.trim().to_owned(),
        Value::Number(key) => key.to_string(),
        _ => return None,
    };
    if key.is_empty() {
        return None;
    }
    let place = text(row, "loginPlace");
    Some(DeviceInfo {
        key,
        name: text(row, "deviceName"),
        os: text(row, "os"),
        place: if place.is_empty() { text(row, "region") } else { place },
        last_active_ms: row
            .get("lastActiveTime")
            .and_then(super::decode::number_as_i64)
            .unwrap_or_default(),
        current: row.get("ifLoginDevice").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn text(row: &Value, name: &str) -> String {
    row.get(name).and_then(Value::as_str).unwrap_or_default().trim().to_owned()
}
