//! `sinks.view_model` subscriber + the (decode + metadata fetch + write)
//! apply step. Skipped while the view is closed; seeded on open.
//!
//! Source-keyed artwork and metadata updates for the currently playing track.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use async_compat::Compat;
use slint::{ComponentHandle, Image, TimerMode, Weak};

use super::metadata::{to_slint_stream_meta, to_slint_track_meta};
use super::write_crossfade_slot;
use super::{NowPlayingSource, NowPlayingState, SourceKey};
use crate::ui::appearance::theme_apply::color;
use crate::ui::aurora;
use crate::ui::backdrop::{self, BackdropSample};
use crate::ui::chips;
use crate::ui::now_playing_artwork::NowPlayingArtwork;
use crate::ui::remote_covers;
use nekonetease_app::library;
use nekonetease_app::state::AppState;
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, Player, TrackMetaRow};

thread_local! {
    /// The measurement behind whatever is in the `Player.np-*` tier now, so a palette change can
    /// re-solve it. `hero_backdrop::PUBLISHED_HERO`'s twin, `None` until the first track lands.
    static PUBLISHED_SAMPLE: Cell<Option<BackdropSample>> = const { Cell::new(None) };
}

/// What one artwork decode hands back to the UI thread: the sharp cover, the
/// blurred backdrop, and the hue + brightness measured off the sharp downscale.
type DecodedArtwork = (Option<Image>, Option<Image>, backdrop::BackdropSample);

/// Subscribe to `sinks.view_model` and react only to actual source changes. Always
/// stashes the current source; then, *only while the view is open*, decodes and blurs
/// the artwork into the inactive slot, flips, and fetches the chips. Closed, it stashes
/// and stops — `wire_now_playing_open` decodes on the next open.
pub(super) fn spawn_source_change_subscriber(
    ui: &AppWindow,
    state: &AppState,
    np_artwork: Arc<NowPlayingArtwork>,
    np_state: Rc<NowPlayingState>,
    initial_key: Option<SourceKey>,
) -> Result<(), slint::EventLoopError> {
    let weak = ui.as_weak();
    let state = state.clone();
    let mut rx = state.sinks.view_model.subscribe();
    // Seeded from the snapshot in `install` so the subscriber doesn't
    // re-fire for the already-seeded restored source.
    let mut last_key = initial_key;
    let mut last_artwork_path =
        np_state.current_source.borrow().as_ref().and_then(|source| source.artwork_path.clone());
    slint::spawn_local(Compat::new(async move {
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            // *Only* the source's identity: this fires on every `view_model` push — play,
            // pause, volume and speed — and acts
            // solely on a source change. Both the read and the compare happen under the
            // borrow, so a push that moved nothing allocates nothing; the guard is gone by
            // the closing brace, well before the `.await` below.
            let new_source = {
                let vm = rx.borrow_and_update();
                let Some(vm) = vm.as_ref() else { continue };
                let source = vm.source();
                let next = NowPlayingSource::from_vm(vm);
                let artwork_path = next.as_ref().and_then(|source| source.artwork_path.clone());
                if SourceKey::describes(last_key.as_ref(), source.as_ref().map(|s| s.id))
                    && artwork_path == last_artwork_path
                {
                    continue;
                }
                last_key = next.as_ref().map(|source| source.key.clone());
                last_artwork_path = artwork_path;
                next
            };
            // Regardless of visibility, so a later open can seed the artwork from it.
            np_state.current_source.borrow_mut().clone_from(&new_source);
            // The decode, blur and metadata read only produce something on screen while a surface
            // renders them. Otherwise `wire_now_playing_open` or `kick_artwork` seeds on the next
            // open.
            if np_state.renders_artwork() {
                apply_source_change(&weak, &state, &np_artwork, np_state.clone(), new_source, true)
                    .await;
            }
        }
        log::debug!("ui::now_playing source-change subscriber stopped");
    }))?;
    Ok(())
}

