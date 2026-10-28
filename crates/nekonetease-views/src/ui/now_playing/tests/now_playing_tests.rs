use super::{SourceKey, Surfaces, clear_unshown_slot};
use nekonetease_engine::player::engine::now_playing::SourceId;
use nekonetease_ui::MiniLayout;
use std::cell::Cell;

/// The key the subscriber would have built for `id`, which is what `describes` answers without
/// building. Spelled here so the tests below assert the equivalence rather than restate the match.
fn key_for(id: SourceId) -> SourceKey {
    match id {
        SourceId::Track(track_id) => SourceKey::Track(track_id),
    }
}

#[test]
fn describes_answers_the_key_compare_it_replaced() {
    // The subscriber runs this on every player emit in place of building a `SourceKey` and
    // comparing it, so the two have to agree over the whole cross product or a source change is
    // either missed or re-applied every tick.
    let ids = [None, Some(SourceId::Track(1)), Some(SourceId::Track(2))];

    for held in ids {
        let held = held.map(key_for);
        for id in ids {
            assert_eq!(
                SourceKey::describes(held.as_ref(), id),
                held == id.map(key_for),
                "held {held:?} against {id:?}"
            );
        }
    }
}

const fn on_screen(
    open: bool,
    mini_visible: bool,
    mini_layout: MiniLayout,
    mini_backdrop: bool,
) -> Surfaces {
    Surfaces { open, mini_visible, mini_layout, mini_backdrop }
}

/// What is on screen beside what the two gates answer, `(surfaces, artwork, panel)`. The corners:
/// Now Playing alone and open under the strip, a hidden miniplayer whose mirrors still name a
/// layout, and every layout with its backdrop off and on.
const RENDERED: [(Surfaces, bool, bool); 9] = [
    (on_screen(true, false, MiniLayout::Strip, false), true, true),
    (on_screen(true, true, MiniLayout::Strip, false), true, true),
    (on_screen(false, false, MiniLayout::Column, true), false, false),
    (on_screen(false, true, MiniLayout::Strip, false), false, false),
    (on_screen(false, true, MiniLayout::Strip, true), true, false),
    (on_screen(false, true, MiniLayout::Card, false), true, false),
    (on_screen(false, true, MiniLayout::Card, true), true, false),
    (on_screen(false, true, MiniLayout::Column, false), true, true),
    (on_screen(false, true, MiniLayout::Column, true), true, true),
];

/// A `true` nothing paints spends a decode and a blur per track, and a `false` under a drawn layout
/// leaves the row tier's thumb, or no backdrop colours at all, until the next track.
#[test]
fn only_a_surface_drawing_the_artwork_asks_for_its_decode() {
    for (surfaces, artwork, _) in RENDERED {
        assert_eq!(surfaces.renders_artwork(), artwork, "{surfaces:?}");
    }
}

/// The strip on its backdrop is the row that tells this gate from the artwork one: it needs the
/// decode and has no slot for a sheet, which would otherwise cost a lookup per track.
#[test]
fn only_now_playing_and_the_column_mount_a_panel() {
    for (surfaces, _, panel) in RENDERED {
        assert_eq!(surfaces.renders_panel(), panel, "{surfaces:?}");
    }
}

/// The slot on show stays, so reopening on the same source paints without a decode, and the hidden
/// one is the memory handed back. Cleared the other way round, the reopen opens on an empty tile.
#[test]
fn a_release_empties_only_the_cross_fade_slot_not_on_show() {
    let cases = [
        // (has image, showing a), then (a emptied, b emptied)
        ((true, true), (false, true)),
        ((true, false), (true, false)),
        ((false, true), (true, true)),
        ((false, false), (true, true)),
    ];

    for ((has_image, use_a), expected) in cases {
        let (a_emptied, b_emptied) = (Cell::new(false), Cell::new(false));
        clear_unshown_slot(has_image, use_a, |_| a_emptied.set(true), |_| b_emptied.set(true));
        let emptied = (a_emptied.get(), b_emptied.get());
        assert_eq!(emptied, expected, "has image {has_image}, showing a {use_a}");
    }
}

const OVERFLOW_MENU: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/now-playing/overflow-menu.slint");
const BAR: &str = include_str!("../../../../../nekonetease-ui/ui/layout/now-playing-bar.slint");

/// A row left behind in the menu is a second way in that opens the thread with the menu still
/// standing, and its `menu-h` term would reserve an empty slot once the row is gone.
#[test]
fn the_comments_entry_lives_on_the_bar_and_not_in_the_overflow_menu() {
    assert!(!OVERFLOW_MENU.contains("open-comments"), "the menu must not keep a comments row");
    assert!(!OVERFLOW_MENU.contains("All comments"), "nor its label");
    assert!(!OVERFLOW_MENU.contains("has-comments"), "nor the menu-height term it counted");
    assert!(BAR.contains("Player.open-comments()"), "the bar's button opens the thread");
    assert!(
        BAR.contains(r#"Player.vm.has_track && Player.vm.track.file_path == """#),
        "a local source has no thread, so the button is hidden for it"
    );
    assert!(BAR.contains("@tr(\"All comments\")"), "the button carries the existing msgid");
}
