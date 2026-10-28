//! Podcast / DJ-radio browsing and subscription, behind the account gate.
//!
//! A programme is listed as a track-shaped row, but its playable id is the `mainSong` id the
//! client carries; these methods never invent a stream path of their own.

use super::CloudService;
use super::validation::validate_id;
use nekonetease_core::entities::dj::{DjProgram, DjProgramPage, DjRadio, DjRadioPage};
use nekonetease_core::error::AppResult;

/// The page size shared by the hot listing and a station's programme list.
const DJ_PAGE_SIZE: u64 = 30;

impl CloudService {
    pub async fn dj_recommend(&self) -> AppResult<Vec<DjRadio>> {
        self.private_request(self.client.dj_recommend()).await
    }

    /// One page of the hot-radio listing; the offset is the count already received.
    pub async fn dj_hot_page(&self, offset: u64) -> AppResult<DjRadioPage> {
        self.private_request(self.client.dj_hot(DJ_PAGE_SIZE, offset)).await
    }

    pub async fn dj_radio(&self, id: i64) -> AppResult<DjRadio> {
        validate_id("电台", id)?;
        self.private_request(self.client.dj_radio_detail(id)).await
    }

    /// One page of a station's programmes, oldest first. The offset is the number of rows already
    /// shown, not a page number.
    pub async fn dj_programs_page(&self, radio_id: i64, offset: u64) -> AppResult<DjProgramPage> {
        validate_id("电台", radio_id)?;
        self.private_request(self.client.dj_programs(radio_id, DJ_PAGE_SIZE, offset, true)).await
    }

    pub async fn dj_program(&self, id: i64) -> AppResult<DjProgram> {
        validate_id("节目", id)?;
        self.private_request(self.client.dj_program_detail(id)).await
    }

    pub async fn dj_set_subscribed(&self, id: i64, subscribed: bool) -> AppResult<()> {
        validate_id("电台", id)?;
        self.private_request(self.client.dj_set_subscribed(id, subscribed)).await
    }
}
