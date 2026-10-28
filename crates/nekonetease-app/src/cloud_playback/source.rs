use std::path::Path;
use std::time::{Duration, Instant};

use nekonetease_audio::player::source::audio::AudioSource;
use nekonetease_audio::player::source::buffered::BufferedAudioSource;
use nekonetease_audio::player::source::file_decode::FileDecoder;
use nekonetease_core::entities::cloud::{Quality, Song, StreamTicket};
use nekonetease_core::entities::track::TrackSummary;
use nekonetease_core::error::{AppError, AppResult, describe};
use nekonetease_core::utils::toast;
use nekonetease_net::services::net::range_reader::RangeReader;

use super::CloudPlayback;
use crate::library::cloud_cache;

#[derive(Clone)]
pub(super) struct ActiveStreamTicket {
    pub(super) ticket: StreamTicket,
    pub(super) quality: Quality,
    pub(super) expires_at: Instant,
}

impl ActiveStreamTicket {
    pub(super) fn is_valid_for(&self, song_id: i64, quality: Quality) -> bool {
        self.ticket.song_id == song_id
            && self.quality == quality
            && self.expires_at > Instant::now()
    }
}

impl CloudPlayback {
    pub(super) fn valid_stream_ticket(
        &self,
        song_id: i64,
        quality: Quality,
    ) -> Option<ActiveStreamTicket> {
        self.active_stream_ticket
            .read()
            .as_ref()
            .filter(|active| active.is_valid_for(song_id, quality))
            .cloned()
    }

    pub(super) fn clear_stream_ticket_if_matches(&self, failed: &ActiveStreamTicket) {
        let mut active = self.active_stream_ticket.write();
        if active.as_ref().is_some_and(|current| current.ticket.url == failed.ticket.url) {
            *active = None;
        }
    }

    pub(super) async fn open_source_with_expired_ticket_retry(
        &self,
        song_id: i64,
        quality: Quality,
        start: Duration,
        cancellation: &tokio_util::sync::CancellationToken,
        mut active_ticket: ActiveStreamTicket,
        mut used_cached_ticket: bool,
    ) -> AppResult<(ActiveStreamTicket, BufferedAudioSource, bool)> {
        if let Some(source) =
            self.open_cached_source(song_id, quality, &active_ticket, start, cancellation).await?
        {
            return Ok((active_ticket, source, true));
        }
        let source = loop {
            let url = active_ticket.ticket.url.clone();
            let extension = decoder_extension(&active_ticket.ticket.format).to_owned();
            let reported_size = active_ticket.ticket.size;
            let source_task = tokio::task::spawn_blocking(move || {
                open_buffered_source(&url, &extension, reported_size, start)
            });
            let source_result = tokio::select! {
                () = cancellation.cancelled() => {
                    return Err(AppError::network_msg("网易云播放请求已取消"));
                }
                result = source_task => {
                    result.map_err(|error| AppError::io("在线播放解码任务失败", error))?
                }
            };
            match source_result {
                Ok(source) => break source,
                Err(error) if used_cached_ticket && is_expired_stream_url_error(&error) => {
                    self.clear_stream_ticket_if_matches(&active_ticket);
                    let ticket = tokio::select! {
                        () = cancellation.cancelled() => {
                            return Err(AppError::network_msg("网易云播放请求已取消"));
                        }
                        result = self.cloud.stream_ticket(song_id, quality) => result?,
                    };
                    if let Some(restriction) = ticket.denial() {
                        let ids = [song_id];
                        let songs = tokio::select! {
                            () = cancellation.cancelled() => {
                                return Err(AppError::network_msg("网易云播放请求已取消"));
                            }
                            result = self.cloud.songs(&ids) => result.ok(),
                        };
                        let song = songs
                            .as_ref()
                            .and_then(|songs| songs.iter().find(|song| song.id == song_id));
                        let restriction = super::refusal::refine_refusal(restriction, song);
                        toast::notify(super::refusal::kind_for(restriction), String::new());
                        return Err(super::refusal::refusal_error(restriction));
                    }
                    let issued_at = Instant::now();
                    let expires_at = issued_at
                        .checked_add(Duration::from_secs(ticket.expires_after_seconds))
                        .unwrap_or(issued_at);
                    active_ticket = ActiveStreamTicket { ticket, quality, expires_at };
                    used_cached_ticket = false;
                }
                Err(error) => return Err(error),
            }
        };
        Ok((active_ticket, source, false))
    }

