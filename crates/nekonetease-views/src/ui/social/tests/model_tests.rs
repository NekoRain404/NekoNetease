use nekonetease_ui::SocialEventRow;

use super::{append_unique, empty_rows, replace_rows, rows_of};

fn row(id: &str) -> SocialEventRow {
    SocialEventRow { id: id.into(), ..Default::default() }
}

#[test]
fn append_unique_drops_a_repeat() {
    let mut existing = vec![row("1"), row("2")];
    let added = append_unique(&mut existing, vec![row("2"), row("3")], |row| row.id.to_string());
    assert_eq!(added, 1);
    assert_eq!(existing.len(), 3);
    assert_eq!(existing[2].id, "3");
}

#[test]
fn append_unique_reports_a_page_of_repeats() {
    let mut existing = vec![row("1")];
    let added = append_unique(&mut existing, vec![row("1")], |row| row.id.to_string());
    assert_eq!(added, 0, "the paging footer reads this as the end of the list");
}

#[test]
fn rows_replace_in_the_model_the_surface_holds() {
    let model = empty_rows::<SocialEventRow>();
    replace_rows(&model, vec![row("1"), row("2")]);
    assert_eq!(rows_of(&model).len(), 2);
    replace_rows(&model, vec![row("3")]);
    let rows = rows_of(&model);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "3");
}
