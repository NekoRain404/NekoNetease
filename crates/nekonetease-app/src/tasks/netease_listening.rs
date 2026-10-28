//! Read-only listening tap and the durable retry queue behind it. Sampling never waits for HTTP
//! and never mutates the player.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nekonetease_core::error::describe;
use nekonetease_engine::player::engine::state::lock_state;
use nekonetease_engine::player::engine::types::PlaybackStatus;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cloud::CloudService;
use crate::state::AppState;
use crate::tasks::TaskSpawner;

mod listen_queue;
mod tracker;
use listen_queue::ListenQueue;
use tracker::{ListenKey, ListenReport, ListenSample, ListenTracker};

/// The retry pass waits this long after a transport or authentication failure, doubling to
/// [`RETRY_MAX`]. Both are a floor against hammering a service that is refusing, not a schedule.
const RETRY_INITIAL: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_mins(10);
/// A clean exit waits this long for the sampler to hand over its final report. The sampler
/// observes the same shutdown token, so only a stalled sampler spends the whole wait.
const EXIT_DRAIN: Duration = Duration::from_millis(1500);

#[derive(PartialEq, Eq)]
enum Drain {
    Delivered,
    Empty,
    Failed,
    Stopped,
}

pub fn spawn(spawner: &TaskSpawner, state: &AppState) {
    let queue = ListenQueue::open(listen_queue::path_in(&state.paths.data_dir));
    let notified = Arc::new(Notify::new());
    // Bounded memory when offline. What does not fit is persisted and retried from there.
    let (reports, pending) = mpsc::channel::<ListenReport>(32);
    let cloud = state.cloud.clone();

    spawn_worker(spawner, cloud.clone(), Arc::clone(&queue), Arc::clone(&notified), pending);
    spawn_retry(spawner, cloud, Arc::clone(&queue), Arc::clone(&notified));

    let state = state.clone();
    spawner.spawn_cancellable(move |shutdown| async move {
        let mut tracker = ListenTracker::default();
        let mut last = Instant::now();
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let exiting = tokio::select! {
                biased;
                () = shutdown.cancelled() => true,
                _ = interval.tick() => false,
            };
            let now = Instant::now();
            if let Some(report) = tracker.observe(sample(&state), now.duration_since(last)) {
                enqueue(&state.cloud, &reports, &queue, &notified, report);
            }
            last = now;
            if exiting {
                if let Some(report) = tracker.finish() {
                    enqueue(&state.cloud, &reports, &queue, &notified, report);
                }
                break;
            }
        }
    });
}

fn spawn_worker(
    spawner: &TaskSpawner,
    cloud: CloudService,
    queue: Arc<ListenQueue>,
    notified: Arc<Notify>,
    mut pending: mpsc::Receiver<ListenReport>,
) {
    spawner.spawn_cancellable(move |shutdown| async move {
        let mut dirty = false;
        loop {
            let report = tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                report = pending.recv() => report,
            };
            let Some(report) = report else { break };
            let queued = tokio::select! {
                biased;
                () = shutdown.cancelled() => persist(&cloud, &queue, report),
                queued = submit(&cloud, &queue, report) => queued,
            };
            if queued {
                notified.notify_one();
                dirty = true;
            }
            // Offline bursts arrive back to back; one write per burst, not one per sample.
            if dirty && pending.is_empty() {
                flush(&queue).await;
                dirty = false;
            }
        }
        // Everything still in the channel is a listen no attempt will be made for, so write it
        // before shutdown's budget closes the runtime.
        let deadline = tokio::time::sleep(EXIT_DRAIN);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                () = &mut deadline => break,
                report = pending.recv() => match report {
                    Some(report) => dirty |= persist(&cloud, &queue, report),
                    None => break,
                },
            }
        }
        if dirty {
            flush(&queue).await;
        }
    });
}

fn spawn_retry(
    spawner: &TaskSpawner,
    cloud: CloudService,
    queue: Arc<ListenQueue>,
    notified: Arc<Notify>,
) {
    spawner.spawn_cancellable(move |shutdown| async move {
        let mut status = cloud.subscribe();
        let mut backoff = RETRY_INITIAL;
        // Load-time expiry and cap eviction live only in memory until the first write.
        flush(&queue).await;
        loop {
            let mut failed = false;
            if let Some(account_id) = cloud.status().account.map(|account| account.id)
                && queue.has_deliverable(account_id)
            {
                failed = drain(&cloud, &queue, account_id, &shutdown).await == Drain::Failed;
            }
            // Nothing deliverable is not a failure: a new report or a session change wakes us.
            let wait = if failed { backoff } else { RETRY_MAX };
            backoff = if failed { (backoff * 2).min(RETRY_MAX) } else { RETRY_INITIAL };
            let token = cloud.account_tasks_token();
            // While the service is refusing, a new report must not shorten the backoff: the live
            // attempt for it just failed too, and waking per completion turns the ladder into a
            // retry per track.
            let wake_on_report = !failed;
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                () = token.cancelled() => backoff = RETRY_INITIAL,
                changed = status.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    backoff = RETRY_INITIAL;
                },
                () = notified.notified(), if wake_on_report => backoff = RETRY_INITIAL,
                () = tokio::time::sleep(wait) => {},
            }
        }
    });
}

