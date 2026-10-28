//! Bounded cover URLs and their fetch, shared by every artwork source.

const COVER_SIZE: &str = "512y512";
// The Settings account tile draws 40 px; 160 covers it at 4×, and the view resizes the decode
// to its own tier anyway.
const AVATAR_SIZE: &str = "160y160";

/// Returns a bounded `NetEase` image URL while leaving other HTTP artwork sources intact.
pub fn cover_request_url(source: &str) -> Option<reqwest::Url> {
    sized_request_url(source, COVER_SIZE)
}

/// The avatar at a request size one tile needs, where a cover asks for a full card.
pub(super) fn avatar_request_url(source: &str) -> Option<reqwest::Url> {
    sized_request_url(source, AVATAR_SIZE)
}

fn sized_request_url(source: &str, size: &str) -> Option<reqwest::Url> {
    let mut url = nekonetease_net::services::net::http_url(source)?;
    let host = url.host_str()?;
    if host == "music.126.net" || host.ends_with(".music.126.net") {
        url.set_scheme("https").ok()?;
        if !url.query_pairs().any(|(key, _)| key == "param") {
            url.query_pairs_mut().append_pair("param", size);
        }
    }
    Some(url)
}

/// Fetch a bounded cover body for the view layer without exposing its network response.
pub async fn fetch_cover_bytes(
    client: &reqwest::Client,
    source: &str,
    max_bytes: usize,
) -> Option<Vec<u8>> {
    fetch_bounded(client, cover_request_url(source)?, "Cover image", max_bytes).await
}

/// The avatar counterpart; reached through [`crate::cloud::CloudService`] so a view never builds
/// a client of its own.
pub(super) async fn fetch_avatar_bytes(
    client: &reqwest::Client,
    source: &str,
    max_bytes: usize,
) -> Option<Vec<u8>> {
    fetch_bounded(client, avatar_request_url(source)?, "Avatar image", max_bytes).await
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: reqwest::Url,
    what: &str,
    max_bytes: usize,
) -> Option<Vec<u8>> {
    let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let max_bytes = u64::try_from(max_bytes).ok()?;
    let bytes =
        nekonetease_net::services::net::read_capped(response, what, max_bytes).await.ok()?;
    (!bytes.is_empty()).then_some(bytes)
}
