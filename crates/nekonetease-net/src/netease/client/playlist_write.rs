use serde_json::{Value, json};

use crate::netease::endpoints::{self as ep, Protocol};
use crate::netease::{Error, Result};

use crate::services::net::nos_upload::CoverUploadToken;

use super::Client;
use super::decode::number_as_i64;

impl Client {
    pub async fn subscribe_playlist(&self, id: i64, subscribe: bool) -> Result<()> {
        let path = if subscribe { "/api/playlist/subscribe" } else { "/api/playlist/unsubscribe" };
        self.request_at_write(path, Protocol::Eapi, json!({"id":id})).await?;
        Ok(())
    }
    pub async fn subscribe_album(&self, id: i64, subscribe: bool) -> Result<()> {
        self.request(ep::album_subscription(subscribe), json!({"id":id})).await?;
        Ok(())
    }
    pub async fn subscribe_artist(&self, id: i64, subscribe: bool) -> Result<()> {
        self.request(
            ep::ARTIST_SUBSCRIBE,
            json!({"artistId":id,"t":if subscribe {1}else{2},"artistIds":format!("[{id}]")}),
        )
        .await?;
        Ok(())
    }

    pub async fn create_playlist(&self, name: &str) -> Result<i64> {
        if name.trim().is_empty() {
            return Err(Error::InvalidResponse);
        }
        let body = self
            .request(ep::PLAYLIST_CREATE, json!({"name":name,"privacy":0,"type":"NORMAL"}))
            .await?;
        decode_created_playlist_id(&body)
    }

    pub async fn remove_playlists(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() || ids.iter().any(|id| *id <= 0) {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::PLAYLIST_REMOVE, json!({"ids":id_list(ids)?})).await?;
        Ok(())
    }

    pub async fn update_playlist_name(&self, id: i64, name: &str) -> Result<()> {
        validate_write_id(id)?;
        if name.trim().is_empty() {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::PLAYLIST_NAME_UPDATE, json!({"id":id,"name":name})).await?;
        Ok(())
    }

    pub async fn update_playlist_desc(&self, id: i64, desc: &str) -> Result<()> {
        validate_write_id(id)?;
        self.request(ep::PLAYLIST_DESC_UPDATE, json!({"id":id,"desc":desc})).await?;
        Ok(())
    }

    pub async fn update_playlist_tags(&self, id: i64, tags: &[String]) -> Result<()> {
        validate_write_id(id)?;
        self.request(ep::PLAYLIST_TAGS_UPDATE, json!({"id":id,"tags":comma_tags(tags)})).await?;
        Ok(())
    }

    pub async fn add_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> Result<()> {
        self.manipulate_playlist_tracks("add", id, track_ids).await
    }

    pub async fn remove_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> Result<()> {
        self.manipulate_playlist_tracks("del", id, track_ids).await
    }

    pub async fn reorder_playlist_tracks(&self, id: i64, track_ids: &[i64]) -> Result<()> {
        self.manipulate_playlist_tracks("update", id, track_ids).await
    }

    async fn manipulate_playlist_tracks(&self, op: &str, id: i64, track_ids: &[i64]) -> Result<()> {
        validate_track_ids(id, track_ids)?;
        self.request(ep::PLAYLIST_MANIPULATE_TRACKS, manipulate_tracks_payload(op, id, track_ids)?)
            .await?;
        Ok(())
    }

    pub async fn unsubscribe_playlist(&self, id: i64) -> Result<()> {
        validate_write_id(id)?;
        self.request(ep::PLAYLIST_UNSUBSCRIBE, json!({"id":id})).await?;
        Ok(())
    }

    /// Allocate the NOS object one cover upload writes to.
    ///
    /// `ext` is the file's own extension, without the dot; the service keeps it on the object key
    /// rather than sniffing the bytes later.
    pub async fn alloc_cover_token(&self, filename: &str, ext: &str) -> Result<CoverUploadToken> {
        if filename.trim().is_empty() || ext.trim().is_empty() {
            return Err(Error::InvalidResponse);
        }
        let body = self
            .request(
                ep::NOS_TOKEN_ALLOC,
                json!({
                    "bucket": "yyimgs",
                    "ext": ext,
                    "filename": filename,
                    "local": false,
                    "nos_product": 0,
                    "return_body": r#"{"code":200,"size":"$(ObjectSize)"}"#,
                    "type": "other",
                }),
            )
            .await?;
        decode_cover_token(&body)
    }

    /// Point a playlist at an uploaded cover. `doc_id` is the object's `docId`, not its key.
    pub async fn set_playlist_cover(&self, id: i64, doc_id: i64) -> Result<()> {
        validate_write_id(id)?;
        if doc_id <= 0 {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::PLAYLIST_COVER_UPDATE, cover_update_payload(id, doc_id)).await?;
        Ok(())
    }
}

/// The server reads id lists as a JSON-encoded string inside an otherwise opaque payload, not as
/// an array.
pub(super) fn id_list(ids: &[i64]) -> Result<String> {
    serde_json::to_string(ids).map_err(|_| Error::InvalidResponse)
}

pub(super) fn manipulate_tracks_payload(op: &str, pid: i64, ids: &[i64]) -> Result<Value> {
    Ok(json!({"op": op, "pid": pid, "trackIds": id_list(ids)?, "imme": true}))
}

pub(super) fn comma_tags(tags: &[String]) -> String {
    tags.join(",")
}

pub(super) fn decode_created_playlist_id(body: &Value) -> Result<i64> {
    body.get("id")
        .and_then(number_as_i64)
        .or_else(|| {
            body.get("playlist").and_then(|playlist| playlist.get("id")).and_then(number_as_i64)
        })
        .filter(|id| *id > 0)
        .ok_or(Error::InvalidResponse)
}

pub(super) fn validate_write_id(id: i64) -> Result<()> {
    if id > 0 { Ok(()) } else { Err(Error::InvalidResponse) }
}

pub(super) fn validate_track_ids(id: i64, track_ids: &[i64]) -> Result<()> {
    validate_write_id(id)?;
    if track_ids.is_empty() || track_ids.iter().any(|track_id| *track_id <= 0) {
        return Err(Error::InvalidResponse);
    }
    Ok(())
}

pub(super) fn decode_cover_token(body: &Value) -> Result<CoverUploadToken> {
    let result = body.get("result").ok_or(Error::InvalidResponse)?;
    let text = |field: &str| {
        result
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let object_key = text("objectKey").ok_or(Error::InvalidResponse)?;
    let token = text("token").ok_or(Error::InvalidResponse)?;
    let doc_id = result
        .get("docId")
        .and_then(number_as_i64)
        .filter(|doc_id| *doc_id > 0)
        .ok_or(Error::InvalidResponse)?;
    Ok(CoverUploadToken { object_key, token, doc_id })
}

/// `coverImgId` is the numeric `docId` API Enhanced's upload plugin returns, not the object key.
pub(super) fn cover_update_payload(id: i64, doc_id: i64) -> Value {
    json!({"id": id, "coverImgId": doc_id})
}
