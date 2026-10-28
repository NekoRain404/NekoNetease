use super::thread_front;

const MESSAGES_VIEW: &str =
    include_str!("../../../../../nekonetease-ui/ui/views/social/messages-view.slint");
const DECLARATION: &str = "property <length> two-pane-min-width:";

/// The breakpoint the view actually branches on, read back so the flip cases cannot drift from it.
fn breakpoint() -> Option<f32> {
    let line = MESSAGES_VIEW.lines().find(|line| line.contains(DECLARATION))?;
    let (_, value) = line.split_once(DECLARATION)?;
    value.trim().trim_end_matches(';').strip_suffix("px")?.parse().ok()
}

fn flat_source() -> String {
    MESSAGES_VIEW.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the view assigns `needle`, told apart from the comparisons a read sits in.
fn assigns(source: &str, needle: &str) -> bool {
    source.match_indices(needle).any(|(index, _)| {
        let rest = source[index + needle.len()..].trim_start();
        rest.starts_with('=') && !rest.starts_with("==")
    })
}

#[test]
fn the_card_breaks_on_its_own_width_and_mounts_one_pane_below_it() {
    assert_eq!(breakpoint(), Some(540.0));
    let source = flat_source();
    assert!(source.contains("property <bool> two-pane: root.width >= root.two-pane-min-width;"));
    assert!(source.contains("if root.two-pane || !root.thread-front: Rectangle {"));
    assert!(source.contains("if root.two-pane || root.thread-front: Rectangle {"));
}

#[test]
fn the_back_control_returns_to_the_list_without_dropping_the_peer() {
    let source = flat_source();
    assert!(source.contains(
        "if !root.two-pane: IconButton { icon: \"arrow_back\"; \
         tooltip-text: @tr(\"Back\"); tooltip-side: TooltipSide.below;"
    ));
    assert!(source.contains("clicked => { root.list-front = true; }"));
    // The peer is `Social.peer-user-id`; a one-pane navigation must never write it.
    assert!(!assigns(&source, "Social.peer-user-id"));
    assert!(!assigns(&source, "Social.open-conversation"));
}

#[test]
fn a_breakpoint_flip_keeps_the_picked_thread_in_both_directions() {
    let declared = breakpoint();
    assert_eq!(declared, Some(540.0), "the flip cases read the view's own breakpoint");
    let Some(breakpoint) = declared else { return };

    // (card width, Back pressed, thread mounted). The peer stays picked throughout, so a row moves
    // only the width around the breakpoint, or — below it — the pane Back left in front.
    let cases = [
        (breakpoint - 1.0, false, true),
        (breakpoint, false, true),
        (breakpoint + 1.0, false, true),
        (breakpoint - 1.0, true, false),
        (breakpoint, true, true),
        (breakpoint + 1.0, true, true),
        (breakpoint - 1.0, false, true),
    ];
    for (width, list_front, thread_mounted) in cases {
        let two_pane = width >= breakpoint;
        assert_eq!(
            thread_mounted,
            two_pane || thread_front(true, list_front),
            "at {width}px with list-front {list_front}"
        );
    }
}
