//! What a queue does when the library underneath it loses a row.
//!
//! A delete is a hard `DELETE`, so the watcher can take a track out from under a playing queue at
//! any moment. The transport removes missing queue entries and moves on when the current row goes.

use super::*;
use crate::library::playback::player_play_tracks;
use crate::state::fixtures::TestPlayback;
use nekonetease_core::error::AppError;
use nekonetease_engine::player::engine::types::PlaybackStatus;

/// A fixture playing `count` staged files from the head, as the header Play pill starts one.
async fn playing(count: usize) -> Result<(TestPlayback, Vec<i64>), AppError> {
    let fx = TestPlayback::empty().await?;
    let ids = fx.stage_playable(count).await?;
    player_play_tracks(&fx.ctx, ids.clone(), None).await?;
    Ok((fx, ids))
}

/// The watcher's own delete: a hard `DELETE`, with nothing left behind to find.
async fn delete_row(fx: &TestPlayback, id: i64) -> Result<(), AppError> {
    sqlx::query("DELETE FROM tracks WHERE id = ?").bind(id).execute(fx.ctx.db.write()).await?;
    Ok(())
}

fn queued_ids(fx: &TestPlayback) -> Vec<i64> {
    lock_state(&fx.ctx.player_state).queue.tracks.iter().map(|t| t.id).collect()
}

/// The base case. A queue holding an id no row answers offers the user a track that cannot be
/// loaded, and every surface below it renders the row it was queued with.
#[tokio::test]
async fn a_queued_track_whose_row_is_gone_drops_out() -> Result<(), AppError> {
    let (fx, ids) = playing(3).await?;
    delete_row(&fx, ids[2]).await?;

    reconcile_once(&fx.ctx.db, &fx.ctx.player_state, &fx.ctx.sinks, &fx.ctx.engine).await?;

    assert_eq!(queued_ids(&fx), ids[..2], "only the row that went away leaves the queue");
    Ok(())
}

/// The entry being played is the one that cannot simply be dropped: the transport is pointing at
/// it, and leaving it there strands playback on an id that resolves to nothing.
#[tokio::test]
async fn the_current_entry_going_missing_advances_to_the_next_survivor() -> Result<(), AppError> {
    let (fx, ids) = playing(3).await?;
    delete_row(&fx, ids[0]).await?;

    reconcile_once(&fx.ctx.db, &fx.ctx.player_state, &fx.ctx.sinks, &fx.ctx.engine).await?;

    let state = lock_state(&fx.ctx.player_state);
    assert_eq!(
        state.current_track().map(|t| t.id),
        Some(ids[1]),
        "playback moves on rather than stopping, the queue still having somewhere to go"
    );
    assert_eq!(state.status, PlaybackStatus::Playing);
    Ok(())
}

/// With nothing left to advance to, the deck is cleared as well as stopped — the now-playing bar
/// projects `source`, so leaving it seated goes on showing a track the queue no longer holds.
#[tokio::test]
async fn the_last_surviving_entry_going_missing_clears_the_deck() -> Result<(), AppError> {
    let (fx, ids) = playing(1).await?;
    delete_row(&fx, ids[0]).await?;

    reconcile_once(&fx.ctx.db, &fx.ctx.player_state, &fx.ctx.sinks, &fx.ctx.engine).await?;

    let state = lock_state(&fx.ctx.player_state);
    assert_eq!(state.queue.tracks, []);
    assert!(state.source.is_none(), "a cleared queue has to clear what the bar is drawing");
    assert_eq!(state.status, PlaybackStatus::Stopped);
    Ok(())
}

/// The pass runs on every `library_changed` bump — a favourite toggle, an import, any scan. With
/// nothing missing it must publish nothing, or the queue sheet rebuilds its rows for every one.
#[tokio::test]
async fn a_queue_with_nothing_missing_publishes_nothing() -> Result<(), AppError> {
    let (fx, _ids) = playing(3).await?;
    let queue_rx = fx.ctx.sinks.queue.subscribe();

    reconcile_once(&fx.ctx.db, &fx.ctx.player_state, &fx.ctx.sinks, &fx.ctx.engine).await?;

    assert_eq!(
        queue_rx.has_changed().ok(),
        Some(false),
        "a pass that found nothing must not re-emit the queue"
    );
    Ok(())
}
