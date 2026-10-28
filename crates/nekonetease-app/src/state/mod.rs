use std::sync::{Arc, OnceLock};

use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

pub mod contexts;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod scan;
pub mod signal;
pub use contexts::PlaybackContext;
pub use scan::{ScanControl, ScanPhase, ScanProgressTick};
pub use signal::{SharedFlag, Signal};

use crate::cloud::CloudService;
use crate::cloud_playback::CloudPlayback;
use crate::services::search_history::SearchHistoryState;
use crate::services::settings;
use nekonetease_artwork::media::image::artwork::CoverCache;
use nekonetease_core::config::Paths;
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::self_writes::SelfWrites;
use nekonetease_engine::player::engine::backend::PlaybackEngine;
use nekonetease_engine::player::engine::event_sink::{MediaControlsSync, PlayerEvent, PlayerSinks};
use nekonetease_engine::player::engine::signal_path::SignalPath;
use nekonetease_engine::player::engine::state::{
    PlayerStateHandle, PlayerViewModelLight, PositionTick, QueueViewModel, lock_state,
};
use nekonetease_integrations::services::integrations::media_controls::{self, MediaControlsHandle};
use nekonetease_net::services::net::pacer::RequestPacer;
use nekonetease_platform::services::platform::always_on_top::{self, AlwaysOnTopCapability};
use nekonetease_playback::player::playback::decks::DECK_COUNT;
use nekonetease_playback::player::playback::output::{AudioOutput, OutputMode};
use nekonetease_playback::player::playback::stream_health::AudioStreamHealth;
use nekonetease_store::database::{self, DbPool};
use nekonetease_store::media::ingest::watcher::{FileEvent, FolderWatcher};

