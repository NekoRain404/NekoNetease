//! Source pins for the play-count badge and the cards that carry it.
//!
//! Every card mounts the shared badge through `EntityCard`, keeping its markup and hide-at-zero
//! behavior in one place.

use nekonetease_testkit::strip_line_comments;

const BADGE: &str =
    include_str!("../../../../nekonetease-ui/ui/components/grid/play-count-badge.slint");
const ENTITY_CARD: &str =
    include_str!("../../../../nekonetease-ui/ui/components/grid/entity-card.slint");

/// The pill's own markup, as only the leaf may spell it. `border-color` rather than the fill,
/// which is a colour other overlays on the same tile legitimately share.
const PILL_MARKUP: &str = "border-color: Theme.surface2";

#[test]
fn entity_cards_mount_the_shared_badge() {
    let entity = strip_line_comments(ENTITY_CARD);
    assert!(
        entity.contains("PlayCountBadge {"),
        "`entity-card.slint` must mount `PlayCountBadge` so library cards can show play counts"
    );

    assert!(
        !entity.contains(PILL_MARKUP),
        "`entity-card.slint` spells the pill's own chrome — the badge is `grid/play-count-badge.slint`"
    );
}

/// The `> 0` suppression lives in the leaf, so a host passes a count and asks no question about
/// it. Re-adding the ternary is harmless on the day it is written and is how the rule ends up
/// stated in four places, one of which will eventually disagree.
#[test]
fn the_leaf_owns_the_hide_at_zero_rule() {
    assert!(
        strip_line_comments(BADGE).contains("visible: root.count > 0"),
        "`play-count-badge.slint` must hide itself at a count of zero — without it every host \
         owes the test, and a host that forgets paints an empty pill on an unplayed row"
    );
}
