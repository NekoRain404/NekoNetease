//! The Settings account card's avatar: one bounded fetch per account, decoded off the UI thread,
//! and discarded once the account it was requested for stops being current.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nekonetease_app::cloud::CloudService;
use nekonetease_artwork::media::image::image_decode::{
    FilterType, MAX_SOURCE_DIM, decode_memory_capped_to, fit_within, resize_rgb8,
};
use nekonetease_ui::{AppWindow, CloudAccount};
use parking_lot::Mutex;
use slint::{ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer, Weak};
use tokio::runtime::Handle;

/// Above any 160 px JPEG the directory serves, below what a forged body could ask the decoder
/// to allocate.
const MAX_SOURCE_BYTES: usize = 256 * 1024;
/// Twice the 40 px tile at 2×, with room for fractional scaling.
const AVATAR_PIXELS: u32 = 96;

/// Decides which status tick owns the avatar. Any change of account or URL bumps the generation,
/// and a completion paints only while its own generation is still the latest.
#[derive(Clone, Default)]
pub struct AvatarController {
    generation: Arc<AtomicU64>,
    last_request: Arc<Mutex<Option<String>>>,
}

enum AvatarUpdate {
    Unchanged,
    Clear(u64),
    Fetch(u64, String),
}

impl AvatarController {
    /// Apply a published session to the account card. `None` covers both a signed-out session and
    /// a profile that carries no avatar.
    pub fn apply(
        &self,
        weak: &Weak<AppWindow>,
        cloud: &CloudService,
        runtime: &Handle,
        avatar_url: Option<String>,
    ) {
        match self.announce(avatar_url) {
            AvatarUpdate::Unchanged => {}
            AvatarUpdate::Clear(generation) => {
                self.paint(weak, generation, None);
            }
            AvatarUpdate::Fetch(generation, url) => {
                // The previous account's face must not sit under the next name while the request
                // is in flight.
                self.paint(weak, generation, None);
                let controller = self.clone();
                let cloud = cloud.clone();
                let runtime = runtime.clone();
                let weak = weak.clone();
                runtime.spawn(async move {
                    let Some(bytes) = cloud.fetch_avatar_bytes(&url, MAX_SOURCE_BYTES).await else {
                        return;
                    };
                    let decoded = tokio::task::spawn_blocking(move || decode_avatar(&bytes))
                        .await
                        .ok()
                        .flatten();
                    let Some(pixels) = decoded else {
                        return;
                    };
                    controller.paint(&weak, generation, Some(pixels));
                });
            }
        }
    }

    fn announce(&self, avatar_url: Option<String>) -> AvatarUpdate {
        let mut last = self.last_request.lock();
        if *last == avatar_url {
            return AvatarUpdate::Unchanged;
        }
        (*last).clone_from(&avatar_url);
        let generation = self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        avatar_url
            .map_or(AvatarUpdate::Clear(generation), |url| AvatarUpdate::Fetch(generation, url))
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn paint(
        &self,
        weak: &Weak<AppWindow>,
        generation: u64,
        pixels: Option<SharedPixelBuffer<Rgb8Pixel>>,
    ) {
        let weak = weak.clone();
        let controller = self.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !controller.is_current(generation) {
                return;
            }
            let image = pixels.map_or_else(Image::default, Image::from_rgb8);
            ui.global::<CloudAccount>().set_avatar_image(image);
        });
    }
}

fn decode_avatar(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
    let decoded = decode_memory_capped_to(bytes, MAX_SOURCE_DIM, AVATAR_PIXELS)?;
    let (width, height) =
        fit_within(decoded.width(), decoded.height(), AVATAR_PIXELS, AVATAR_PIXELS);
    let resized = resize_rgb8(&decoded, width, height, FilterType::Lanczos3)?;
    let (width, height) = resized.dimensions();
    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(width, height);
    buffer.make_mut_bytes().copy_from_slice(resized.as_raw());
    Some(buffer)
}

#[cfg(test)]
#[path = "tests/netease_avatar_tests.rs"]
mod tests;
