//! The durable retry queue's file contract: what survives a reload, what is evicted, and what a
//! damaged file is allowed to do to the next launch.

use std::fs;

use nekonetease_core::error::AppError;
use tempfile::TempDir;

use super::*;

#[test]
fn a_pushed_entry_round_trips_through_the_queue_file() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    let queue = ListenQueue::open_at(path.clone(), 1_000);
    assert!(queue.push(123, 200, Some(7), 1_000));
    queue.flush()?;

    let raw = fs::read_to_string(&path)?;
    for expected in
        ["\"version\": 1", "\"song_id\": 123", "\"seconds\": 200", "\"captured_at_ms\": 1000"]
    {
        assert!(raw.contains(expected), "queue file is missing {expected}: {raw}");
    }

    let reloaded = ListenQueue::open_at(path, 2_000);
    let entries = reloaded.deliverable(7);
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].song_id, entries[0].seconds), (123, 200));
    Ok(())
}

#[test]
fn the_cap_evicts_the_oldest_entries_and_keeps_the_newest() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    let queue = ListenQueue::open_at(path.clone(), 0);
    let overflow = u64::try_from(CAPACITY).unwrap_or(u64::MAX) + 10;
    for index in 0_u64..overflow {
        let song_id = i64::try_from(index + 1).unwrap_or(i64::MAX);
        assert!(queue.push(song_id, 1, Some(1), 10_000 + index * 10));
    }
    assert_eq!(queue.deliverable(1).len(), CAPACITY);

    queue.flush()?;
    let reloaded = ListenQueue::open_at(path, 20_000);
    let entries = reloaded.deliverable(1);
    assert_eq!(entries.len(), CAPACITY);
    assert_eq!(entries.first().map(|entry| entry.song_id), Some(11));
    assert_eq!(
        entries.last().map(|entry| entry.song_id),
        Some(i64::try_from(overflow).unwrap_or(i64::MAX))
    );
    Ok(())
}

#[test]
fn load_drops_entries_older_than_seven_days() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    let now = MAX_AGE_MS + 1_000_000;
    let fresh = now - MAX_AGE_MS + 1;
    let json = format!(
        "{{\"version\":1,\"entries\":[\
         {{\"song_id\":1,\"seconds\":10,\"captured_at_ms\":0,\"account_id\":7}},\
         {{\"song_id\":2,\"seconds\":10,\"captured_at_ms\":{fresh},\"account_id\":7}}]}}"
    );
    fs::write(&path, json)?;

    let queue = ListenQueue::open_at(path, now);
    let entries = queue.deliverable(7);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].song_id, 2);
    Ok(())
}

#[test]
fn delivered_entries_are_removed_and_undelivered_ones_survive() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    let queue = ListenQueue::open_at(path.clone(), 0);
    assert!(queue.push(1, 30, Some(7), 100));
    assert!(queue.push(2, 30, Some(7), 200));
    assert!(queue.push(3, 30, Some(7), 300));

    let entries = queue.deliverable(7);
    assert_eq!(entries.iter().map(|entry| entry.song_id).collect::<Vec<_>>(), vec![1, 2, 3]);
    queue.remove(entries[1].seq);
    queue.flush()?;

    let reloaded = ListenQueue::open_at(path, 400);
    assert_eq!(
        reloaded.deliverable(7).iter().map(|entry| entry.song_id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    Ok(())
}

#[test]
fn a_corrupted_queue_file_reads_as_empty_and_can_be_rewritten() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    fs::write(&path, "{ not json")?;

    let queue = ListenQueue::open_at(path.clone(), 5);
    assert!(!queue.has_deliverable(1));
    assert!(queue.push(5, 60, Some(1), 5));
    queue.flush()?;

    let reloaded = ListenQueue::open_at(path, 10);
    assert_eq!(reloaded.deliverable(1).len(), 1);
    Ok(())
}

#[test]
fn another_accounts_entries_are_never_delivered() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let path = path_in(tmp.path());
    let queue = ListenQueue::open_at(path, 0);
    assert!(queue.push(1, 30, Some(7), 100));
    assert!(queue.push(2, 30, Some(8), 200));

    assert_eq!(queue.deliverable(7).iter().map(|entry| entry.song_id).collect::<Vec<_>>(), vec![1]);
    assert_eq!(queue.deliverable(8).iter().map(|entry| entry.song_id).collect::<Vec<_>>(), vec![2]);
    Ok(())
}

#[test]
fn consecutive_identical_reports_inside_the_window_collapse() -> Result<(), AppError> {
    let tmp = TempDir::new()?;
    let queue = ListenQueue::open_at(path_in(tmp.path()), 0);

    assert!(queue.push(1, 30, Some(7), 1_000));
    assert!(!queue.push(1, 30, Some(7), 3_000), "a repeat inside five seconds is one listen");
    assert!(queue.push(1, 30, Some(7), 9_000), "a later repeat is a new listen");
    assert!(queue.push(2, 30, Some(7), 9_100), "a different song is never collapsed");
    assert_eq!(queue.deliverable(7).len(), 3);
    Ok(())
}
