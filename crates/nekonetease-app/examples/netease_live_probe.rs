//! Live read-only probe against the `NetEase` account saved in the OS keyring.
//!
//! `cargo run --release --example netease_live_probe -p nekonetease-app`
//!
//! Prints one `ok <n rows>` or `FAIL <reason>` line per call. Session material and response
//! payloads stay off stdout; only row counts and an error's own one-line message are shown.
//! `--wiki` adds the same check for the song-wiki page, printing fact and paragraph counts.

#![allow(clippy::print_stdout)]

use std::fmt::Display;
use std::process::ExitCode;

use nekonetease_core::entities::cloud::{CommentThread, SearchKind, Song};
use nekonetease_core::entities::comment::CommentSort;
use nekonetease_net::netease::Client;
use serde::Deserialize;

const KEYRING_SERVICE: &str = "NekoNetease";
const KEYRING_USER: &str = "netease-session";
const SESSION_VERSION: u8 = 1;
const SEARCH_TERM: &str = "周杰伦";
const COMMENT_LIMIT: u64 = 20;
const DISK_LIMIT: u64 = 20;
const MV_PAGE: u64 = 30;

/// The shape `cloud::credential` stores. That module keeps it private and an example is its own
/// crate, so this copy is the alternative to a `#[doc(hidden)]` accessor in production code.
#[derive(Deserialize)]
struct StoredSession {
    version: u8,
    device_id: String,
    cookie: String,
}

fn valid_device_id(value: &str) -> bool {
    value.len() == 52 && value.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

fn decode_session(value: &str) -> Option<StoredSession> {
    let session = serde_json::from_str::<StoredSession>(value).ok()?;
    (session.version == SESSION_VERSION
        && valid_device_id(&session.device_id)
        && !session.cookie.is_empty()
        && !session.cookie.contains(['\r', '\n']))
    .then_some(session)
}

fn read_credential() -> Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error),
    }
}

fn report_exit(message: &str) -> ExitCode {
    println!("{message}");
    ExitCode::FAILURE
}

struct Probe {
    failures: usize,
}

impl Probe {
    fn ok(label: &str, rows: usize) {
        println!("ok {label} {rows} rows");
    }

    fn fail(&mut self, label: &str, error: &impl Display) {
        self.failures += 1;
        println!("FAIL {label} {error}");
    }

    fn fail_message(&mut self, label: &str, message: &str) {
        self.failures += 1;
        println!("FAIL {label} {message}");
    }
}