async fn drain(
    cloud: &CloudService,
    queue: &Arc<ListenQueue>,
    account_id: i64,
    shutdown: &CancellationToken,
) -> Drain {
    let entries = queue.deliverable(account_id);
    if entries.is_empty() {
        return Drain::Empty;
    }
    let mut outcome = Drain::Delivered;
    for entry in entries {
        let status = cloud.status();
        let token = cloud.account_tasks_token();
        if status.account.as_ref().map(|account| account.id) != Some(account_id)
            || token.is_cancelled()
        {
            outcome = Drain::Stopped;
            break;
        }
        // Cancelling an in-flight request leaves the entry on disk, so the cost is a possible
        // duplicate, never a lost listen; the alternative is making every exit wait for a socket.
        let result = tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                outcome = Drain::Stopped;
                break;
            }
            () = token.cancelled() => {
                outcome = Drain::Stopped;
                break;
            }
            result = cloud.report_listen(entry.song_id, entry.seconds, status.generation) => result,
        };
        match result {
            Ok(()) if cloud.status().generation == status.generation => queue.remove(entry.seq),
            Ok(()) => {
                // The account moved under the request; leave the entry to its owner.
                outcome = Drain::Stopped;
                break;
            }
            // The service has no idempotency key, so a transport failure may already have landed.
            // Retrying is still the side that keeps the listen, and the duplicate is the cost.
            Err(error) => {
                log::warn!("NetEase listening retry: {}", describe(&error));
                outcome = Drain::Failed;
                break;
            }
        }
    }
    flush(queue).await;
    outcome
}

fn sample(state: &AppState) -> ListenSample {
    let account = state.cloud.status();
    let player = lock_state(&state.player_state);
    let key = account.account.and_then(|_| {
        player
            .source
            .as_ref()
            .filter(|track| {
                track.id > 0
                    && track.file_path.is_empty()
                    && player.status != PlaybackStatus::Stopped
            })
            .map(|track| ListenKey {
                song_id: track.id,
                playback_generation: state.cloud_playback.listening_generation(),
                account_generation: account.generation,
            })
    });
    ListenSample {
        key,
        playing: player.status == PlaybackStatus::Playing,
        position_ms: player.position_ms,
        speed: player.playback_speed,
        decoder_generation: state.cloud_playback.decoder_generation(),
    }
}

fn enqueue(
    cloud: &CloudService,
    reports: &mpsc::Sender<ListenReport>,
    queue: &ListenQueue,
    notified: &Notify,
    report: ListenReport,
) {
    if reports.try_send(report).is_ok() {
        return;
    }
    // The worker is behind or gone. The tap already took this listen off the player, so dropping
    // it is a loss rather than a deferred sample.
    if let Some(account_id) = account_for(cloud, report.key.account_generation) {
        if queue.push(report.key.song_id, report.seconds, Some(account_id), listen_queue::now_ms())
        {
            notified.notify_one();
        }
    } else {
        log::warn!("NetEase listening queue is full or closed and its account is gone; dropping");
    }
}

async fn submit(cloud: &CloudService, queue: &ListenQueue, report: ListenReport) -> bool {
    let status = cloud.status();
    let Some(account) = status.account else {
        return false;
    };
    if status.generation != report.key.account_generation {
        return false;
    }
    match cloud
        .report_listen(report.key.song_id, report.seconds, report.key.account_generation)
        .await
    {
        Ok(()) if cloud.status().generation == status.generation => false,
        // The account moved while the write was in flight, and the service skips rather than
        // reports a stale generation. Keep the listen for the account that heard it.
        Ok(()) => push_report(queue, report, account.id),
        Err(error) => {
            log::warn!(
                "NetEase listening report queued after a failed attempt: {}",
                describe(&error)
            );
            push_report(queue, report, account.id)
        }
    }
}

fn persist(cloud: &CloudService, queue: &ListenQueue, report: ListenReport) -> bool {
    account_for(cloud, report.key.account_generation)
        .is_some_and(|account_id| push_report(queue, report, account_id))
}

fn push_report(queue: &ListenQueue, report: ListenReport, account_id: i64) -> bool {
    queue.push(report.key.song_id, report.seconds, Some(account_id), listen_queue::now_ms())
}

/// The signed-in account a report belongs to. A generation mismatch means the session that heard
/// it has ended, and reporting it under the replacement would put it in the wrong history.
fn account_for(cloud: &CloudService, generation: u64) -> Option<i64> {
    let status = cloud.status();
    if status.generation != generation {
        return None;
    }
    status.account.map(|account| account.id)
}

async fn flush(queue: &Arc<ListenQueue>) {
    let queue = Arc::clone(queue);
    match tokio::task::spawn_blocking(move || queue.flush()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("NetEase listening queue write: {}", describe(&error)),
        Err(error) => log::warn!("NetEase listening queue write task: {}", describe(&error)),
    }
}
