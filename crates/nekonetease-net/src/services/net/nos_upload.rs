//! The NOS object-storage upload a playlist cover goes through.
//!
//! The image leaves as a raw POST rather than a signed JSON request, so it has its own function
//! here instead of the WEAPI helper: allocate a token, stream the file under it, and hand back
//! what the playlist update names.
//!
//! Neither the token nor the object URL is logged, and the client is the redirect-free one — a
//! redirect would carry `x-nos-token` to whatever host answered.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use tokio::sync::mpsc;

use nekonetease_core::error::AppError;

use super::{build_api_http_client, http_url};

/// Ceiling on one cover upload. The service allocates the remote object before it judges the
/// image, so the bound has to hold before a byte leaves.
pub const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;

const CHUNK_BYTES: usize = 64 * 1024;
const CHUNK_QUEUE: usize = 4;
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The three values `/api/nos/token/alloc` hands back for one upload.
///
/// Deliberately not `Debug`: the token authorises the object, and a panic message or a stray
/// `{:?}` must not be able to print it.
pub struct CoverUploadToken {
    pub(crate) object_key: String,
    pub(crate) token: String,
    pub(crate) doc_id: i64,
}

impl CoverUploadToken {
    pub const fn doc_id(&self) -> i64 {
        self.doc_id
    }
}

/// Upload `path` as the object `token` allocated, declaring `content_type`.
///
/// The file is read on the blocking pool and crosses to the request over a bounded channel, so a
/// cover larger than the expected few megabytes never sits in memory whole.
pub async fn upload_cover_image(
    token: &CoverUploadToken,
    path: &Path,
    content_type: &str,
) -> Result<(), AppError> {
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|e| AppError::io("Cover image could not be read", e))?
        .len();
    if size == 0 || size > MAX_COVER_BYTES {
        return Err(AppError::Validation(format!(
            "Cover image must be between 1 and {MAX_COVER_BYTES} bytes"
        )));
    }
    let client = build_api_http_client()
        .ok_or_else(|| AppError::network_msg("Cover upload is unavailable"))?;
    let url = object_url(&token.object_key)
        .ok_or_else(|| AppError::network_msg("Cover upload URL is invalid"))?;

    let (sender, receiver) = mpsc::channel(CHUNK_QUEUE);
    let reader_path = path.to_path_buf();
    let _reader = tokio::task::spawn_blocking(move || send_file(&reader_path, &sender));
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async move { receiver.recv().await.map(|chunk| (chunk, receiver)) },
    ));
    let response = client
        .post(url)
        .timeout(UPLOAD_TIMEOUT)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, size.to_string())
        .header("x-nos-token", &token.token)
        .body(body)
        .send()
        .await
        .map_err(|e| AppError::network("Cover image could not be uploaded", e))?;
    if response.status().is_success() {
        return Ok(());
    }
    Err(AppError::network_msg(format!("Cover upload returned HTTP {}", response.status().as_u16())))
}

/// The object's URL. The key comes from the service, but it is still parsed into path segments so
/// a key carrying `?` or `#` cannot rewrite the query or drop it.
fn object_url(object_key: &str) -> Option<reqwest::Url> {
    let mut url = http_url("https://nosup-hz1.127.net/yyimgs/")?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        for segment in object_key.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
        }
    }
    url.set_query(Some("offset=0&complete=true&version=1.0"));
    Some(url)
}

/// Feeds `sender` from a blocking read. A closed receiver is the request being dropped, which is
/// the only thing that can cancel this half.
fn send_file(path: &Path, sender: &mpsc::Sender<Result<Vec<u8>, std::io::Error>>) {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            let _ = sender.blocking_send(Err(error));
            return;
        }
    };
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if sender.blocking_send(Ok(buffer[..read].to_vec())).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.blocking_send(Err(error));
                break;
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/nos_upload_tests.rs"]
mod tests;
