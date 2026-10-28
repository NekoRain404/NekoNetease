//! Tests for the favorite writers and the currently-playing track cache.

use std::sync::Arc;

use super::*;
use crate::state::fixtures::test_sinks;
use nekonetease_engine::player::engine::fixtures::test_track;
use nekonetease_store::database::queries::fixtures::insert_test_track;

struct Fixture {
    write: FavoriteWrite,
}

impl Fixture {
    async fn new() -> Result<Self, AppError> {
        let db = DbPool::test_pool().await?;
        queries::folder::insert_folder(&db, "/music", true).await?;
        Ok(Self {
            write: FavoriteWrite {
                db,
                player_state: Arc::new(PlayerStateHandle::default()),
                sinks: Arc::new(test_sinks()),
                library_changed: Signal::new(),
            },
        })
    }

    async fn track(&self, name: &str) -> Result<i64, AppError> {
        insert_test_track(&self.write.db, &format!("/music/{name}.mp3"), name, "Artist", "A", "R")
            .await
    }

    fn seat(&self, id: i64, favorite: bool) {
        let mut track = (*test_track("Song", Some("Artist"), None)).clone();
        track.id = id;
        track.is_favorite = favorite;
        lock_state(&self.write.player_state).source = Some(Arc::new(track));
    }

    fn deck_favorite(&self) -> Option<bool> {
        lock_state(&self.write.player_state).current_track().map(|t| t.is_favorite)
    }

    async fn row_favorite(&self, id: i64) -> Result<bool, AppError> {
        let flag: i64 = sqlx::query_scalar("SELECT is_favorite FROM tracks WHERE id = ?")
            .bind(id)
            .fetch_one(self.write.db.read())
            .await?;
        Ok(flag != 0)
    }
}

#[tokio::test]
async fn a_favorite_write_marks_every_row_it_was_given() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let first = fixture.track("first").await?;
    let second = fixture.track("second").await?;

    fixture.write.set(&[first, second], true).await?;

    assert!(fixture.row_favorite(first).await?);
    assert!(fixture.row_favorite(second).await?);
    Ok(())
}

#[tokio::test]
async fn a_favorite_write_wakes_the_library_subscribers() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let id = fixture.track("song").await?;
    let changed = fixture.write.library_changed.subscribe();

    fixture.write.set(&[id], true).await?;

    assert_eq!(changed.has_changed().ok(), Some(true));
    Ok(())
}

#[tokio::test]
async fn a_favorite_write_mirrors_the_flag_onto_the_playing_track() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let id = fixture.track("song").await?;
    fixture.seat(id, false);

    fixture.write.set(&[id], true).await?;

    assert_eq!(fixture.deck_favorite(), Some(true));
    Ok(())
}

#[tokio::test]
async fn a_favorite_write_leaves_a_playing_track_outside_the_set_alone() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let playing = fixture.track("playing").await?;
    let listed = fixture.track("listed").await?;
    fixture.seat(playing, false);

    fixture.write.set(&[listed], true).await?;

    assert_eq!(fixture.deck_favorite(), Some(false));
    assert!(fixture.row_favorite(listed).await?);
    Ok(())
}

#[tokio::test]
async fn toggling_with_nothing_on_the_deck_writes_nothing() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let changed = fixture.write.library_changed.subscribe();

    let toggled = fixture.write.toggle_current().await?;

    assert_eq!(toggled, None);
    assert_eq!(changed.has_changed().ok(), Some(false));
    Ok(())
}

#[tokio::test]
async fn toggling_the_playing_track_flips_the_row_and_the_cached_copy() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let id = fixture.track("song").await?;
    fixture.seat(id, false);

    let toggled = fixture.write.toggle_current().await?;

    assert_eq!(toggled, Some((id, true)));
    assert!(fixture.row_favorite(id).await?);
    assert_eq!(fixture.deck_favorite(), Some(true));
    Ok(())
}

#[tokio::test]
async fn toggling_an_already_favorited_track_clears_it() -> Result<(), AppError> {
    let fixture = Fixture::new().await?;
    let id = fixture.track("song").await?;
    fixture.write.set(&[id], true).await?;
    fixture.seat(id, true);

    let toggled = fixture.write.toggle_current().await?;

    assert_eq!(toggled, Some((id, false)));
    assert!(!fixture.row_favorite(id).await?);
    Ok(())
}
