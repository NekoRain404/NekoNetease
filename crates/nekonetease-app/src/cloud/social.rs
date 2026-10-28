//! Social reads and the one social write, behind the session guard.
//!
//! Message and notification text is returned to the caller and nothing else: no function here
//! stores it, logs it, or hands it to a cache. The rolling log is attached to public bug reports,
//! so a private conversation in it would be a leak the user cannot undo. The lists are read-only
//! for the same reason — this module has no send, delete or mark-read call.

use nekonetease_core::entities::social::{
    SocialConversationPage, SocialEventPage, SocialMessagePage, SocialNoticePage,
    SocialPlaylistPage, SocialProfile, SocialSubcount,
};
use nekonetease_core::error::{AppError, AppResult};

use super::CloudService;
use super::validation::validate_id;

/// The service's own page limit for these endpoints. A caller may ask for less, never for an
/// unbounded page: every one of these responses otherwise returns the account's whole history.
const MAX_PAGE_LIMIT: u32 = 100;

impl CloudService {
    pub async fn social_profile(&self, user_id: i64) -> AppResult<SocialProfile> {
        validate_id("用户", user_id)?;
        self.public_request(self.client.user_profile(user_id)).await
    }

    /// The signed-in account's totals, which the profile endpoint reports for somebody else.
    pub async fn social_subcount(&self) -> AppResult<SocialSubcount> {
        self.private_request(self.client.user_subcount()).await
    }

    pub async fn social_set_followed(&self, user_id: i64, followed: bool) -> AppResult<()> {
        let (account, cancellation) = self.account_snapshot()?;
        validate_id("用户", user_id)?;
        if user_id == account.id {
            return Err(AppError::Validation("不能关注自己".to_owned()));
        }
        self.account_request_in_generation(
            self.client.set_user_followed(user_id, followed),
            cancellation,
        )
        .await
    }

    pub async fn social_events(
        &self,
        user_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SocialEventPage> {
        validate_id("用户", user_id)?;
        let limit = page_limit(limit)?;
        self.public_request(self.client.user_events(user_id, before, limit)).await
    }

    pub async fn social_playlists(
        &self,
        user_id: i64,
        offset: u64,
        limit: u32,
    ) -> AppResult<SocialPlaylistPage> {
        validate_id("用户", user_id)?;
        let limit = page_limit(limit)?;
        self.public_request(self.client.user_playlist_page(user_id, offset, u64::from(limit))).await
    }

    pub async fn social_conversations(
        &self,
        offset: u64,
        limit: u32,
    ) -> AppResult<SocialConversationPage> {
        let (account, cancellation) = self.account_snapshot()?;
        let limit = page_limit(limit)?;
        self.account_request_in_generation(
            self.client.private_conversations(account.id, offset, u64::from(limit)),
            cancellation,
        )
        .await
    }

    pub async fn social_messages(
        &self,
        user_id: i64,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SocialMessagePage> {
        let (account, cancellation) = self.account_snapshot()?;
        validate_id("用户", user_id)?;
        let limit = page_limit(limit)?;
        self.account_request_in_generation(
            self.client.private_messages(account.id, user_id, before, limit),
            cancellation,
        )
        .await
    }

    pub async fn social_notices(
        &self,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SocialNoticePage> {
        let (_, cancellation) = self.account_snapshot()?;
        let limit = page_limit(limit)?;
        self.account_request_in_generation(self.client.notices(before, limit), cancellation).await
    }

    pub async fn social_comment_notifications(
        &self,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SocialNoticePage> {
        let (account, cancellation) = self.account_snapshot()?;
        let limit = page_limit(limit)?;
        self.account_request_in_generation(
            self.client.comment_notifications(account.id, before, limit),
            cancellation,
        )
        .await
    }
}

fn page_limit(limit: u32) -> AppResult<u32> {
    if (1..=MAX_PAGE_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(AppError::Validation(format!("每页数量必须在 1 到 {MAX_PAGE_LIMIT} 之间")))
    }
}

#[cfg(test)]
#[path = "../tests/cloud_social_tests.rs"]
mod tests;
