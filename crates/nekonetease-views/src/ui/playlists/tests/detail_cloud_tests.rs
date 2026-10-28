use super::cloud_playlist_owned;
use nekonetease_core::entities::cloud::{Account, Playlist as CloudPlaylist, PlaylistCreator};

fn cloud_playlist(creator: Option<i64>, subscribed: bool) -> CloudPlaylist {
    CloudPlaylist {
        creator: creator.map(|id| PlaylistCreator { id, ..Default::default() }),
        subscribed,
        ..Default::default()
    }
}

#[test]
fn cloud_playlist_ownership_prefers_the_creator_then_the_subscribed_flag() {
    let account = Account { id: 7, ..Default::default() };
    assert!(cloud_playlist_owned(&cloud_playlist(Some(7), false), Some(&account)));
    assert!(!cloud_playlist_owned(&cloud_playlist(Some(8), false), Some(&account)));
    assert!(!cloud_playlist_owned(&cloud_playlist(Some(7), false), None));
    assert!(cloud_playlist_owned(&cloud_playlist(None, false), None));
    assert!(!cloud_playlist_owned(&cloud_playlist(None, true), None));
}