/// Fetch metadata and decode the cover for `track`, then write both into the `Player`
/// global. Runs inside a `Compat` future, so it stays on the UI thread across the
/// `.await`s.
///
/// `animate` picks the cross-fade behaviour — `true` for a live change while the view is
/// open, `false` for the seed-on-open path. Records the applied identity in
/// `applied_source`, so a redundant re-seed is skipped.
pub(super) async fn apply_source_change(
    weak: &Weak<AppWindow>,
    state: &AppState,
    np_artwork: &Arc<NowPlayingArtwork>,
    np_state: Rc<NowPlayingState>,
    source: Option<NowPlayingSource>,
    animate: bool,
) {
    let key = source.as_ref().map(|s| s.key.clone());
    let track = source.as_ref().and_then(|s| s.track.clone());
    let artwork_path = source.as_ref().and_then(|s| s.artwork_path.clone());

    // A source with no local artwork may spend a moment resolving its remote URL. Clear the old
    // song's large slots before that await so they are never shown as the new song's cover.
    if artwork_path.as_deref().is_none_or(str::is_empty)
        && let Some(ui) = weak.upgrade()
    {
        let player = ui.global::<Player>();
        player.set_np_cover_has_image(false);
        player.set_blur_has_image(false);
        write_backdrop_tiers(&ui, BackdropSample::default());
    }

    // **Kicked rather than awaited, and ahead of both reads rather than past the guard below.**
    // The lookup reaches a file and then a socket, so awaited here it put the cover, the title and
    // the backdrop behind a request that can time out, leaving this view painting the previous
    // track for as long as the directory takes to refuse. The hook reads the source cell every
    // caller here has already written, takes its own claim before its first `.await` and drops its
    // result if a newer song took that claim, which is the guard below one layer in.
    np_state.lyrics.kick();

    let meta = fetch_track_meta(state, track.as_ref()).await;
    let (cover, blurred, sample, resolved_artwork_path) =
        decode_artwork_for(state, np_artwork, artwork_path.clone(), track.as_deref()).await;
    let cover_missing = cover.is_none();

    // --- Write to Slint (UI thread) ---
    let Some(ui) = weak.upgrade() else { return };

    // A newer source change may have landed mid-decode: the open-seed task and the
    // subscriber can both have an `apply_source_change` in flight. If the decoded source
    // is no longer current, drop it — the newer call owns the slots and `applied_source`.
    let mut current = np_state.current_source.borrow_mut();
    if key != current.as_ref().map(|s| s.key.clone())
        || artwork_path != current.as_ref().and_then(|s| s.artwork_path.clone())
    {
        return;
    }
    if let Some(current) = current.as_mut() {
        current.artwork_path.clone_from(&resolved_artwork_path);
    }
    drop(current);

    if resolved_artwork_path != artwork_path
        && resolved_artwork_path.as_deref().is_some_and(remote_covers::is_remote_artwork_url)
    {
        let covers = ui.global::<nekonetease_ui::RowCovers>();
        covers.set_generation(covers.get_generation().wrapping_add(1));
    }

    let player = ui.global::<Player>();
    publish_chips(&player, &np_state, meta);
    write_backdrop_tiers(&ui, sample);

    // The blur is the backdrop, so it takes the same cross-fade as the cover.
    write_crossfade_slot(
        blurred,
        animate,
        player.get_blur_use_a(),
        |img| player.set_blur_img_a(img),
        |img| player.set_blur_img_b(img),
        |v| player.set_blur_use_a(v),
        |v| player.set_blur_has_image(v),
    );

    write_crossfade_slot(
        cover,
        animate,
        player.get_np_cover_use_a(),
        |img| player.set_np_cover_a(img),
        |img| player.set_np_cover_b(img),
        |v| player.set_np_cover_use_a(v),
        |v| player.set_np_cover_has_image(v),
    );

    *np_state.applied_source.borrow_mut() = key;
    *np_state.applied_artwork_path.borrow_mut() = resolved_artwork_path;

    if cover_missing
        && let Some(track) = track.as_deref()
        && let (Some(artist), Some(album)) = (track.artist.as_deref(), track.album.as_deref())
        && !artist.trim().is_empty()
        && !album.trim().is_empty()
    {
        schedule_artwork_retry(weak, state, np_artwork, &np_state, artist, album);
    } else {
        np_state.artwork_retry.stop();
    }
}

