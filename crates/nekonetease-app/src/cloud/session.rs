//! Login, restoration, logout, and the transitions that guard them.

use std::sync::atomic::Ordering;
use std::time::Duration;

use nekonetease_core::entities::cloud::Account;
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_net::netease::{Client, QrStatus as NetEaseQrStatus};
use tokio_util::sync::CancellationToken;

use super::credential::{SESSION_VERSION, delete_credential, valid_device_id, write_credential};
use super::validation::validate_nonempty;
use super::{CloudService, QrStatus, SessionStatus, StoredSession};

impl CloudService {
    /// Restores a saved cookie once during startup. A rejected cookie is deleted and leaves the
    /// application signed out; it is never exposed to the UI or written to logs.
    pub async fn restore_session(&self) -> AppResult<bool> {
        let _guard = self.transition.lock().await;
        let Some(saved) = self.pending_restore.lock().await.take() else {
            return Ok(false);
        };
        if saved.version != SESSION_VERSION || !valid_device_id(&saved.device_id) {
            let removed = tokio::task::spawn_blocking(delete_credential)
                .await
                .is_ok_and(|result| result.is_ok());
            self.publish(None, Some("已清除无法读取的登录信息。".into()), removed);
            return Ok(false);
        }

        self.begin_transition().await;
        self.client.restore_cookie(&saved.cookie).await;
        match self.client.account().await {
            Ok(account) => {
                self.guest_initialized.store(true, Ordering::Release);
                self.publish(Some(account), None, self.status().remember_available);
                Ok(true)
            }
            Err(error) => {
                self.client.clear_session().await;
                self.guest_initialized.store(false, Ordering::Release);
                if matches!(error, nekonetease_net::netease::Error::Authentication) {
                    let removed = tokio::task::spawn_blocking(delete_credential)
                        .await
                        .is_ok_and(|result| result.is_ok());
                    self.publish(None, Some("保存的登录已失效，请重新登录。".into()), removed);
                    Ok(false)
                } else {
                    *self.pending_restore.lock().await = Some(saved);
                    self.publish(
                        None,
                        Some("暂时无法连接网易云，登录恢复将在网络可用后重试。".into()),
                        self.status().remember_available,
                    );
                    Err(error.into())
                }
            }
        }
    }

    /// Restores the saved session on [`RESTORE_RETRY_DELAYS`], stopping at the first success or
    /// when `shutdown` fires.
    ///
    /// Exhausting the schedule publishes a notice instead of leaving the connection failure as
    /// the last word; the pending session itself stays stored so nothing is lost.
    pub async fn restore_session_with_retry(&self, shutdown: &CancellationToken) {
        let Some(last_error) = retry_restore(shutdown, || self.restore_session()).await else {
            return;
        };
        log::warn!(
            "NetEase session restore: gave up after {} attempts: {}",
            RESTORE_RETRY_DELAYS.len(),
            describe(&last_error)
        );
        // A sign-in by another route may have consumed the saved session and published its own
        // notice; do not overwrite that one.
        if self.pending_restore.lock().await.is_some() {
            self.publish(
                None,
                Some("无法连接网易云，登录恢复已停止，可重新扫码或登录。".into()),
                self.status().remember_available,
            );
        }
    }

