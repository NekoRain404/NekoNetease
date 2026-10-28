//! The account's uploaded cloud-disk songs, paged for the cloud collection view.

use nekonetease_core::entities::cloud::CloudDiskPage;
use nekonetease_core::error::AppResult;

use super::CloudService;

impl CloudService {
    /// One page of the signed-in account's cloud disk. Read-only, so a retried page is safe.
    pub async fn cloud_disk_page(&self, offset: u64, limit: u64) -> AppResult<CloudDiskPage> {
        self.private_request(self.client.cloud_disk_songs(offset, limit)).await
    }
}
