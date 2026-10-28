//! The recording wiki's "About this song" page.

use nekonetease_core::entities::song_about::SongAbout;
use nekonetease_core::error::AppResult;

use super::CloudService;
use super::validation::validate_id;

impl CloudService {
    /// Read-only, and empty when the service holds no wiki entry for the recording.
    pub async fn song_about(&self, song_id: i64) -> AppResult<SongAbout> {
        validate_id("歌曲", song_id)?;
        self.private_request(self.client.song_about(song_id)).await
    }
}
