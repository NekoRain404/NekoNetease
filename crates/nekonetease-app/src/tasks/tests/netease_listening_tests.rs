use super::*;

fn sample(position_ms: u64) -> ListenSample {
    ListenSample {
        key: Some(ListenKey { song_id: 42, playback_generation: 1, account_generation: 7 }),
        playing: true,
        position_ms,
        speed: 1.0,
        decoder_generation: 1,
    }
}

fn tick(tracker: &mut ListenTracker, sample: ListenSample) -> Option<ListenReport> {
    tracker.observe(sample, Duration::from_millis(500))
}

#[test]
fn actual_progress_is_reported_once_on_stop() {
    let mut tracker = ListenTracker::default();
    for position in [0, 500, 1000, 1500, 2000] {
        assert_eq!(tick(&mut tracker, sample(position)), None);
    }
    let stopped = ListenSample { key: None, playing: false, ..sample(0) };
    assert_eq!(tick(&mut tracker, stopped).map(|report| report.seconds), Some(2));
    assert_eq!(tracker.finish(), None);
}

#[test]
fn pause_buffering_and_seeking_do_not_inflate_time() {
    let mut tracker = ListenTracker::default();
    for position in [0, 500, 1000, 1000, 1000, 120_000, 120_500] {
        let _ = tick(&mut tracker, sample(position));
    }
    for _ in 0..20 {
        let _ = tick(&mut tracker, ListenSample { playing: false, ..sample(120_500) });
    }
    let _ = tick(&mut tracker, sample(120_500));
    let _ = tick(&mut tracker, sample(121_000));
    let _ = tick(&mut tracker, sample(0));
    assert_eq!(tracker.finish().map(|report| report.seconds), Some(2));
}

#[test]
fn repeats_and_account_changes_keep_separate_identities() {
    let mut tracker = ListenTracker::default();
    for position in [0, 500, 1000] {
        let _ = tick(&mut tracker, sample(position));
    }
    let repeated = ListenSample {
        key: Some(ListenKey { song_id: 42, playback_generation: 2, account_generation: 7 }),
        ..sample(0)
    };
    let first = tick(&mut tracker, repeated);
    assert_eq!(first.map(|report| (report.key.playback_generation, report.seconds)), Some((1, 1)));
    let _ = tick(&mut tracker, ListenSample { position_ms: 500, ..repeated });
    let _ = tick(&mut tracker, ListenSample { position_ms: 1000, ..repeated });
    let switched = ListenSample {
        key: Some(ListenKey { song_id: 42, playback_generation: 2, account_generation: 8 }),
        ..sample(1000)
    };
    let old_account = tick(&mut tracker, switched);
    assert_eq!(old_account.map(|report| report.key.account_generation), Some(7));
    assert_eq!(tracker.finish(), None);
}

#[test]
fn resume_offset_and_suspend_are_not_listening_time() {
    let mut tracker = ListenTracker::default();
    let _ = tick(&mut tracker, sample(120_000));
    let _ = tracker.observe(sample(180_000), Duration::from_mins(1));
    let _ = tick(&mut tracker, sample(180_500));
    let _ = tick(&mut tracker, sample(181_000));
    assert_eq!(tracker.finish().map(|report| report.seconds), Some(1));
}

#[test]
fn playback_speed_counts_wall_time_and_short_skips_are_ignored() {
    let mut tracker = ListenTracker::default();
    for position in [0, 1000, 2000, 3000, 4000] {
        let _ = tick(&mut tracker, ListenSample { speed: 2.0, ..sample(position) });
    }
    assert_eq!(tracker.finish().map(|report| report.seconds), Some(2));
    let _ = tick(&mut tracker, sample(0));
    let _ = tick(&mut tracker, sample(500));
    assert_eq!(tracker.finish(), None);
}

#[test]
fn seek_and_quality_decoder_swaps_preserve_one_listen_without_counting_the_jump() {
    let mut tracker = ListenTracker::default();
    for position in [0, 500, 1000] {
        let _ = tick(&mut tracker, sample(position));
    }
    for position in [1500, 2000, 2500] {
        assert_eq!(
            tick(&mut tracker, ListenSample { decoder_generation: 2, ..sample(position) }),
            None
        );
    }
    assert_eq!(tracker.finish().map(|report| report.seconds), Some(2));
}