/// Retry a transient remote lookup/download only while the same source still has a chance to
/// produce a cover. Definitive no-matches stay quiet; failed lookups expire after a short delay.
fn schedule_artwork_retry(
    weak_ui: &Weak<AppWindow>,
    state: &AppState,
    np_artwork: &Arc<NowPlayingArtwork>,
    np_state: &Rc<NowPlayingState>,
    artist: &str,
    album: &str,
) {
    let weak_ui = weak_ui.clone();
    let weak_np = Rc::downgrade(np_state);
    let state = state.clone();
    let np_artwork = Arc::clone(np_artwork);
    let artist = artist.to_owned();
    let album = album.to_owned();
    let source_key = np_state.current_source.borrow().as_ref().map(|source| source.key.clone());
    np_state.artwork_retry.start(
        TimerMode::SingleShot,
        nekonetease_app::services::track_artwork::TRANSIENT_RETRY_DELAY,
        move || {
            let (Some(_ui), Some(np_state)) = (weak_ui.upgrade(), weak_np.upgrade()) else {
                return;
            };
            if !np_state.renders_artwork() {
                return;
            }
            let Some(source) = np_state.current_source.borrow().clone() else { return };
            if Some(source.key.clone()) != source_key {
                return;
            }
            let is_remote_source =
                source.artwork_path.as_deref().is_some_and(remote_covers::is_remote_artwork_url);
            let lookup_may_change = matches!(
                state.track_artwork.cached_url(&artist, &album),
                nekonetease_app::services::track_artwork::CachedCoverUrl::NotCached
                    | nekonetease_app::services::track_artwork::CachedCoverUrl::Pending
                    | nekonetease_app::services::track_artwork::CachedCoverUrl::Found(_)
            );
            if !is_remote_source && !lookup_may_change {
                return;
            }

            let weak_ui = weak_ui.clone();
            let state = state.clone();
            let np_artwork = Arc::clone(&np_artwork);
            let result = slint::spawn_local(Compat::new(async move {
                apply_source_change(&weak_ui, &state, &np_artwork, np_state, Some(source), false)
                    .await;
            }));
            if let Err(error) = result {
                log::warn!("ui::now_playing artwork retry task spawn: {}", describe(&error));
            }
        },
    );
}

/// The eight chip columns for `track`, awaited inline — sqlx has a reactor here. Every failure arm
/// is the same empty row, which is what clears the chips: a missing row, a failed read and no track
/// at all are three spellings of "nothing to state about this track".
async fn fetch_track_meta(state: &AppState, track: Option<&Arc<TrackSummary>>) -> TrackMetaRow {
    let Some(track) = track else {
        return TrackMetaRow::default();
    };

    // A cloud song carries its `NetEase` id, which indexes another table entirely — a local
    // row holding the same integer would answer for a file this track is not.
    if track.file_path.is_empty() {
        return to_slint_stream_meta(track);
    }

    match library::tracks::get_track_meta(&state.db, track.id).await {
        Ok(Some(meta)) => to_slint_track_meta(&meta),
        Ok(None) => TrackMetaRow::default(),
        Err(e) => {
            log::warn!("ui::now_playing get_track_meta({}): {}", track.id, describe(&e));
            TrackMetaRow::default()
        }
    }
}

/// Decode the source's artwork on the blocking pool. A *single* decode derives both the sharp tile
/// and the blurred backdrop — the cover is the largest image on the app's hot path, so decoding it
/// once rather than twice halves the per-skip cost.
///
/// Local paths use the path-keyed artwork cache. HTTP/S sources are fetched under a byte cap and
/// share the same cache keyed by URL, so no image file is needed for either path.
///
/// Every arm that isn't a decoded pair answers with an empty sample rather than a previous one:
/// `BackdropSample::solve` reads that as "no artwork" and falls back to `Theme.accent`, the honest
/// answer for a source whose artwork is missing or unreadable.
async fn decode_artwork_for(
    state: &AppState,
    np_artwork: &Arc<NowPlayingArtwork>,
    artwork_path: Option<String>,
    track: Option<&TrackSummary>,
) -> (Option<Image>, Option<Image>, backdrop::BackdropSample, Option<String>) {
    let empty = (None, None, BackdropSample::default());
    let mut resolved_path = artwork_path.filter(|path| !path.is_empty());
    let mut decoded = match resolved_path.as_deref() {
        Some(source) => decode_artwork_source(state, np_artwork, source).await,
        None => empty,
    };

    // Some local files have no embedded cover (or reference a damaged/missing stored one). Resolve
    // the tagged album online only after the local path failed, so intact local artwork remains
    // immediate and offline playback keeps working.
    if decoded.0.is_none()
        && let Some(track) = track
        && let (Some(artist), Some(album)) = (track.artist.as_deref(), track.album.as_deref())
        && let Some(url) = state.track_artwork.resolve(state.http_client(), artist, album).await
    {
        decoded = decode_artwork_source(state, np_artwork, &url).await;
        resolved_path = Some(url);
    }

    (decoded.0, decoded.1, decoded.2, resolved_path)
}

