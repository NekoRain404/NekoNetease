use super::{CARD_THUMB_SIZE, THUMB_SIZE, card_artwork_url, decode_thumbnail};
use image::{ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

#[test]
fn cards_keep_detail_and_aspect_ratio_while_rows_stay_small()
-> Result<(), Box<dyn std::error::Error>> {
    let source = RgbImage::from_pixel(800, 400, Rgb([40, 120, 200]));
    let mut bytes = Cursor::new(Vec::new());
    source.write_to(&mut bytes, ImageFormat::Png)?;
    let row = decode_thumbnail(bytes.get_ref(), THUMB_SIZE).ok_or("row decode failed")?;
    let card = decode_thumbnail(bytes.get_ref(), CARD_THUMB_SIZE).ok_or("card decode failed")?;
    assert_eq!((row.width(), row.height()), (72, 36));
    assert_eq!((card.width(), card.height()), (512, 256));
    Ok(())
}

#[test]
fn card_request_upgrades_a_small_cdn_size_without_dropping_other_query_values() {
    assert_eq!(
        card_artwork_url("https://p1.music.126.net/a.jpg?param=72y72&token=abc").as_deref(),
        Some("https://p1.music.126.net/a.jpg?token=abc&param=512y512")
    );
    assert_eq!(
        card_artwork_url("https://example.com/a.jpg?size=72").as_deref(),
        Some("https://example.com/a.jpg?size=72")
    );
}
