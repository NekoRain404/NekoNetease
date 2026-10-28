//! `EntityCard`'s overlay slot, asked of every host that opens it.
//!
//! A control a host puts in that slot is drawn inside the card's artwork square but *written*
//! inside the host's own `EntityCard { … }` block, and Slint resolves names where they are
//! written. So `parent` there is the card, not the square: a control using `card-height - …` would
//! land `card-height - tile-size` below where it reads, with a source that reads exactly like the
//! one that was right.

use nekonetease_testkit::{MIN_SLINT_SOURCES, UI_DIR, blocks_named, stripped_sources};

/// What a host writes to bind the slot's visibility, and how the walk below finds the hosts rather
/// than naming them — a third one is covered the day it is written.
const OPENS_THE_SLOT: &str = "show-overlay-actions:";

/// The component the pins below read. [`blocks_named`] takes the name followed by `{`, so the
/// card's own `EntityCard inherits Rectangle {` declaration is not a mount.
const CARD: &str = "EntityCard";

/// Every `EntityCard { … }` block in the tree that opens the overlay slot, paired with its file.
///
/// **The mount's own braces rather than the whole file**, since a host is free to spell `parent`
/// anywhere else in it: `x: parent.width - self.width` is the canonical `OverlayScrollbar` mount,
/// so a file-wide ban would fail the first overlay host that also carries a scrollbar and the
/// repair would be to weaken the pin.
fn overlay_mounts() -> Vec<(String, String)> {
    let mut mounts: Vec<(String, String)> = Vec::new();

    for (path, src) in stripped_sources(UI_DIR, "slint", MIN_SLINT_SOURCES) {
        for body in blocks_named(&src, CARD) {
            if body.contains(OPENS_THE_SLOT) {
                mounts.push((path.clone(), body.to_owned()));
            }
        }
    }

    assert!(
        !mounts.is_empty(),
        "nothing mounts `{OPENS_THE_SLOT}` any more, so the pins below measure an empty set"
    );
    mounts
}

/// The offending files, named once each however many mounts they carry.
fn files_of(mounts: Vec<(String, String)>) -> Vec<String> {
    let mut paths: Vec<String> = mounts.into_iter().map(|(path, _)| path).collect();
    paths.sort();
    paths.dedup();
    paths
}

/// The mistake is invisible in the source and only the generated tree tells the two apart, so the
/// rule is the blunt one: nothing inside the mount names `parent` at all. Both of today's frame
/// the square through `EntityCard`'s published `tile-size`, which is what the card exports it for.
#[test]
fn no_overlay_host_positions_against_parent() {
    let against_parent = files_of(
        overlay_mounts().into_iter().filter(|(_, body)| body.contains("parent.")).collect(),
    );

    assert!(
        against_parent.is_empty(),
        "{against_parent:?} put controls in `EntityCard`'s overlay slot and position against \
         `parent` — which resolves to the card, not to the artwork square the slot is drawn in. \
         Frame them off the card's `tile-size` instead"
    );
}

/// A host reading the slot's hover back is the other half of the deal: the card's `touch` goes
/// false under a higher-z button, and only the host that named those buttons can say so.
#[test]
fn every_overlay_host_answers_for_its_own_hover() {
    let silent = files_of(
        overlay_mounts()
            .into_iter()
            .filter(|(_, body)| !body.contains("overlay-hovered:"))
            .collect(),
    );

    assert!(
        silent.is_empty(),
        "{silent:?} put controls in the overlay slot without passing `overlay-hovered` — the \
         card's fill and cover zoom drop out while the pointer is on one of them"
    );
}
