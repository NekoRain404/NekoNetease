//! Music-video routes: one MV's detail, stream, collect and like state, plus the two listings the
//! UI browses.

use serde_json::{Value, json};

use nekonetease_core::entities::cloud::{CommentThread, Mv, MvStream};
use nekonetease_core::entities::comment::CommentThreadInfo;

use crate::netease::endpoints as ep;
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, number_as_u64};

impl Client {
    pub async fn mv_detail(&self, id: i64) -> Result<Mv> {
        let body = self.request(ep::MV_DETAIL, json!({"id": id})).await?;
        decode_mv_detail(&body)
    }

    /// A refused stream answers `data: null`, which is a verdict rather than a malformed body:
    /// the caller tells the user no stream is available instead of surfacing a decode error.
    /// `quality` is the endpoint's `r` tier: 1080, 720, 480 or 240.
    pub async fn mv_url(&self, id: i64, quality: u16) -> Result<MvStream> {
        let body = self.request(ep::MV_URL, mv_url_body(id, quality)).await?;
        match body.get("data") {
            Some(data) if !data.is_null() => decode_ref(data),
            _ => Ok(MvStream::default()),
        }
    }

    /// The account's like state and count for an MV's comment thread. The read route is public,
    /// like the thread itself, so an anonymous session still gets the counts.
    pub async fn mv_thread_info(&self, id: i64) -> Result<CommentThreadInfo> {
        if id <= 0 {
            return Err(Error::InvalidResponse);
        }
        let body = self
            .request(
                ep::COMMENT_THREAD_INFO,
                json!({"threadid": CommentThread::Mv(id).wire_id(), "composeliked": true}),
            )
            .await?;
        Ok(decode_thread_info(&body))
    }

    pub async fn mv_like(&self, id: i64, like: bool) -> Result<()> {
        if id <= 0 {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::mv_like_endpoint(like), mv_like_body(id)).await?;
        Ok(())
    }

    pub async fn mv_subscribe(&self, id: i64, subscribed: bool) -> Result<()> {
        if id <= 0 {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::mv_subscription(subscribed), mv_subscribe_body(id)).await?;
        Ok(())
    }

    pub async fn artist_mvs(
        &self,
        artist_id: i64,
        limit: u64,
        offset: u64,
    ) -> Result<(Vec<Mv>, bool)> {
        let body = self
            .request(
                ep::ARTIST_MVS,
                json!({
                    "artistId": artist_id,
                    "limit": limit,
                    "offset": offset,
                    "total": true,
                }),
            )
            .await?;
        let mvs = body.get("mvs").ok_or(Error::InvalidResponse)?;
        let has_more = body.get("hasMore").and_then(Value::as_bool).unwrap_or(false);
        Ok((decode_ref(mvs)?, has_more))
    }

    pub async fn similar_mvs(&self, id: i64) -> Result<Vec<Mv>> {
        let body = self.request(ep::SIMILAR_MVS, json!({"mvid": id})).await?;
        let mvs = body.get("mvs").ok_or(Error::InvalidResponse)?;
        decode_ref(mvs)
    }
}

/// The detail object carries the counts; the collect state sits on the envelope, which is why
/// this decodes the whole body rather than the `data` row alone.
pub(super) fn decode_mv_detail(body: &Value) -> Result<Mv> {
    let data = body.get("data").filter(|data| !data.is_null()).ok_or(Error::InvalidResponse)?;
    let mut mv: Mv = decode_ref(data)?;
    mv.subscribed = body["subed"].as_bool().or_else(|| data["subed"].as_bool()).unwrap_or(false);
    Ok(mv)
}

/// The counts sit at the top level on the reference response and under `data` on the v2 shape.
pub(super) fn decode_thread_info(body: &Value) -> CommentThreadInfo {
    let payload = body.get("data").filter(|data| data.is_object()).unwrap_or(body);
    CommentThreadInfo {
        liked: payload["liked"].as_bool().unwrap_or(false),
        liked_count: number_as_u64(&payload["likedCount"]).unwrap_or_default(),
    }
}

/// The stream request body, kept apart so the chosen tier is testable without a socket.
pub(super) fn mv_url_body(id: i64, quality: u16) -> Value {
    json!({"id": id, "r": quality})
}

/// The resource-like family keys the whole thread, so the MV's own id is wrapped in its prefix.
pub(super) fn mv_like_body(id: i64) -> Value {
    json!({"threadId": CommentThread::Mv(id).wire_id()})
}

/// `mvIds` is a JSON array spelled as a string in the reference clients; the endpoint reads the
/// same id twice, once as a scalar and once as the collection.
pub(super) fn mv_subscribe_body(id: i64) -> Value {
    json!({"mvId": id, "mvIds": format!("[\"{id}\"]")})
}