#[derive(Clone)]
pub struct AppState {
    pub paths: Arc<Paths>,
    pub runtime: Handle,
    /// Direct `NetEase` API and account session boundary shared by cloud views and playback.
    pub cloud: CloudService,
    pub cloud_playback: CloudPlayback,
    /// Session-scoped URL lookups for tracks without readable embedded artwork.
    pub track_artwork: Arc<crate::services::track_artwork::TrackArtworkResolver>,
    pub db: DbPool,
    pub cover_cache: CoverCache,
    pub player_state: Arc<PlayerStateHandle>,
    pub engine: Arc<PlaybackEngine>,
    /// In-app MV playback. Inert on a machine where libmpv would not initialise, so every caller
    /// may drive it without asking first; [`available`](nekonetease_video::VideoPlayer::available)
    /// is for the UI that has something to say about it.
    pub video: Arc<nekonetease_video::VideoPlayer>,
    /// Fault counters the output device's error callback writes into, on the
    /// audio thread. Drained by `tasks::audio_health`, the output's reopen being the
    /// one other reader.
    pub audio_health: Arc<AudioStreamHealth>,
    pub sinks: Arc<PlayerSinks>,
    pub position_tx: watch::Sender<Option<PositionTick>>,
    /// What the playing source goes through on its way to the device, `None` while stopped.
    pub signal_path_tx: watch::Sender<Option<SignalPath>>,
    /// Bumped whenever the track library is mutated by a scan or watcher
    /// event. UI subscribers re-fetch the Tracks model on each tick.
    pub library_changed: Signal,
    /// Bumped after every play-count flush. Split from `library_changed`
    /// so the per-song flush (every track completion) doesn't imply
    /// "library structure changed" — the only views that depend on
    /// play-driven data subscribe here: Favorites (hero mosaic + Most Played
    /// strip, ranked by `play_count`) and Recently-Played (ordered by
    /// `last_played`, written on the same flush). Everything else
    /// (Tracks/Browse refreshers, `queue_prune`, the folder list) stays on
    /// `library_changed` and no longer fires per played song.
    pub stats_changed: Signal,
    /// Bumped after the cloud liked-song membership changes from a player control, so the
    /// Favorites view can refresh its cached list without coupling `AppState` to Slint handles.
    pub cloud_liked_changed: Signal,
    /// Bumped after the UI language changes. A locale switch re-resolves every live `@tr`
    /// binding on its own, so this exists only for the strings Rust renders through a
    /// trampoline and stores in a model, which nothing re-reads. One subscriber: the
    /// notification stack, whose rows outlive every navigation. Everything else so stored
    /// sits behind a section that hands its models back on the way to Settings.
    pub locale_changed: Signal,
    /// Bumped whenever the watcher reports a kernel-queue overflow (notify
    /// `Flag::Rescan`). A UI-thread subscriber pushes a transient
    /// "library re-syncing" toast through the notifications stack so the
    /// user knows why Tracks/Browse paused refreshing while the
    /// reconcile runs. Coalescing semantics: a burst of overflows that
    /// land in the same `watch` slot only paints one toast — which is
    /// also what the reconcile's own coalescing does to its spawn.
    pub rescan_notice: Signal,
    /// Bumped after the auto-update switch reaches disk. `tasks::updater_daily` sleeps for hours
    /// at a stretch and up to a week under the failure backoff, so without this the switch is a
    /// control whose effect lands whenever the loop happens to wake. The tick only wakes it early;
    /// what it does on waking is still decided by the file, which is what keeps a hand-edited
    /// `settings.json` working.
    pub auto_check_changed: Signal,
    /// Bumped by `tasks::audio_health` when the output device goes away. A
    /// UI-thread subscriber pushes a sticky warning toast — nothing else
    /// notices, so playback runs on with the position ticking and no sound.
    /// Coalesces like `rescan_notice`: a burst paints one toast.
    pub audio_device_lost: Signal,
    /// How many scans are restoring cover art whose stored file went missing, for the notice
    /// that says so. A count rather than a [`Signal`] because the boot scan can start restoring
    /// before the notice's subscriber exists, and a payload is still there when it attaches.
    pub artwork_restoring: watch::Sender<u32>,
    /// The folder scan in flight: its progress for the Library settings bar, and its cancel.
    pub scan: Arc<ScanControl>,
    pub watcher: Arc<parking_lot::Mutex<FolderWatcher>>,
    /// Files this process wrote itself (tag edits), so the watcher can drop the
    /// `Modified` events its own writes generate instead of paying for a full
    /// re-ingest of a file it just retagged. See [`SelfWrites`].
    pub self_writes: Arc<SelfWrites>,
    pub always_on_top: AlwaysOnTopCapability,
    pub search_history: Arc<SearchHistoryState>,
    /// Whether lyrics run at all: `library::lyrics`
    /// refuses on it from a worker, and the Now Playing switch and the Settings card
    /// both write it.
    pub lyrics_enabled: SharedFlag,
    /// Whether the lyrics panel may look a sheet up online, on the same terms as
    /// `library::lyrics`'s guard runs on a worker once per
    /// track change, where a `settings.json` read would be a file read on the path a
    /// track change already pays for.
    pub lyrics_online_enabled: SharedFlag,
    /// Whether the panel draws the romanization it already holds. Here for a different
    /// reason from its neighbours: no worker reads it, but the Settings card and the
    /// Now Playing menu both write it, and two callbacks reaching one field through
    /// disk is the race the mirror exists to close.
    pub lyrics_romanization_shown: SharedFlag,
    /// How often the lyrics directory may be asked, and the stop it can impose.
    ///
    /// Owned here rather than kept as a `static` beside the client, so it is one instance a test
    /// can construct fresh and the pacing is state rather than process-global. Shared by every
    /// clone of `AppState`, which is the point: the floor is per host, not per caller.
    pub lyrics_pacer: Arc<RequestPacer>,
    /// Whether a star rating is also written into the file's own tag, on the same
    /// `tasks::rating_writeback` asks once per
    /// coalesced burst, and a `settings.json` read there would be a file read on
    /// the path a rating click already paid for.
    pub write_ratings_to_tags: SharedFlag,
    pub media_controls: Option<Arc<MediaControlsHandle>>,
    /// Shared `reqwest::Client`, initialized for the `NetEase` service and reused
    /// by other remote calls. `Arc<OnceLock<…>>` keeps clones of `AppState` on
    /// one connection pool (`reqwest::Client` itself wraps an internal pool in
    /// `Arc`).
    http_client: Arc<OnceLock<reqwest::Client>>,
    /// Tracks every spawned background task so shutdown can wait for them
    /// to finish their current write before the runtime is dropped.
    pub task_tracker: TaskTracker,
    /// Broadcast cancel signal — long-running loops listen via `cancelled().await`.
    pub shutdown_token: CancellationToken,
}

