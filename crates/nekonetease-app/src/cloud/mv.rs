//! Music-video browsing over the public request gate.

use super::CloudService;
use super::validation::validate_id;
use nekonetease_core::entities::cloud::{Mv, MvStream};
use nekonetease_core::entities::comment::CommentThreadInfo;
use nekonetease_core::error::{AppError, AppResult};

/// The artist listing's page size, matching the other paged cloud lists.
const MV_PAGE_SIZE: u64 = 30;

/// The tiers the stream endpoint accepts as `r`; anything else is a caller bug, not a
/// server refusal, so it is rejected before the request.
const MV_QUALITIES: [u16; 4] = [1080, 720, 480, 240];

impl CloudService {
    pub async fn mv_detail(&self, id: i64) -> AppResult<Mv> {
        validate_id("MV", id)?;
        self.public_request(self.client.mv_detail(id)).await
    }

    pub async fn mv_url(&self, id: i64, quality: u16) -> AppResult<MvStream> {
        validate_id("MV", id)?;
        if !MV_QUALITIES.contains(&quality) {
            return Err(AppError::Validation(format!("MV 清晰度无效: {quality}")));
        }
        self.public_request(self.client.mv_url(id, quality)).await
    }

    /// One page of an artist's filmography. The offset is the count already shown, so the caller
    /// advances it by the page it received rather than by the constant.
    pub async fn artist_mvs_page(&self, artist_id: i64, offset: u64) -> AppResult<(Vec<Mv>, bool)> {
        validate_id("歌手", artist_id)?;
        self.public_request(self.client.artist_mvs(artist_id, MV_PAGE_SIZE, offset)).await
    }

    pub async fn similar_mvs(&self, id: i64) -> AppResult<Vec<Mv>> {
        validate_id("MV", id)?;
        self.public_request(self.client.similar_mvs(id)).await
    }

    /// The MV thread's like state and count. The route is public, and the detail's own counts
    /// cover collect and share, so the header needs no second request for them.
    pub async fn mv_thread_info(&self, id: i64) -> AppResult<CommentThreadInfo> {
        validate_id("MV", id)?;
        self.public_request(self.client.mv_thread_info(id)).await
    }

    /// Liking an MV changes the signed-in account, so it passes the private gate.
    pub async fn mv_like(&self, id: i64, like: bool) -> AppResult<()> {
        validate_id("MV", id)?;
        self.private_request(self.client.mv_like(id, like)).await
    }

    /// Collecting an MV is likewise account-scoped.
    pub async fn mv_subscribe(&self, id: i64, subscribed: bool) -> AppResult<()> {
        validate_id("MV", id)?;
        self.private_request(self.client.mv_subscribe(id, subscribed)).await
    }
}
