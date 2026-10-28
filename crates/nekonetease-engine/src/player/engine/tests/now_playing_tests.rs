use super::non_empty;

#[test]
fn whitespace_metadata_is_absent() {
    assert_eq!(non_empty(" \t\n"), None);
}

#[test]
fn metadata_is_trimmed_without_allocating() {
    assert_eq!(non_empty("  Artist  "), Some("Artist"));
}
