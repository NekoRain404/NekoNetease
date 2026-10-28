//! Source pin for the NekoNetease logo mounted in `components/custom-titlebar.slint`.

use nekonetease_testkit::strip_line_comments;

const TITLEBAR: &str =
    include_str!("../../../../nekonetease-ui/ui/components/custom-titlebar.slint");

/// The bar declares exactly one `Image`, so its body needs no disambiguation.
fn logo_mount() -> String {
    let code = strip_line_comments(TITLEBAR);
    let rest = code.split_once("Image {").map_or("", |(_, rest)| rest);
    assert!(
        !rest.is_empty(),
        "`custom-titlebar.slint` no longer mounts an `Image` — every assertion over that body \
         would walk an empty string and pass"
    );

    let mut depth = 1usize;
    let mut body: Vec<&str> = Vec::new();
    for line in rest.lines() {
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        if depth == 0 {
            break;
        }
        body.push(line);
    }
    body.join("\n")
}

#[test]
fn the_titlebar_uses_the_supplied_logo_without_recolouring_it() {
    let mount = logo_mount();
    assert!(
        mount.contains("nekonetease.png"),
        "the bar's only `Image` is no longer the logo — this pin is reading the wrong element"
    );
    assert!(
        !mount.contains("colorize:"),
        "the supplied logo contains its own colours and background"
    );
}