/// Receivers handed back from `AppState::init` for `boot::tasks` to consume.
/// Holding them on `AppState` would force a `Mutex<Option<…>>` shape nothing
/// needs; returning them keeps the struct stable.
pub struct StartupChannels {
    pub media_control_rx: Option<mpsc::Receiver<PlayerEvent>>,
    pub file_event_rx: mpsc::Receiver<FileEvent>,
}

impl AppState {
    #[allow(
        clippy::too_many_lines,
        reason = "initializes the shared runtime services and engine state as one startup transaction"
    )]
    pub async fn init(paths: Paths, runtime: Handle) -> AppResult<(Self, StartupChannels)> {
        // Ahead of the output, which opens differently under exclusive output.
        let settings = settings::read_settings(&paths).unwrap_or_else(|e| {
            log::warn!("Failed to read settings on startup: {}; using defaults", describe(&e));
            settings::SettingsData::default()
        });

        // The error callback records into counters rather than logging, because cpal calls it on
        // the output worker thread; `player::playback::stream_health` argues that.
        let audio_health = Arc::new(AudioStreamHealth::default());
        let audio_output = open_output(&settings, audio_health.clone())?;
        // The runtime handle is only used to schedule the deferred half of a
        // faded pause / stop (arm the ramp now, pause the decks once it lands).
        let engine = Arc::new(PlaybackEngine::with_output(audio_output, runtime.clone())?);
        // Ahead of the first `open`: the worker applies the setting to a player with no file
        // loaded, where the alternative is to start decoding with a choice the user turned off.
        let video = nekonetease_video::VideoPlayer::spawn();
        video.set_hwdec(settings.playback.hwdec_enabled);

        let db = database::init_database(&paths).await?;

        let player_state = Arc::new(PlayerStateHandle::default());
        {
            let mut s = lock_state(&player_state);
            s.volume = settings.volume.min(nekonetease_engine::player::engine::state::MAX_VOLUME);
            s.is_muted = settings.playback.is_muted;
            s.pre_mute_volume = s.volume;
            s.gapless_enabled = settings.playback.gapless_playback;
            s.playback_speed = settings.playback.playback_speed.clamp(
                nekonetease_engine::player::engine::state::MIN_SPEED,
                nekonetease_engine::player::engine::state::MAX_SPEED,
            );
            let vol = s.effective_volume();
            let speed = s.playback_speed;
            drop(s);
            engine.set_volume(vol);
            engine.set_speed(speed);
        }

        hydrate_audio_dsp(&engine, &settings);

        let cover_cache: CoverCache = nekonetease_artwork::media::image::artwork::new_cover_cache();

        let (vm_tx, _) = watch::channel::<Option<PlayerViewModelLight>>(None);
        let (q_tx, _) = watch::channel::<Option<QueueViewModel>>(None);
        let (position_tx, _) = watch::channel::<Option<PositionTick>>(None);
        let (signal_path_tx, _) = watch::channel::<Option<SignalPath>>(None);
        let shutdown_token = CancellationToken::new();
        let scan = Arc::new(ScanControl::new(&shutdown_token));

        let (mc_handle, mc_rx) = media_controls::init_media_controls();
        let mc_handle = Arc::new(mc_handle);
        let media_controls_sync: Arc<dyn MediaControlsSync> = mc_handle.clone();

        let sinks = Arc::new(PlayerSinks {
            view_model: vm_tx,
            queue: q_tx,
            media_controls: Some(media_controls_sync),
            external_track_playback: std::sync::RwLock::new(None),
        });

        let (file_tx, file_rx) = mpsc::channel::<FileEvent>(256);
        let watcher = Arc::new(parking_lot::Mutex::new(FolderWatcher::new(file_tx)));

        let always_on_top_capability = always_on_top::detect_capability();
        log::info!(
            "Always-on-top: method={:?}, supported={}, native_decorations={}",
            always_on_top_capability.method,
            always_on_top_capability.supported,
            always_on_top_capability.use_native_decorations
        );

        let search_history = Arc::new(SearchHistoryState::init(&paths).await);
        // Shared lazy client: the TLS stack and connection pool are constructed on first use.
        let http_client = Arc::new(OnceLock::new());
        let http =
            http_client.get_or_init(nekonetease_net::services::net::build_http_client).clone();
        let (cloud, cloud_playback) = initialize_cloud_playback(
            http,
            paths.data_dir.join("netease-artwork"),
            engine.clone(),
            player_state.clone(),
            sinks.clone(),
            runtime.clone(),
            settings.playback.cloud_quality,
        )
        .await;

        let state = Self {
            paths: Arc::new(paths),
            runtime,
            cloud,
            cloud_playback,
            track_artwork: Arc::new(crate::services::track_artwork::TrackArtworkResolver::new()),
            db,
            cover_cache,
            player_state,
            engine,
            video,
            audio_health,
            sinks,
            position_tx,
            signal_path_tx,
            library_changed: Signal::new(),
            stats_changed: Signal::new(),
            cloud_liked_changed: Signal::new(),
            locale_changed: Signal::new(),
            rescan_notice: Signal::new(),
            auto_check_changed: Signal::new(),
            audio_device_lost: Signal::new(),
            artwork_restoring: watch::Sender::new(0),
            scan,
            watcher,
            self_writes: Arc::new(SelfWrites::default()),
            always_on_top: always_on_top_capability,
            search_history,
            lyrics_enabled: SharedFlag::new(settings.lyrics.lyrics_enabled),
            lyrics_online_enabled: SharedFlag::new(settings.lyrics.lyrics_online_enabled),
            lyrics_romanization_shown: SharedFlag::new(settings.lyrics.lyrics_romanization_shown),
            lyrics_pacer: Arc::new(crate::library::lyrics::pacer()),
            write_ratings_to_tags: SharedFlag::new(settings.library.write_ratings_to_tags),
            media_controls: Some(mc_handle),
            http_client,
            task_tracker: TaskTracker::new(),
            shutdown_token,
        };

        let channels = StartupChannels { media_control_rx: Some(mc_rx), file_event_rx: file_rx };

        Ok((state, channels))
    }

    /// Persist a settings mutation on the blocking pool, logging (never
    /// surfacing) a failure under `label`. This is the write half of the
    /// two-phase "apply live, then persist" shape used by the EQ / `ReplayGain`
    /// / playback-settings installers: the caller has already applied the value
    /// to the running player, so a failed disk write must not undo it — the
    /// warn is the only report.
    pub fn persist_blocking(
        &self,
        label: &'static str,
        f: impl FnOnce(&Paths) -> Result<(), AppError> + Send + 'static,
    ) {
        // `label` already names the setting, so one line here covers every
        // caller — including the three settings-row helpers that pass one
        // through. On the way in, so a write that hangs still says what it was.
        log::debug!("settings: {label}");
        let paths = Arc::clone(&self.paths);
        self.runtime.spawn_blocking(move || {
            if let Err(e) = f(&paths) {
                log::warn!("{label}: {}", describe(&e));
            }
        });
    }

    /// The shared `reqwest::Client`, reused for the process lifetime. Its pool is
    /// also injected into [`CloudService`](crate::cloud::CloudService), so API
    /// calls and other remote services share connections.
    pub fn http_client(&self) -> &reqwest::Client {
        self.http_client.get_or_init(nekonetease_net::services::net::build_http_client)
    }
}

