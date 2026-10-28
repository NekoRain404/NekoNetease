use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{Mutex, RwLock};

#[cfg(test)]
use super::endpoints as ep;
use super::{Error, Result, crypto};

use self::request::chrono_millis;
use self::session::valid_cookie_pair;

mod account;
mod charts;
mod cloud_disk;
mod decode;
mod discovery;
mod dj;
mod favorites;
mod library;
mod mv;
mod playback;
mod playlist_write;
mod request;
mod session;
mod social;
mod wiki;

#[cfg(test)]
#[path = "../tests/album_collection_tests.rs"]
mod album_collection_tests;

#[cfg(test)]
#[path = "../tests/account_tests.rs"]
mod account_tests;

#[cfg(test)]
#[path = "../tests/client_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/discovery_tests.rs"]
mod discovery_tests;

#[cfg(test)]
#[path = "../tests/library_tests.rs"]
mod library_tests;

#[cfg(test)]
#[path = "../tests/playlist_write_tests.rs"]
mod playlist_write_tests;

#[cfg(test)]
#[path = "../tests/cloud_disk_tests.rs"]
mod cloud_disk_tests;

#[cfg(test)]
#[path = "../tests/mv_tests.rs"]
mod mv_tests;

#[cfg(test)]
#[path = "../tests/charts_tests.rs"]
mod charts_tests;

#[cfg(test)]
#[path = "../tests/wiki_tests.rs"]
mod wiki_tests;

/// The shared HTTP client is injected by the application. Credentials are never Debug-printed.
#[derive(Clone)]
pub struct Client {
    /// `None` when the redirect-free API client could not be built, which keeps the API layer
    /// disabled rather than handing it a client that follows redirects.
    http: Option<reqwest::Client>,
    cookies: Arc<RwLock<BTreeMap<String, String>>>,
    xeapi_key: Arc<Mutex<Option<crypto::XeapiKey>>>,
    epoch: Arc<AtomicU64>,
    device_id: String,
    wnmcid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QrStatus {
    Expired,
    Waiting,
    Confirming,
    Authorized,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self::from_parts(crypto::hex(&rand::random::<[u8; 26]>(), true))
    }

    /// Creates a client with a stable device identifier restored from the OS credential store.
    pub fn with_device_id(device_id: impl Into<String>) -> Result<Self> {
        let device_id = device_id.into();
        if device_id.len() != 52
            || !device_id.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        {
            return Err(Error::InvalidDeviceId);
        }
        Ok(Self::from_parts(device_id))
    }

    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    fn from_parts(device_id: String) -> Self {
        let suffix = rand::random::<[u8; 6]>()
            .map(|byte| char::from(b'a' + byte % 26))
            .into_iter()
            .collect::<String>();
        Self {
            // Redirect-free: an API call is a POST to a host this client names itself.
            http: crate::services::net::build_api_http_client(),
            cookies: Arc::new(RwLock::new(BTreeMap::new())),
            xeapi_key: Arc::new(Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(0)),
            device_id,
            wnmcid: format!("{suffix}.{}.01.0", chrono_millis()),
        }
    }

    /// The API's only client. A build failure disables the layer instead of redirecting.
    fn http(&self) -> Result<&reqwest::Client> {
        self.http.as_ref().ok_or(Error::Transport)
    }

    pub fn generation(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub async fn clear_session(&self) {
        let mut cookies = self.cookies.write().await;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        cookies.clear();
    }

    pub async fn restore_cookie(&self, cookie: &str) {
        self.clear_session().await;
        let mut cookies = self.cookies.write().await;
        for pair in cookie.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=')
                && valid_cookie_pair(name, value)
            {
                cookies.insert(name.to_owned(), value.to_owned());
            }
        }
    }

    pub async fn cookie_for_storage(&self) -> String {
        self.cookies
            .read()
            .await
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}
