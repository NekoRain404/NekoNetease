//! The chart catalogue.
//!
//! A chart is a playlist, so its tracks are read through [`CloudService::library_playlist`] rather
//! than a second request here. This module only supplies the list the empty search page groups.

use nekonetease_core::entities::cloud::Playlist;
use nekonetease_core::error::AppResult;

use super::CloudService;

impl CloudService {
    /// Returns every published chart in the server's order, tags included.
    pub async fn chart_catalogue(&self) -> AppResult<Vec<Playlist>> {
        self.public_request(self.client.chart_catalogue()).await
    }
}