/// The song wiki, over a spread of recordings: some pages carry review prose and some only facts.
async fn probe_wiki(client: &Client, probe: &mut Probe) {
    const TERMS: [&str; 10] = [
        "晴天",
        "七里香",
        "青花瓷",
        "夜曲",
        "月亮代表我的心",
        "童年",
        "山丘",
        "红豆",
        "十年",
        "后来",
    ];
    for term in TERMS {
        let id = match client.search(term, SearchKind::Songs, 0).await {
            Ok(result) => result.songs.first().map(|song| song.id),
            Err(error) => {
                probe.fail(term, &error);
                None
            }
        };
        let Some(id) = id else { continue };
        match client.song_about(id).await {
            Ok(about) => {
                println!(
                    "about {term} facts={} reviews={}",
                    about.facts.len(),
                    about.reviews.len()
                );
            }
            Err(error) => probe.fail(term, &error),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let secret = match tokio::task::spawn_blocking(read_credential).await {
        Ok(Ok(Some(secret))) => secret,
        Ok(Ok(None)) => {
            return report_exit(
                "no NetEase session in the OS keyring; sign in to NekoNetease first",
            );
        }
        Ok(Err(error)) => {
            return report_exit(&format!("could not read the OS keyring: {error}"));
        }
        Err(error) => return report_exit(&format!("the keyring read failed: {error}")),
    };
    let Some(session) = decode_session(&secret) else {
        return report_exit("the stored session does not match the app's format; sign in again");
    };
    let client = match Client::with_device_id(session.device_id) {
        Ok(client) => client,
        Err(error) => return report_exit(&format!("could not build the NetEase client: {error}")),
    };
    client.restore_cookie(&session.cookie).await;
    println!("session restored from the OS keyring");

    let mut probe = Probe { failures: 0 };

    match client.account().await {
        Ok(_) => Probe::ok("account()", 1),
        Err(error) => probe.fail("account()", &error),
    }

    let mut artist_id = None;
    let mut mv_id = None;
    match client.search(SEARCH_TERM, SearchKind::Songs, 0).await {
        Ok(result) => {
            Probe::ok("search(Songs)", result.songs.len());
            if let Some(song) = result.songs.first() {
                artist_id = song.artists.first().map(|artist| artist.id);
            }
            mv_id = result.songs.iter().find_map(Song::mv_id);
        }
        Err(error) => probe.fail("search(Songs)", &error),
    }

    let mut album_id = None;
    match client.search(SEARCH_TERM, SearchKind::Albums, 0).await {
        Ok(result) => {
            Probe::ok("search(Albums)", result.albums.len());
            album_id = result.albums.first().map(|album| album.id);
        }
        Err(error) => probe.fail("search(Albums)", &error),
    }

    match client.chart_catalogue().await {
        Ok(charts) => Probe::ok("chart_catalogue()", charts.len()),
        Err(error) => probe.fail("chart_catalogue()", &error),
    }

    match mv_id {
        Some(id) => match client.mv_detail(id).await {
            Ok(_) => Probe::ok("mv_detail()", 1),
            Err(error) => probe.fail("mv_detail()", &error),
        },
        None => probe.fail_message("mv_detail()", "no MV id in the song search result"),
    }

    match album_id {
        Some(id) => {
            match client
                .comments_page(CommentThread::Album(id), CommentSort::Hot, 0, COMMENT_LIMIT)
                .await
            {
                Ok(page) => Probe::ok("comments_page(Album)", page.comments.len()),
                Err(error) => probe.fail("comments_page(Album)", &error),
            }
        }
        None => {
            probe.fail_message("comments_page(Album)", "no album id in the album search result");
        }
    }

    match client.cloud_disk_songs(0, DISK_LIMIT).await {
        Ok(page) => Probe::ok("cloud_disk_songs(0, 20)", page.songs.len()),
        Err(error) => probe.fail("cloud_disk_songs(0, 20)", &error),
    }

    match client.daily().await {
        Ok(songs) => Probe::ok("daily()", songs.len()),
        Err(error) => probe.fail("daily()", &error),
    }

    match client.recommendations().await {
        Ok(playlists) => Probe::ok("recommendations()", playlists.len()),
        Err(error) => probe.fail("recommendations()", &error),
    }

    match artist_id {
        Some(id) => match client.artist_mvs(id, MV_PAGE, 0).await {
            Ok((mvs, _)) => Probe::ok("artist_mvs()", mvs.len()),
            Err(error) => probe.fail("artist_mvs()", &error),
        },
        None => probe.fail_message("artist_mvs()", "no artist id in the song search result"),
    }

    match mv_id {
        Some(id) => match client.similar_mvs(id).await {
            Ok(mvs) => Probe::ok("similar_mvs()", mvs.len()),
            Err(error) => probe.fail("similar_mvs()", &error),
        },
        None => probe.fail_message("similar_mvs()", "no MV id in the song search result"),
    }

    match client.dj_recommend().await {
        Ok(radios) => Probe::ok("dj_recommend()", radios.len()),
        Err(error) => probe.fail("dj_recommend()", &error),
    }

    // `CloudService::roaming_songs(Personal, None)` issues exactly this request.
    match client.fm().await {
        Ok(songs) => Probe::ok("roaming_songs(Personal, None)", songs.len()),
        Err(error) => probe.fail("roaming_songs(Personal, None)", &error),
    }

    if std::env::args().any(|argument| argument == "--wiki") {
        probe_wiki(&client, &mut probe).await;
    }
    if probe.failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
