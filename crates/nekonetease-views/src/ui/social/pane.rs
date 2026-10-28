//! Which of the messages surface's two panes faces at a given card width.
//!
//! The breakpoint is `messages-view.slint`'s own `two-pane-min-width`, since it is a layout fact
//! about the card. What the view asks Rust is the one-pane facing rule below it, through
//! `Social.messages-thread-front` — kept here so both directions of a breakpoint flip are testable
//! without a window.

/// Whether the thread is the pane facing while the card is too narrow for both.
///
/// A picked peer holds the thread in front until the Back control puts the list back. The peer
/// itself is `Social.peer-user-id`, which no width reads and this rule never writes.
pub(super) fn thread_front(peer_picked: bool, list_front: bool) -> bool {
    peer_picked && !list_front
}

#[cfg(test)]
#[path = "tests/pane_tests.rs"]
mod tests;
