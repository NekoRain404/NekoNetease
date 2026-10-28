use std::path::Path;

use nekonetease_net::services::net::nos_upload::MAX_COVER_BYTES;

use super::{cover_format, validate_cover_size};

#[test]
fn cover_format_reads_the_picks_suffix() {
    for (path, extension, content_type) in [
        ("cover.jpg", "jpg", "image/jpeg"),
        ("cover.JPEG", "jpeg", "image/jpeg"),
        ("cover.png", "png", "image/png"),
        ("cover.WebP", "webp", "image/webp"),
    ] {
        let format = cover_format(Path::new(path));
        assert!(format.is_ok(), "{path} should be accepted");
        let Ok(format) = format else { continue };
        assert_eq!(format.extension, extension);
        assert_eq!(format.content_type, content_type);
    }
}

#[test]
fn cover_format_refuses_other_images_and_a_missing_suffix() {
    for path in ["cover.gif", "cover.bmp", "cover", "cover."] {
        assert!(cover_format(Path::new(path)).is_err(), "{path} should be refused");
    }
}

#[test]
fn cover_size_holds_the_bound_on_both_sides() {
    assert!(validate_cover_size(1).is_ok());
    assert!(validate_cover_size(MAX_COVER_BYTES).is_ok());
    assert!(validate_cover_size(MAX_COVER_BYTES + 1).is_err());
}

#[test]
fn an_empty_cover_is_refused_before_the_upload() {
    assert!(validate_cover_size(0).is_err());
}
