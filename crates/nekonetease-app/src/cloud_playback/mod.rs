//! Joins typed `NetEase` stream tickets to the native decoder and output engine.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nekonetease_core::entities::cloud::{Account, PlaybackRestriction, Quality};
use nekonetease_core::error::AppResult;
use nekonetease_engine::player::engine::backend::PlaybackEngine;
use nekonetease_engine::player::engine::event_sink::PlayerSinks;
use nekonetease_engine::player::engine::state::PlayerStateHandle;
use parking_lot::RwLock;

use crate::cloud::{CloudService, QrStatus, SessionStatus};
use tokio::sync::watch;

use source::ActiveStreamTicket;

mod playback;
mod queue;
mod queue_playback;
mod refusal;
mod source;

pub use queue::CloudQueueShuffle;

/// Result of resolving and handing a cloud track to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudPlaybackResult {
    /// False when a newer play, pause, stop, or account transition superseded this request.
    pub started: bool,
    /// The server marked the track as a limited trial.
    pub trial: bool,
    /// Why the server withheld unrestricted playback, when it did.
    pub restriction: Option<PlaybackRestriction>,
    /// The tier the server actually served, when it reported one and the stream started.
    pub served_level: Option<Quality>,
}

/// Application-level orchestration for direct, buffered `NetEase` playback.
#[derive(Clone)]
pub struct CloudPlayback {
    cloud: CloudService,
    http: reqwest::Client,
    artwork_dir: PathBuf,
    engine: Arc<PlaybackEngine>,
    player_state: Arc<PlayerStateHandle>,
    sinks: Arc<PlayerSinks>,
    quality: Arc<RwLock<Quality>>,
    queue_generation: Arc<AtomicU64>,
    pending_queue_playback_generation: Arc<AtomicU64>,
    listening_generation: Arc<AtomicU64>,
    decoder_generation: Arc<AtomicU64>,
    active_stream_ticket: Arc<RwLock<Option<ActiveStreamTicket>>>,
}

impl CloudPlayback {
    /// Build the shared API and playback handles from the one HTTP client and artwork cache root.
    pub async fn initialize(
        http: reqwest::Client,
        artwork_dir: PathBuf,
        engine: Arc<PlaybackEngine>,
        player_state: Arc<PlayerStateHandle>,
        sinks: Arc<PlayerSinks>,
    ) -> (CloudService, Self) {
        let cloud = CloudService::new(http.clone()).await;
        let playback = Self::new(cloud.clone(), http, artwork_dir, engine, player_state, sinks);
        (cloud, playback)
    }

    pub fn new(
        cloud: CloudService,
        http: reqwest::Client,
        artwork_dir: PathBuf,
        engine: Arc<PlaybackEngine>,
        player_state: Arc<PlayerStateHandle>,
        sinks: Arc<PlayerSinks>,
    ) -> Self {
        Self {
            cloud,
            http,
            artwork_dir,
            engine,
            player_state,
            sinks,
            quality: Arc::new(RwLock::new(Quality::default())),
            queue_generation: Arc::new(AtomicU64::new(0)),
            pending_queue_playback_generation: Arc::new(AtomicU64::new(0)),
            listening_generation: Arc::new(AtomicU64::new(0)),
            decoder_generation: Arc::new(AtomicU64::new(0)),
            active_stream_ticket: Arc::new(RwLock::new(None)),
        }
    }

    pub fn quality(&self) -> Quality {
        *self.quality.read()
    }

    /// Identifies real cloud-track starts, including a repeat of the same song. Seeking and
    /// quality changes reopen the decoder with a start position and preserve this identity.
    pub fn listening_generation(&self) -> u64 {
        self.listening_generation.load(Ordering::Acquire)
    }

    /// Changes on every installed cloud decoder, allowing observers to discard seek jumps.
    pub fn decoder_generation(&self) -> u64 {
        self.decoder_generation.load(Ordering::Acquire)
    }

    pub fn set_quality(&self, quality: Quality) {
        *self.quality.write() = quality;
    }

    pub fn account_status(&self) -> SessionStatus {
        self.cloud.status()
    }

    pub fn subscribe_account(&self) -> watch::Receiver<SessionStatus> {
        self.cloud.subscribe()
    }

    /// Account changes stop the previous account's audio before the service invalidates its
    /// account-scoped requests and session.
    pub async fn login_phone(
        &self,
        phone: &str,
        country_code: &str,
        password: &str,
    ) -> AppResult<Account> {
        self.engine.stop();
        self.cloud.login_phone(phone, country_code, password).await
    }

    pub async fn login_email(&self, email: &str, password: &str) -> AppResult<Account> {
        self.engine.stop();
        self.cloud.login_email(email, password).await
    }

    pub async fn begin_qr_login(&self) -> AppResult<String> {
        self.engine.stop();
        self.cloud.begin_qr_login().await
    }

    /// Starts the automatic launch-page QR flow without interrupting a restored account.
    pub async fn begin_qr_login_if_signed_out(&self) -> AppResult<Option<String>> {
        let key = self.cloud.begin_qr_login_if_signed_out().await?;
        if key.is_some() {
            self.engine.stop();
        }
        Ok(key)
    }

    pub async fn check_qr_login(&self, key: &str) -> AppResult<QrStatus> {
        let status = self.cloud.check_qr_login(key).await?;
        if status == QrStatus::Authorized {
            self.engine.stop();
        }
        Ok(status)
    }

    pub async fn logout(&self) -> AppResult<()> {
        self.engine.stop();
        self.cloud.logout().await
    }
}

#[cfg(test)]
#[path = "../tests/cloud_queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "../tests/cloud_playback_tests.rs"]
mod tests;
