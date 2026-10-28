use super::{cloud_song_row, patch_like};
use nekonetease_core::entities::cloud::Song;
use nekonetease_ui::TrackListRow;
use slint::{Model, ModelRc, VecModel};

const CLOUD_MENU: &str = include_str!(
    "../../../../../nekonetease-ui/ui/components/track-list/cloud-track-context-menu.slint"
);
const TRACK_ROW: &str =
    include_str!("../../../../../nekonetease-ui/ui/components/track-list/track-list-row.slint");

fn row(id: &str) -> TrackListRow {
    TrackListRow { id: i32::MAX, remote_id: id.into(), ..TrackListRow::default() }
}

#[test]
fn liking_one_large_cloud_id_leaves_other_clamped_rows_unchanged() {
    let rows =
        ModelRc::new(VecModel::from(vec![row("2147483647"), row("2147483648"), row("3000000000")]));

    patch_like(&rows, 2_147_483_648, true);

    assert_eq!(rows.iter().map(|row| row.is_favorite).collect::<Vec<_>>(), [false, true, false]);
}

#[test]
fn unliking_an_unknown_id_does_not_modify_the_model() {
    let rows = ModelRc::new(VecModel::from(vec![
        TrackListRow { is_favorite: true, ..row("2147483648") },
        TrackListRow { is_favorite: true, ..row("3000000000") },
    ]));

    patch_like(&rows, 4_000_000_000, false);

    assert_eq!(rows.iter().map(|row| row.is_favorite).collect::<Vec<_>>(), [true, true]);
}

#[test]
fn a_song_without_a_music_video_leaves_the_menu_field_empty() {
    let row = cloud_song_row(Song { mv: 0, ..Song::default() }, 0, false);

    assert_eq!(row.mv_id, None);
    let ui = crate::ui::tracks::to_slint_track_list_rows(std::slice::from_ref(&row));
    assert_eq!(ui.first().map(|row| row.mv_remote_id.as_str()), Some(""));
}

#[test]
fn a_wide_music_video_id_reaches_the_row_as_exact_decimal_text() {
    // Above 2^32, so a Slint `int` would clamp it.
    let wide = 5_436_712_345_678_901_i64;
    let row = cloud_song_row(Song { mv: wide, ..Song::default() }, 0, false);

    assert_eq!(row.mv_id, Some(wide));
    let ui = crate::ui::tracks::to_slint_track_list_rows(std::slice::from_ref(&row));
    assert_eq!(ui.first().map(|row| row.mv_remote_id.as_str()), Some("5436712345678901"));
}

#[test]
fn the_music_video_badge_is_gated_on_the_rows_own_id() {
    assert!(
        TRACK_ROW.contains(r#"root.row-data.mv_remote_id != """#),
        "the badge must be shown only for a row that carries an MV"
    );
    assert!(
        TRACK_ROW.contains("CloudTrackActions.open-mv(root.row-data.mv_remote_id)"),
        "the badge must hand the row's own MV id to the action"
    );
    assert!(
        !CLOUD_MENU.contains("open-mv"),
        "the badge is the one entry now, so the context menu must not keep a second"
    );
}
