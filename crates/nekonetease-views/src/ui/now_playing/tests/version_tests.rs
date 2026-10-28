use super::{MAX_VERSION_CHARS, SEPARATOR, version_text};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn the_aliases_join_in_the_order_the_wire_gave_them() {
    let text = version_text("Song", &names(&["Live", "Remastered"]), &[]);

    assert_eq!(text, "Live · Remastered");
}

#[test]
fn translated_names_follow_the_aliases() {
    let text = version_text("Song", &names(&["Live"]), &names(&["现场版"]));

    assert_eq!(text, "Live · 现场版");
}

#[test]
fn a_name_the_title_already_states_is_dropped() {
    let text = version_text("Song", &names(&["Song", "Remastered"]), &names(&["Song"]));

    assert_eq!(text, "Remastered");
}

#[test]
fn repeats_and_blank_names_leave_nothing_doubled() {
    let text = version_text("Song", &names(&["Live", " Live ", ""]), &names(&["Live"]));

    assert_eq!(text, "Live");
}

#[test]
fn a_song_with_no_other_names_leaves_the_line_empty() {
    assert_eq!(version_text("Song", &[], &[]), "");
}

#[test]
fn a_line_exactly_at_the_budget_is_kept_whole() {
    let exact = "z".repeat(MAX_VERSION_CHARS);

    assert_eq!(version_text("Song", std::slice::from_ref(&exact), &[]), exact);
}

#[test]
fn a_name_past_the_budget_is_cut_rather_than_stretching_the_menu() {
    let long = "x".repeat(MAX_VERSION_CHARS + 6);
    let text = version_text("Song", std::slice::from_ref(&long), &[]);

    assert_eq!(text.chars().count(), MAX_VERSION_CHARS + SEPARATOR.chars().count() + 1);
    assert!(text.ends_with("…"), "{text}");
}

#[test]
fn names_after_the_budget_is_spent_are_dropped_and_marked() {
    let long = "y".repeat(MAX_VERSION_CHARS - 4);
    let text = version_text("Song", &[long.clone(), "Remastered".to_owned()], &[]);

    assert_eq!(text, format!("{long} · …"));
}
