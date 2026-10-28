//! 云贝 balance, the daily check-in, and the account's signed-in devices.

use nekonetease_core::entities::cloud::CloudBeans;
use nekonetease_core::error::AppResult;

use super::CloudService;
use super::validation::validate_nonempty;

/// A device row the account card can draw. The key is the route's own opaque handle and never
/// reaches a log line; the remaining fields are display data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceSession {
    pub key: String,
    pub name: String,
    pub os: String,
    pub place: String,
    pub last_active_ms: i64,
    pub current: bool,
}

impl CloudService {
    /// Reads the signed-in account's 云贝 balance and whether today's check-in is already claimed.
    pub async fn cloud_beans(&self) -> AppResult<CloudBeans> {
        self.private_request(self.client.cloud_beans()).await
    }

    /// Claims today's 云贝 check-in. `false` means the server reports it was already claimed.
    pub async fn check_in(&self) -> AppResult<bool> {
        self.private_request(self.client.check_in()).await
    }

    /// Lists the account's signed-in devices, including the current one.
    pub async fn devices(&self) -> AppResult<Vec<DeviceSession>> {
        let devices = self.private_request(self.client.devices()).await?;
        Ok(devices
            .into_iter()
            .map(|device| DeviceSession {
                key: device.key,
                name: device.name,
                os: device.os,
                place: device.place,
                last_active_ms: device.last_active_ms,
                current: device.current,
            })
            .collect())
    }

    /// Signs the named device out of the account.
    pub async fn revoke_device(&self, key: &str) -> AppResult<()> {
        validate_nonempty("设备标识", key)?;
        self.private_request(self.client.revoke_device(key)).await
    }
}
