use slint::Model;

use super::{chart_row, chunk};
use nekonetease_core::entities::cloud::Playlist;

fn chart(id: i64, name: &str, tags: &[&str], cover: Option<&str>) -> Playlist {
    Playlist {
        id,
        name: name.to_owned(),
        cover_url: cover.map(str::to_owned),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        ..Playlist::default()
    }
}

#[test]
fn a_chart_row_carries_the_id_as_text_so_an_id_past_i32_survives() {
    let id = i64::from(i32::MAX) + 7;
    let row = chart_row(&chart(id, "Soaring", &[], None));
    let expected = id.to_string();
    assert_eq!(row.remote_id.as_str(), expected.as_str());
}

#[test]
fn a_chart_row_subtitles_every_tag_and_nothing_when_there_are_none() {
    let row = chart_row(&chart(1, "Soaring", &["华语", "流行"], None));
    assert_eq!(row.subtitle.as_str(), "华语 · 流行");
    let bare = chart_row(&chart(1, "Soaring", &[], None));
    assert_eq!(bare.subtitle.as_str(), "");
}

#[test]
fn a_chart_row_keeps_only_a_remote_cover() {
    let remote = chart_row(&chart(1, "Soaring", &[], Some("https://cdn.example/a.jpg")));
    assert_eq!(remote.artwork_path.as_str(), "https://cdn.example/a.jpg");

    // An entity row's cover tier fetches remote artwork and nothing else, so a local path
    // would name a file that fetcher cannot read.
    let absolute = chart_row(&chart(1, "Soaring", &[], Some("/music/a.jpg")));
    assert_eq!(absolute.artwork_path.as_str(), "");
    let relative = chart_row(&chart(1, "Soaring", &[], Some("artwork/abc.jpg")));
    assert_eq!(relative.artwork_path.as_str(), "");
    let none = chart_row(&chart(1, "Soaring", &[], None));
    assert_eq!(none.artwork_path.as_str(), "");
}

#[test]
fn rows_chunk_into_the_page_column_count() {
    let rows: Vec<_> = (0..5).map(|i| chart_row(&chart(i, "Soaring", &[], None))).collect();

    let grid = chunk(&rows, 2);

    assert_eq!(grid.len(), 3);
    let widths: Vec<usize> = grid.iter().map(|row| row.charts.row_count()).collect();
    assert_eq!(widths, [2, 2, 1]);
}
