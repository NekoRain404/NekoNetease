//! Process-isolated peak-RSS comparison for the old and scaled in-memory artwork paths.
//! Generate the fixture in a separate process so its allocations do not enter either peak.

use std::io::Write;
use std::path::Path;

use nekonetease_artwork::media::image::image_decode::{
    FilterType, MAX_SOURCE_DIM, decode_memory_capped, decode_memory_capped_to, encode_jpeg,
    resize_rgb8,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("expected fixture, full or scaled")?;
    let path = args.next().ok_or("expected image path")?;
    if mode == "fixture" {
        return write_fixture(Path::new(&path));
    }

    // Match the application's bounded global pool rather than scaling threads with the runner.
    rayon::ThreadPoolBuilder::new().num_threads(2).build_global()?;
    let bytes = std::fs::read(path)?;
    let decoded = match mode.as_str() {
        "full" => decode_memory_capped(&bytes, MAX_SOURCE_DIM),
        "scaled" => decode_memory_capped_to(&bytes, MAX_SOURCE_DIM, 72),
        _ => return Err("expected fixture, full or scaled".into()),
    }
    .ok_or("image decode failed")?;
    let thumbnail = resize_rgb8(&decoded, 72, 72, FilterType::Box).ok_or("resize failed")?;
    writeln!(
        std::io::stdout().lock(),
        "{mode}: decoded={}x{}, decoded_bytes={}, thumbnail_bytes={}",
        decoded.width(),
        decoded.height(),
        decoded.as_bytes().len(),
        thumbnail.as_raw().len(),
    )?;
    Ok(())
}

fn write_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let image = image::RgbImage::from_fn(4096, 4096, |x, y| {
        let channel = |value| u8::try_from(value).unwrap_or(255);
        image::Rgb([channel(x / 16), channel(y / 16), channel((x + y) / 32)])
    });
    std::fs::write(path, encode_jpeg(image, 90)?)?;
    Ok(())
}