async fn initialize_cloud_playback(
    http: reqwest::Client,
    artwork_dir: std::path::PathBuf,
    engine: Arc<PlaybackEngine>,
    player_state: Arc<PlayerStateHandle>,
    sinks: Arc<PlayerSinks>,
    runtime: Handle,
    quality: nekonetease_core::entities::cloud::Quality,
) -> (CloudService, CloudPlayback) {
    let (cloud, cloud_playback) =
        CloudPlayback::initialize(http, artwork_dir, engine, player_state, sinks.clone()).await;
    cloud_playback.set_quality(quality);
    sinks.set_external_track_playback(cloud_playback.external_track_handler(runtime));
    (cloud, cloud_playback)
}

/// Open the output the saved choice will play through. Parked under exclusive output, which claims
/// the card at the first play, and for a chosen shared device, which [`hydrate_audio_dsp`] opens:
/// opening the default device shared would only close it again.
fn open_output(
    settings: &settings::SettingsData,
    health: Arc<AudioStreamHealth>,
) -> AppResult<AudioOutput> {
    let choice = settings.output.output_choice();
    if choice.mode == OutputMode::Exclusive {
        log::info!("Audio output: parked until the first play claims the device");
        return AudioOutput::open_parked(DECK_COUNT, health);
    }
    if choice.shared_device().is_some() {
        log::info!("Audio output: parked until the chosen device opens");
        return AudioOutput::open_parked(DECK_COUNT, health);
    }
    let output = AudioOutput::open(DECK_COUNT, health)?;
    log::info!("Audio output: {:?}", output.negotiated());
    Ok(output)
}