/// Decode one local path or bounded HTTP/S response on the blocking pool. Remote bytes use the
/// same capped decoder and artwork-pair cache as local files, keyed by their URL in memory.
async fn decode_artwork_source(
    state: &AppState,
    np_artwork: &Arc<NowPlayingArtwork>,
    source: &str,
) -> DecodedArtwork {
    let np = np_artwork.clone();
    if remote_covers::is_remote_artwork_url(source) {
        let Some(bytes) = remote_covers::fetch_image_bytes(state.http_client(), source).await
        else {
            return (None, None, BackdropSample::default());
        };
        let source = source.to_owned();
        match state.runtime.spawn_blocking(move || np.get_or_decode_memory(&source, &bytes)).await {
            Ok(Some(pair)) => {
                (Some(Image::from_rgb8(pair.cover)), pair.blur.map(Image::from_rgb8), pair.sample)
            }
            Ok(None) => (None, None, BackdropSample::default()),
            Err(e) => {
                log::warn!("ui::now_playing remote artwork task join: {}", describe(&e));
                (None, None, BackdropSample::default())
            }
        }
    } else {
        let source = source.to_owned();
        match state.runtime.spawn_blocking(move || np.get_or_decode(Path::new(&source))).await {
            Ok(Some(pair)) => {
                (Some(Image::from_rgb8(pair.cover)), pair.blur.map(Image::from_rgb8), pair.sample)
            }
            Ok(None) => (None, None, BackdropSample::default()),
            Err(e) => {
                log::warn!("ui::now_playing artwork task join: {}", describe(&e));
                (None, None, BackdropSample::default())
            }
        }
    }
}

/// Chunk the new track's chips and push them with the meta row they came from.
///
/// Refreshes the shadow from the just-fetched `meta` and pushes a freshly chunked model, so the
/// view reflects the new track without waiting on a width-change fire. With no layout pass yet the
/// chunk collapses to one row and the strip's mount `Timer` fires a real width immediately. `None`
/// because this column can grow downward — the hero band can't.
fn publish_chips(player: &Player<'_>, np_state: &NowPlayingState, meta: TrackMetaRow) {
    let chip_texts = super::metadata::visible_chip_texts(&meta);
    let chip_rows = chips::chunk_chips_to_rows(&chip_texts, np_state.chip_last_width.get(), None);
    // Unconditional: a new track's chips are new text, which a row-length comparison
    // can't see. Recording the shape is what lets the width channel skip its repaints.
    *np_state.chip_last_shape.borrow_mut() = chips::split_shape(&chip_rows);
    player.set_chip_rows(chips::rows_to_model(chip_rows));
    *np_state.chip_texts.borrow_mut() = chip_texts;
    player.set_track_meta(meta);
}

/// Every colour this view paints on the backdrop, answered together. Which arm runs and both of
/// its fallbacks live on [`BackdropSample::solve`] and [`aurora::tints`], so this tier and the
/// hero's resolve them identically — including for a track with no artwork, which washes the
/// accent rather than dropping to the other backdrop.
fn write_backdrop_tiers(ui: &AppWindow, sample: BackdropSample) {
    let theme = backdrop::theme_tokens(ui);
    let colors = sample.solve(&theme, backdrop::kind(ui));
    let player = ui.global::<Player>();

    PUBLISHED_SAMPLE.set(Some(sample));

    player.set_np_accent_bright(backdrop::chrome_brush(&colors));
    player.set_np_chrome_text(backdrop::chrome_text_brush(&colors));
    player.set_np_chip_fill(backdrop::chip_fill_brush(&colors));
    player.set_np_viz(backdrop::viz_brush(&colors));
    player.set_np_on_backdrop(backdrop::text_brush(&colors));
    player.set_np_on_backdrop_muted(backdrop::muted_brush(&colors));
    player.set_np_floor_start(color(colors.floor_start));
    player.set_np_floor_end(color(colors.floor_end));
    player.set_np_scrim(backdrop::scrim_brush(&colors));

    let [tint_1, tint_2, tint_3] = aurora::tints(sample.seeds, &theme);
    player.set_np_tint_1(tint_1.to_color());
    player.set_np_tint_2(tint_2.to_color());
    player.set_np_tint_3(tint_3.to_color());
}

/// Re-solve the view's tiers against a palette that has just changed —
/// `hero_backdrop::republish_for_palette`'s twin, and the one that matters more. A band republishes
/// on every detail open, where all three callers of [`apply_source_change`] dedup on
/// `applied_source`, so without this the tiers hold until the next *source*.
pub(crate) fn republish_for_palette(ui: &AppWindow) {
    if let Some(sample) = PUBLISHED_SAMPLE.get() {
        write_backdrop_tiers(ui, sample);
    }
}
