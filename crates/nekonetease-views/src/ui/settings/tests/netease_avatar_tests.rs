use image::{Rgb, RgbImage};
use nekonetease_artwork::media::image::image_decode::encode_jpeg;

use super::{AVATAR_PIXELS, AvatarController, AvatarUpdate, decode_avatar};

fn jpeg_fixture(width: u32, height: u32) -> Option<Vec<u8>> {
    encode_jpeg(RgbImage::from_pixel(width, height, Rgb([40, 90, 140])), 80).ok()
}

#[test]
fn an_avatar_decodes_within_the_tile_it_is_drawn_in() {
    let bytes = jpeg_fixture(640, 480);
    assert!(bytes.is_some(), "the fixture must encode");
    let Some(bytes) = bytes else { return };
    let pixels = decode_avatar(&bytes);
    assert!(pixels.is_some(), "the fixture must decode");
    let Some(pixels) = pixels else { return };
    assert!(pixels.width() <= AVATAR_PIXELS);
    assert!(pixels.height() <= AVATAR_PIXELS);
    assert!(pixels.width() > 0 && pixels.height() > 0);
}

#[test]
fn a_body_that_is_not_an_image_decodes_to_nothing() {
    assert!(decode_avatar(b"<html>not an avatar</html>").is_none());
}

#[test]
fn a_repeated_avatar_url_does_not_refetch() {
    let controller = AvatarController::default();
    assert!(matches!(controller.announce(Some("a".into())), AvatarUpdate::Fetch(..)));
    assert!(matches!(controller.announce(Some("a".into())), AvatarUpdate::Unchanged));
}

#[test]
fn a_new_account_supersedes_an_in_flight_avatar() {
    let controller = AvatarController::default();
    let first = controller.announce(Some("a".into()));
    assert!(matches!(first, AvatarUpdate::Fetch(..)));
    let AvatarUpdate::Fetch(first, _) = first else { return };
    let second = controller.announce(Some("b".into()));
    assert!(matches!(second, AvatarUpdate::Fetch(..)));
    let AvatarUpdate::Fetch(second, _) = second else { return };
    assert_ne!(first, second);
    assert!(!controller.is_current(first));
    assert!(controller.is_current(second));
}

#[test]
fn signing_out_supersedes_an_in_flight_avatar() {
    let controller = AvatarController::default();
    let first = controller.announce(Some("a".into()));
    assert!(matches!(first, AvatarUpdate::Fetch(..)));
    let AvatarUpdate::Fetch(first, _) = first else { return };
    let second = controller.announce(None);
    assert!(matches!(second, AvatarUpdate::Clear(..)));
    let AvatarUpdate::Clear(second) = second else { return };
    assert!(!controller.is_current(first));
    assert!(controller.is_current(second));
}