/// Seed the playback engine's lock-free cells (graphic EQ, `ReplayGain`,
/// crossfade, following the file's rate) from persisted settings before
/// playback starts, so the first track is already processed when any of them
/// is enabled. All of them live on the engine (not `PlayerState`). Ordering is deliberate: values first,
/// `enabled` last, so the enable's generation bump publishes a fully-seeded
/// state to the audio thread.
fn hydrate_audio_dsp(engine: &PlaybackEngine, settings: &settings::SettingsData) {
    // `set_eq_gains` clamps and length-normalises the (possibly hand-edited)
    // gain list; the EQ ships off by default.
    engine.set_eq_gains(&settings.equalizer.eq_band_gains);
    engine.set_eq_preamp(settings.equalizer.eq_preamp);
    engine.set_eq_enabled(settings.equalizer.eq_enabled);

    // ReplayGain master state seeds the same way — per-track gain is baked per
    // source at play time. Ships off by default; the mode string falls back to
    // Album on an unknown value.
    engine.set_replaygain_preamp(settings.replaygain.rg_preamp);
    engine.set_replaygain_mode(
        nekonetease_playback::player::playback::replaygain::RgMode::from_settings_str(
            &settings.replaygain.rg_mode,
        ),
    );
    engine.set_replaygain_prevent_clipping(settings.replaygain.rg_prevent_clipping);
    engine.set_replaygain_enabled(settings.replaygain.rg_enabled);

    // Crossfade ships off; `set_crossfade_duration_ms` clamps a hand-edited value.
    engine.set_crossfade_duration_ms(settings.crossfade.crossfade_duration_ms);
    engine.set_crossfade_manual(settings.crossfade.crossfade_manual);
    engine.set_crossfade_skip_same_album(settings.crossfade.crossfade_skip_same_album);
    engine.set_crossfade_fade_on_pause(settings.crossfade.crossfade_fade_on_pause);
    engine.set_crossfade_enabled(settings.crossfade.crossfade_enabled);
    engine.set_follow_rate(settings.output.output_follow_rate);
    engine.set_release_when_paused(settings.output.output_paused_device.releases());
    engine.set_resync_hold(settings.output.resync_hold());
    // Nothing is loaded yet, so an exclusive choice leaves the output parked as `open_output` left
    // it, and claims the card at the first play. A shared one opens its chosen device here.
    engine.set_output_choice(settings.output.output_choice());

    // The visualizer is deliberately absent: its tap is armed by the
    // Now-Playing view being on screen, not by a persisted flag, so it must
    // stay disarmed until `Visualizer.set-active` fires. See
    // `nekonetease-views`' `ui/visualizer/`.
}