    /// Initializes a guest session for endpoints that allow anonymous discovery.
    pub async fn ensure_guest(&self) -> AppResult<()> {
        if self.status().account.is_some() || self.guest_initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        let _guard = self.transition.lock().await;
        if self.status().account.is_some() || self.guest_initialized.load(Ordering::Acquire) {
            return Ok(());
        }
        self.client.initialize_guest().await?;
        self.guest_initialized.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn login_phone(
        &self,
        phone: &str,
        country_code: &str,
        password: &str,
    ) -> AppResult<Account> {
        validate_nonempty("手机号", phone)?;
        validate_nonempty("国家区号", country_code)?;
        validate_nonempty("密码", password)?;
        let _guard = self.transition.lock().await;
        self.begin_login().await;
        let account = self.client.login_phone(phone, country_code, password).await?;
        self.complete_login(account).await
    }

    /// Sends the SMS login code. Nothing about the account changes until the code is redeemed.
    pub async fn send_login_captcha(&self, phone: &str, country_code: &str) -> AppResult<()> {
        let phone = validate_phone(phone)?;
        let country_code = validate_country_code(country_code)?;
        self.client.send_login_captcha(phone, country_code).await.map_err(Into::into)
    }

    pub async fn login_phone_captcha(
        &self,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> AppResult<Account> {
        let phone = validate_phone(phone)?;
        let country_code = validate_country_code(country_code)?;
        let captcha = validate_captcha(captcha)?;
        let _guard = self.transition.lock().await;
        self.begin_login().await;
        let account = self.client.login_phone_captcha(phone, country_code, captcha).await?;
        self.complete_login(account).await
    }

    pub async fn login_email(&self, email: &str, password: &str) -> AppResult<Account> {
        validate_nonempty("邮箱", email)?;
        validate_nonempty("密码", password)?;
        let _guard = self.transition.lock().await;
        self.begin_login().await;
        let account = self.client.login_email(email, password).await?;
        self.complete_login(account).await
    }

    pub async fn begin_qr_login(&self) -> AppResult<String> {
        let _guard = self.transition.lock().await;
        self.begin_login().await;
        self.client.qr_key().await.map_err(Into::into)
    }

    /// Starts the launch-page QR flow when no account is signed in.
    ///
    /// A pending restore deliberately does not gate this. Such a session is pending precisely
    /// because the network was unreachable, and the retry schedule gives up after about two
    /// minutes; the user has to be able to sign in while it waits. `begin_login` clears the
    /// pending copy under this same lock, so a retry already in flight cannot bring it back.
    pub async fn begin_qr_login_if_signed_out(&self) -> AppResult<Option<String>> {
        let _guard = self.transition.lock().await;
        if self.status().account.is_some() {
            return Ok(None);
        }
        self.begin_login().await;
        self.client.qr_key().await.map(Some).map_err(Into::into)
    }

    pub async fn check_qr_login(&self, key: &str) -> AppResult<QrStatus> {
        let _guard = self.transition.lock().await;
        let result = match self.client.qr_check(key).await? {
            NetEaseQrStatus::Expired => QrStatus::Expired,
            NetEaseQrStatus::Waiting => QrStatus::Waiting,
            NetEaseQrStatus::Confirming => QrStatus::Confirming,
            NetEaseQrStatus::Authorized => QrStatus::Authorized,
        };
        if result == QrStatus::Authorized {
            let account = self.client.account().await?;
            self.complete_login(account).await?;
        }
        Ok(result)
    }

    /// Invalidates account tasks and local session state before asking the server to end its
    /// session. Local logout still completes if the remote endpoint is unavailable.
    pub async fn logout(&self) -> AppResult<()> {
        let _guard = self.transition.lock().await;
        let cookie = self.client.cookie_for_storage().await;
        let device_id = self.client.device_id().to_owned();
        self.begin_transition().await;
        if !cookie.is_empty()
            && let Ok(remote_client) = Client::with_device_id(device_id)
        {
            tokio::spawn(async move {
                remote_client.restore_cookie(&cookie).await;
                let _ = remote_client.logout().await;
            });
        }
        self.guest_initialized.store(false, Ordering::Release);
        self.pending_restore.lock().await.take();
        let deleted =
            tokio::task::spawn_blocking(delete_credential).await.is_ok_and(|result| result.is_ok());
        let notice = (!deleted).then(|| "已退出当前账号，但系统未能删除已保存的登录信息。".into());
        self.publish(None, notice, deleted);
        Ok(())
    }

    pub(super) async fn begin_transition(&self) {
        self.account_tasks.read().cancel();
        self.liked_ids_revision.fetch_add(1, Ordering::AcqRel);
        *self.liked_ids_cache.write() = None;
        *self.recent_songs_cache.write() = None;
        self.playlist_cache.lock().clear();
        self.publish(None, None, self.status().remember_available);
        *self.account_tasks.write() = CancellationToken::new();
        self.client.clear_session().await;
        self.guest_initialized.store(false, Ordering::Release);
    }

    async fn begin_login(&self) {
        self.begin_transition().await;
        self.pending_restore.lock().await.take();
        let deleted =
            tokio::task::spawn_blocking(delete_credential).await.is_ok_and(|result| result.is_ok());
        if !deleted {
            self.publish(
                None,
                Some("无法清除先前保存的登录信息；新登录成功后会尝试覆盖。".into()),
                false,
            );
        }
    }

    async fn complete_login(&self, account: Account) -> AppResult<Account> {
        self.guest_initialized.store(true, Ordering::Release);
        let cookie = self.client.cookie_for_storage().await;
        let saved = StoredSession {
            version: SESSION_VERSION,
            device_id: self.client.device_id().to_owned(),
            cookie,
        };
        let serialized = serde_json::to_string(&saved)
            .map_err(|_| AppError::network_msg("无法保存网易云登录状态"))?;
        let write = tokio::task::spawn_blocking(move || write_credential(&serialized)).await;
        let remembered = matches!(write, Ok(Ok(())));
        let notice = if remembered {
            None
        } else {
            Some("本机凭据存储不可用，本次登录只保留在内存中。".into())
        };
        self.publish(Some(account.clone()), notice, remembered);
        Ok(account)
    }

    pub(super) fn publish(
        &self,
        account: Option<Account>,
        notice: Option<String>,
        remember_available: bool,
    ) {
        let generation = self.status.borrow().generation.wrapping_add(1);
        self.status.send_replace(SessionStatus { account, remember_available, notice, generation });
    }
}

/// The restore's whole attempt budget: an immediate try, then a doubling backoff capped at a
/// minute — seven attempts totalling about two minutes.
///
/// A launch-time outage takes tens of seconds to clear (a captive portal, a VPN still settling),
/// so the first retries earn their keep. Past two minutes the saved session is usually hopeless
/// for this run, and a task retrying for the whole session would keep contending with a user who
/// signed in by another route. Stopping costs nothing but the choice: the pending session stays
/// stored, and the QR and password entries do not wait on it.
pub(super) const RESTORE_RETRY_DELAYS: [Duration; 7] = [
    Duration::ZERO,
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(32),
    Duration::from_secs(60),
];

/// Runs `attempt` until it reports success or `shutdown` cancels, returning the last error once
/// every delay in [`RESTORE_RETRY_DELAYS`] is spent.
pub(super) async fn retry_restore<F, Fut>(
    shutdown: &CancellationToken,
    mut attempt: F,
) -> Option<AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<bool>>,
{
    let mut last_error = None;
    for delay in RESTORE_RETRY_DELAYS {
        tokio::select! {
            () = shutdown.cancelled() => return None,
            () = tokio::time::sleep(delay) => {},
        }
        match attempt().await {
            Ok(_) => return None,
            Err(error) => {
                log::warn!("NetEase session restore: {}", describe(&error));
                last_error = Some(error);
            }
        }
    }
    last_error
}

const PHONE_MIN_DIGITS: usize = 5;
const PHONE_MAX_DIGITS: usize = 20;
const COUNTRY_CODE_MAX_DIGITS: usize = 4;
const CAPTCHA_MIN_DIGITS: usize = 4;
const CAPTCHA_MAX_DIGITS: usize = 8;

pub(super) fn validate_phone(phone: &str) -> AppResult<&str> {
    let phone = phone.trim();
    if phone.is_empty() {
        return Err(AppError::Validation("手机号不能为空".to_owned()));
    }
    if !phone.bytes().all(|byte| byte.is_ascii_digit())
        || !(PHONE_MIN_DIGITS..=PHONE_MAX_DIGITS).contains(&phone.len())
    {
        return Err(AppError::Validation("手机号格式不正确".to_owned()));
    }
    Ok(phone)
}

pub(super) fn validate_country_code(country_code: &str) -> AppResult<&str> {
    let country_code = country_code.trim();
    if country_code.is_empty() {
        return Err(AppError::Validation("国家区号不能为空".to_owned()));
    }
    if !country_code.bytes().all(|byte| byte.is_ascii_digit())
        || country_code.len() > COUNTRY_CODE_MAX_DIGITS
    {
        return Err(AppError::Validation("国家区号格式不正确".to_owned()));
    }
    Ok(country_code)
}

pub(super) fn validate_captcha(captcha: &str) -> AppResult<&str> {
    let captcha = captcha.trim();
    if captcha.is_empty() {
        return Err(AppError::Validation("验证码不能为空".to_owned()));
    }
    if !captcha.bytes().all(|byte| byte.is_ascii_digit())
        || !(CAPTCHA_MIN_DIGITS..=CAPTCHA_MAX_DIGITS).contains(&captcha.len())
    {
        return Err(AppError::Validation("验证码格式不正确".to_owned()));
    }
    Ok(captcha)
}
