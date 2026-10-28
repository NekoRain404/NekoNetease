//! The frame hand-off: dimensions, the byte budget, and the refusal of a short buffer.

use super::{image_of, ms_to_i32};
use nekonetease_video::VideoFrame;

fn frame(width: u32, height: u32) -> VideoFrame {
    VideoFrame { width, height, pixels: vec![0x7f; width as usize * height as usize * 4] }
}

#[test]
fn an_rgba_frame_becomes_an_image_of_the_same_size() {
    let size = image_of(&frame(960, 540)).map(|image| (image.size().width, image.size().height));
    assert_eq!(size, Some((960, 540)));
}

#[test]
fn a_frame_with_no_dimensions_or_too_few_bytes_is_refused() {
    let mut short = frame(8, 8);
    short.pixels.truncate(short.byte_len() - 1);
    assert!(image_of(&short).is_none());
    assert!(image_of(&frame(0, 8)).is_none());
    assert!(image_of(&frame(8, 0)).is_none());
}

#[test]
fn a_progress_value_too_large_for_slint_saturates() {
    assert_eq!(ms_to_i32(0), 0);
    assert_eq!(ms_to_i32(90_000), 90_000);
    assert_eq!(ms_to_i32(u64::MAX), i32::MAX);
}
