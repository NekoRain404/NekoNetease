//! The sheet card against the window's resize floor: a 400 px minimum wider than the panel it is
//! drawn in clips both edges at the narrowest window the app allows.

const QUEUE_SHEET: &str = include_str!("../../../../../nekonetease-ui/ui/views/queue-sheet.slint");
const SHELL: &str = include_str!("../../../../../nekonetease-ui/ui/globals/shell.slint");

#[test]
fn the_sheet_card_shrinks_to_the_panel_instead_of_overflowing_it() {
    let source = QUEUE_SHEET.split_whitespace().collect::<Vec<_>>().join(" ");

    // The 400 px floor is clamped to the panel less a gutter on both sides, so the card cannot
    // outgrow the panel at any window width the shell allows.
    assert!(source.contains(
        "max(min(400px, max(0px, parent.width - 2 * Theme.pad-md)), parent.width * 0.5)"
    ));
    // The unclamped floor is what clipped both edges at a 350 px window.
    assert!(!source.contains("max(400px, parent.width * 0.5)"));

    assert!(SHELL.contains("out property <length> window-min-width: 350px;"));
}
