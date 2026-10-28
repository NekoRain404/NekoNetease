//! The account gates every cloud call passes through before it reaches the network.

use std::future::Future;

use nekonetease_core::entities::cloud::Account;
use nekonetease_core::error::{AppError, AppResult};
use tokio_util::sync::CancellationToken;

use super::CloudService;

impl CloudService {
    pub(super) fn require_account(&self) -> AppResult<Account> {
        self.status().account.ok_or_else(|| AppError::network_msg("请先登录网易云账号"))
    }

    pub(super) fn account_snapshot(&self) -> AppResult<(Account, CancellationToken)> {
        // Capture the account's cancellation generation before reading its identity. A transition
        // cancels that generation before publishing signed-out state, so a request carrying the
        // previous uid can never slip through under a newly installed account.
        let cancellation = self.account_tasks_token();
        let account = self.require_account()?;
        if cancellation.is_cancelled() {
            return Err(AppError::network_msg("网易云请求已取消"));
        }
        Ok((account, cancellation))
    }

    pub(super) async fn public_request<T>(
        &self,
        request: impl Future<Output = nekonetease_net::netease::Result<T>>,
    ) -> AppResult<T> {
        self.ensure_guest().await?;
        self.account_request(request).await
    }

    pub(super) async fn private_request<T>(
        &self,
        request: impl Future<Output = nekonetease_net::netease::Result<T>>,
    ) -> AppResult<T> {
        let cancellation = self.account_tasks_token();
        self.require_account()?;
        self.account_request_in_generation(request, cancellation).await
    }

    pub(super) async fn account_request<T>(
        &self,
        request: impl Future<Output = nekonetease_net::netease::Result<T>>,
    ) -> AppResult<T> {
        let cancellation = self.account_tasks_token();
        self.account_request_in_generation(request, cancellation).await
    }

    pub(super) async fn account_request_in_generation<T>(
        &self,
        request: impl Future<Output = nekonetease_net::netease::Result<T>>,
        cancellation: CancellationToken,
    ) -> AppResult<T> {
        if cancellation.is_cancelled() {
            return Err(AppError::network_msg("网易云请求已取消"));
        }
        tokio::select! {
            () = cancellation.cancelled() => Err(AppError::network_msg("网易云请求已取消")),
            result = request => {
                if cancellation.is_cancelled() {
                    Err(AppError::network_msg("网易云请求已取消"))
                } else {
                    result.map_err(Into::into)
                }
            },
        }
    }
}
