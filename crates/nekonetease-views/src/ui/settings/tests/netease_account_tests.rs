use super::qr::login_qr_svg;

#[test]
fn a_login_key_renders_as_an_svg_document() {
    let svg = login_qr_svg("https://music.163.com/login?codekey=abc123");

    assert!(matches!(
        &svg,
        Ok(document) if document.contains("<svg") && document.contains("</svg>")
    ));
}

#[test]
fn the_rendered_code_carries_the_key_it_was_built_from() {
    let first = login_qr_svg("key-one");
    let second = login_qr_svg("key-two");

    assert!(matches!((&first, &second), (Ok(a), Ok(b)) if a != b));
}

#[test]
fn an_empty_key_still_produces_a_well_formed_document() {
    // The server is what decides a key is usable; the renderer must not fail on a shape it can encode.
    assert!(matches!(login_qr_svg(""), Ok(document) if document.contains("<svg")));
}