    /// A fresh entry plays through the decoder a local file does, with no range reader behind it.
    ///
    /// `Ok(None)` is both a miss and an entry that will not decode: the network path is still
    /// there, so a damaged cache file costs a second read rather than the track.
    async fn open_cached_source(
        &self,
        song_id: i64,
        quality: Quality,
        ticket: &ActiveStreamTicket,
        start: Duration,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> AppResult<Option<BufferedAudioSource>> {
        if ticket.ticket.trial {
            return Ok(None);
        }
        // `CloudPlayback` is handed the artwork directory rather than `Paths`; the data root is
        // the directory above it.
        let Some(data_dir) = self.artwork_dir.parent() else {
            return Ok(None);
        };
        let tier = super::playback::served_quality(&ticket.ticket.level).unwrap_or(quality);
        let Some(entry) = cloud_cache::cached_stream_at(data_dir, song_id, tier) else {
            return Ok(None);
        };
        let task = tokio::task::spawn_blocking(move || open_cached_file(&entry, start));
        let result = tokio::select! {
            () = cancellation.cancelled() => {
                return Err(AppError::network_msg("网易云播放请求已取消"));
            }
            result = task => result.map_err(|error| AppError::io("缓存解码任务失败", error))?,
        };
        match result {
            Ok(source) => Ok(Some(source)),
            Err(error) => {
                log::warn!(
                    "Cloud cache entry unusable, playing from the network: {}",
                    describe(&error)
                );
                Ok(None)
            }
        }
    }
}

pub(super) async fn download_cover(
    http: &reqwest::Client,
    url: &str,
    dir: &std::path::Path,
) -> Option<String> {
    const MAX_BYTES: u64 = 1_048_576;
    let url = crate::cloud::cover_request_url(url)?;
    let bytes = nekonetease_net::services::net::get_capped(
        http,
        &url,
        "NetEase cover",
        Duration::from_secs(30),
        MAX_BYTES,
    )
    .await
    .ok()?;
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        nekonetease_artwork::media::image::artwork::store_image(&bytes, "jpg", &dir)
    })
    .await
    .ok()
    .flatten()
}

pub(super) fn is_expired_stream_url_error(error: &AppError) -> bool {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = cause {
        if current
            .downcast_ref::<nekonetease_net::services::net::range_reader::MediaError>()
            .is_some_and(|error| {
                matches!(
                    error,
                    nekonetease_net::services::net::range_reader::MediaError::HttpStatus(
                        401 | 403 | 404 | 410
                    )
                )
            })
        {
            return true;
        }
        cause = std::error::Error::source(current);
    }
    false
}

pub(super) fn track_summary(
    song: Song,
    artwork_path: Option<String>,
    is_favorite: bool,
) -> TrackSummary {
    let artist = song.artist_names();
    let mv_id = song.mv_id();
    TrackSummary {
        id: song.id,
        file_path: String::new(),
        file_name: song.name.clone(),
        title: song.name,
        artist: (!artist.is_empty()).then_some(artist),
        album: (!song.album.name.is_empty()).then_some(song.album.name),
        duration_ms: i64::try_from(song.duration_ms).unwrap_or(i64::MAX),
        artwork_path,
        track_number: None,
        disc_number: None,
        last_position: 0,
        is_favorite,
        rating: 0,
        replaygain_track_gain: None,
        replaygain_track_peak: None,
        replaygain_album_gain: None,
        replaygain_album_peak: None,
        stream: None,
        mv_id,
    }
}

pub(super) fn open_buffered_source(
    url: &str,
    extension: &str,
    reported_size: Option<u64>,
    start: Duration,
) -> AppResult<BufferedAudioSource> {
    let reader =
        RangeReader::open(url).map_err(|error| AppError::network("在线播放源加载失败", error))?;
    let byte_len = reader.byte_len().or(reported_size);
    let mut decoder = FileDecoder::open_reader(reader, extension, byte_len)?;
    if !start.is_zero() {
        decoder
            .try_seek(start)
            .map_err(|error| AppError::Player(format!("在线播放定位失败：{error}")))?;
    }
    BufferedAudioSource::open(move || Ok(decoder))
}

/// Opens a cached entry the way a local file plays, extending its lease so a track kept in
/// rotation is measured from the last time it was heard rather than from its download.
pub(super) fn open_cached_file(path: &Path, start: Duration) -> AppResult<BufferedAudioSource> {
    let mut decoder = FileDecoder::open(path)?;
    if !start.is_zero() {
        decoder
            .try_seek(start)
            .map_err(|error| AppError::Player(format!("缓存播放定位失败：{error}")))?;
    }
    let source = BufferedAudioSource::open(move || Ok(decoder))?;
    if let Err(error) = cloud_cache::mark_used(path) {
        log::warn!("Could not extend the cloud cache entry's lease: {}", describe(&error));
    }
    Ok(source)
}

pub(super) fn decoder_extension(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "flac" => "flac",
        "aac" => "aac",
        "m4a" => "m4a",
        _ => "mp3",
    }
}
