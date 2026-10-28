//! Source pins for `crates/nekonetease-ui/ui/components/icon-button.slint` — the tree's
//! most-mounted control, and one with no Rust module of its own.
//!
//! The first two pins below are halves of one fix: the glyph sits outside the disc and
//! places itself by hand. Each half is invisible on its own — the button looks
//! and behaves identically either way — and each is one plausible tidy-up away.
//!
//! The third walks every mount in the tree. A plain `Rectangle` exposes no accessible name,
//! so a mount that names itself with neither property reads as an unlabelled button, and
//! nothing in the build or the other two pins can see that.

use nekonetease_testkit::{
    MIN_SLINT_SOURCES, UI_DIR, block_body, strip_line_comments, stripped_sources,
};

const BUTTON: &str = include_str!("../../../../nekonetease-ui/ui/components/icon-button.slint");

fn code() -> String {
    strip_line_comments(BUTTON)
}

/// The body `opener` opens, cut at its own closing brace — a brace walk rather than a
/// split on `\n    }`, because the depth of both blocks this file reads is exactly what
/// the second pin is about: cut on the root's indent, a re-nested glyph's body runs to
/// `disc`'s closing brace and the first pin reads its bindings out of the very element it
/// was checking hadn't swallowed them.
fn block(code: &str, opener: &str) -> String {
    let rest = code.split_once(opener).map_or(String::new(), |(_, rest)| rest.to_owned());
    assert!(
        !rest.is_empty(),
        "`icon-button.slint` no longer declares `{opener}` — every assertion over that body \
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

/// The glyph places itself, and a centring layout would be a real regression.
/// `gen_layout_info_prop` folds a child's layout info into its parent's **unless the child
/// binds `x` or `y`**, which is the only reason `disc` has ever stayed out of it. A layout
/// binds neither, so it folds — and with it the glyph's size and the animated `press-scale`
/// behind it. Nothing moves, the root pinning `min == max == diameter`; what it costs is
/// every press dirtying this root's `layout_info` and re-solving the *host* layout once per
/// frame. The tell is a `+ …_layoutinfo_h` term in the generated `layout_info`.
#[test]
fn the_glyph_places_itself_rather_than_taking_a_layout() {
    let code = code();
    assert!(
        !code.contains("Layout {"),
        "`icon-button.slint` must declare no layout — a centring one folds its child's \
         layout info into the root and puts `press-scale` on the host's layout cache"
    );

    let glyph = block(&code, "MaterialIcon {");
    for axis in ["x:", "y:"] {
        assert!(
            glyph.contains(axis),
            "the glyph must bind `{axis}` — that binding is what keeps it out of the fold, so \
             dropping it is the same regression as reintroducing the layout"
        );
    }
}

/// The glyph is the disc's **sibling**, and that is what makes `fade` free.
/// `Opacity::need_layer` bails at exactly `1.0` and on a lone *childless* child, so
/// `disc`'s `opacity: root.fade` costs a canvas alpha multiply only while nothing is
/// nested inside it. Nest the glyph back — which reads as tidier, the two sharing a centre
/// and a `press-scale` — and every value under 1.0 renders the disc to an offscreen
/// texture, which under `LibraryTabBand`'s animating clip is a fresh GPU texture per frame
/// of every morph.
///
/// Nothing else covers it: `library_tab_band_tests` pins that the band fades the disc
/// through `IconButton`'s `fade` rather than its own `opacity`, which stays true — and
/// worthless — the moment the disc stops being childless.
#[test]
fn the_glyph_is_a_sibling_of_the_disc_rather_than_its_child() {
    let disc = block(&code(), "disc := Rectangle {");
    assert!(
        !disc.contains("MaterialIcon"),
        "the glyph must stay a sibling of `disc`: nested, the disc has a child, `need_layer` \
         stops bailing, and its `opacity: root.fade` allocates a texture per frame of a band \
         morph — the cost moving the glyph out is what removed"
    );
}

/// Vacuity floor for the mount walk below. Loose on purpose — the tree mounts around forty, and
/// one tight enough to notice an ordinary file deletion would trip first.
const MIN_ICON_BUTTON_MOUNTS: usize = 30;

/// The body of every `IconButton` *mount*, skipping the components that `inherits IconButton` —
/// the needle is the type's own name, and a declaration carries it too.
fn icon_button_mounts(src: &str) -> Vec<&str> {
    const NEEDLE: &str = "IconButton";
    let bytes = src.as_bytes();
    let mut out = Vec::new();

    for (at, _) in src.match_indices(NEEDLE) {
        if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
            continue;
        }
        if src[..at].split_whitespace().next_back() == Some("inherits") {
            continue;
        }
        let after = at + NEEDLE.len();
        let Some(open) = (after..bytes.len()).find(|i| !bytes[*i].is_ascii_whitespace()) else {
            continue;
        };
        if bytes[open] == b'{'
            && let Some(body) = block_body(src, open)
        {
            out.push(body);
        }
    }
    out
}

/// Every `IconButton` mount names itself, either with the tooltip a sighted user already reads
/// or with `a11y-label` where a card or dialog would clip that pill. Without one the control
/// exposes an empty `accessible-label`, which only a screen reader can see.
#[test]
fn every_icon_button_mount_carries_an_accessible_name() {
    let mut mounts = 0usize;
    let mut unlabelled = Vec::new();

    for (path, src) in stripped_sources(UI_DIR, "slint", MIN_SLINT_SOURCES) {
        for body in icon_button_mounts(&src) {
            mounts += 1;
            if !body.contains("tooltip-text:") && !body.contains("a11y-label:") {
                unlabelled.push(path.clone());
            }
        }
    }

    assert!(
        mounts >= MIN_ICON_BUTTON_MOUNTS,
        "only {mounts} `IconButton` mounts found — the walk's needle has stopped matching"
    );
    assert!(
        unlabelled.is_empty(),
        "`IconButton` mounts carrying neither `tooltip-text` nor `a11y-label` expose an empty \
         accessible name: {unlabelled:?}"
    );
}
