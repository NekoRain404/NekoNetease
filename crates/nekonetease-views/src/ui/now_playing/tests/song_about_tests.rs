use nekonetease_core::entities::song_about::SongAbout;

use super::{MAX_ABOUT_CHARS, about_message};

fn about(facts: &[&str], reviews: &[&str]) -> SongAbout {
    SongAbout {
        facts: facts.iter().map(|fact| (*fact).to_owned()).collect(),
        reviews: reviews.iter().map(|line| (*line).to_owned()).collect(),
    }
}

#[test]
fn facts_and_reviews_are_one_block_with_the_reviews_below() {
    let message = about_message(&about(&["流行", "国语"], &["第一段", "第二段"]));
    assert_eq!(message, "流行 · 国语\n\n第一段\n\n第二段");
}

#[test]
fn a_page_with_nothing_to_show_composes_nothing() {
    assert_eq!(about_message(&SongAbout::default()), "");
    assert_eq!(about_message(&about(&[], &["只有一段"])), "只有一段");
    assert_eq!(about_message(&about(&["国语"], &[])), "国语");
}

#[test]
fn a_long_page_is_cut_at_the_character_budget() {
    let long = "字".repeat(MAX_ABOUT_CHARS + 1);
    let message = about_message(&about(&[], &[long.as_str()]));
    assert_eq!(message.chars().count(), MAX_ABOUT_CHARS);
    assert!(message.ends_with('…'));

    let exact = "字".repeat(MAX_ABOUT_CHARS);
    assert_eq!(about_message(&about(&[], &[exact.as_str()])), exact);
}

const OVERFLOW_MENU: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/now-playing/overflow-menu.slint");

/// The row is the only way in, so it owes both halves: the click that reaches the callback and the
/// height term that reserves its slot. A missing term paints the last row over the menu's edge.
#[test]
fn the_about_row_is_cloud_only_and_reserves_its_menu_height() {
    assert!(OVERFLOW_MENU.contains("Player.open-song-about()"));
    assert!(OVERFLOW_MENU.contains("if root.about-shown: OverflowRow"));
    assert!(
        OVERFLOW_MENU.contains(
            "property <bool> about-shown: Player.vm.has_track && Player.vm.track.file_path == \"\""
        ),
        "a local file has no wiki entry, so the row is hidden for it"
    );
    assert!(
        OVERFLOW_MENU.contains("(root.about-shown ? 1 : 0)"),
        "the menu is measured once as it opens, so the row's slot is one of the counted terms"
    );
}
