//! Account sessions for the direct `NetEase` client.
//!
//! This module is the application boundary for login and session lifetime. Views receive typed
//! account state and never see cookies, device credentials, or raw HTTP responses.

mod account;
mod cache;
mod charts;
mod cloud_disk;
mod covers;
mod credential;
mod discovery;
mod dj;
mod library;
mod mv;
mod playlists;
mod requests;
mod session;
mod social;
mod validation;
mod wiki;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use lru::LruCache;
use nekonetease_core::entities::cloud::Account;
use nekonetease_net::netease::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

use self::cache::{LikedIdsCache, PLAYLIST_CACHE_CAP, PlaylistCache, RecentSongsCache};
use self::credential::{decode_session, delete_credential, read_credential, valid_device_id};

pub use self::covers::{cover_request_url, fetch_cover_bytes};

/// QR sign-in progress exposed to the application and view layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrStatus {
    Expired,
    Waiting,
    Confirming,
    Authorized,
}

/// UI-safe account state. The credential and the session cookie intentionally have no public
/// representation in this type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionStatus {
    pub account: Option<Account>,
    pub remember_available: bool,
    pub notice: Option<String>,
    /// Changes on account transitions so views can discard private cached models.
    pub generation: u64,
}

/// Owns one `NetEase` client and coordinates login, restoration, logout, and account-scoped work.
#[derive(Clone)]
pub struct CloudService {
    client: Client,
    http: reqwest::Client,
    pending_restore: Arc<Mutex<Option<StoredSession>>>,
    status: watch::Sender<SessionStatus>,
    transition: Arc<Mutex<()>>,
    account_tasks: Arc<parking_lot::RwLock<CancellationToken>>,
    guest_initialized: Arc<AtomicBool>,
    liked_ids_cache: LikedIdsCache,
    liked_ids_revision: Arc<AtomicU64>,
    recent_songs_cache: RecentSongsCache,
    playlist_cache: PlaylistCache,
}

#[derive(Serialize, Deserialize)]
struct StoredSession {
    version: u8,
    device_id: String,
    cookie: String,
}

impl CloudService {
    /// Reads only the OS credential store. If it is unavailable, the service remains usable and
    /// reports that this process cannot remember a login.
    pub async fn new(http: reqwest::Client) -> Self {
        let read = tokio::task::spawn_blocking(read_credential).await;
        let (stored, remember_available, notice) = match read {
            Ok(Ok(Some(secret))) => {
                if let Some(stored) = decode_session(&secret) {
                    log::debug!("cloud: restoring the saved NetEase session");
                    (Some(stored), true, None)
                } else {
                    log::debug!("cloud: the saved credential is unreadable and was dropped");
                    let removed = tokio::task::spawn_blocking(delete_credential)
                        .await
                        .is_ok_and(|result| result.is_ok());
                    (None, removed, Some("已清除无法读取的登录信息。".into()))
                }
            }
            Ok(Ok(None)) => {
                log::debug!("cloud: the credential store holds no saved session");
                (None, true, None)
            }
            _ => {
                log::debug!("cloud: the credential store could not be read");
                (None, false, Some("系统凭据存储不可用，登录信息只会保留在本次运行中。".into()))
            }
        };

        let device_id = stored
            .as_ref()
            .map(|session| session.device_id.clone())
            .filter(|id| valid_device_id(id));
        let client =
            device_id.and_then(|id| Client::with_device_id(id).ok()).unwrap_or_else(Client::new);
        let initial = SessionStatus { account: None, remember_available, notice, generation: 0 };
        let (status, _) = watch::channel(initial);

        Self {
            client,
            http,
            pending_restore: Arc::new(Mutex::new(stored)),
            status,
            transition: Arc::new(Mutex::new(())),
            account_tasks: Arc::new(parking_lot::RwLock::new(CancellationToken::new())),
            guest_initialized: Arc::new(AtomicBool::new(false)),
            liked_ids_cache: Arc::new(parking_lot::RwLock::new(None)),
            liked_ids_revision: Arc::new(AtomicU64::new(0)),
            recent_songs_cache: Arc::new(parking_lot::RwLock::new(None)),
            playlist_cache: Arc::new(parking_lot::Mutex::new(LruCache::new(PLAYLIST_CACHE_CAP))),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<SessionStatus> {
        self.status.subscribe()
    }

    pub fn status(&self) -> SessionStatus {
        self.status.borrow().clone()
    }

    /// Tasks that hold account-private data should select on this token. It is cancelled at the
    /// beginning of every account transition, before a new session becomes visible.
    pub fn account_tasks_token(&self) -> CancellationToken {
        self.account_tasks.read().clone()
    }

    /// Avatar bytes through the client the session already owns. The cap belongs to the caller,
    /// which is the only place that knows the size the image is drawn at.
    pub async fn fetch_avatar_bytes(&self, source: &str, max_bytes: usize) -> Option<Vec<u8>> {
        covers::fetch_avatar_bytes(&self.http, source, max_bytes).await
    }
}

#[cfg(test)]
#[path = "../tests/cloud_service_tests.rs"]
mod tests;
