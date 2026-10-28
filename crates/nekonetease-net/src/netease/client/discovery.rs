use std::collections::HashSet;

use serde_json::{Value, json};

use nekonetease_core::entities::cloud::{
    Comment, CommentThread, Playlist, SearchKind, SearchResult,
};
use nekonetease_core::entities::comment::{CommentPage, CommentSort, ThreadComment};
use nekonetease_core::entities::netease_discovery::{HotSearchItem, PlaylistCategory};

use crate::netease::endpoints::{self as ep, Protocol};
use crate::netease::{Error, Result};

use super::Client;
use super::decode::{decode_ref, field, number_as_i64, number_as_u64};

impl Client {
    pub async fn discover(&self) -> Result<Vec<Playlist>> {
        field(
            self.request(ep::DISCOVER, json!({"limit":30,"total":true,"n":1000})).await?,
            "result",
        )
    }

    pub async fn hot_search(&self) -> Result<Vec<HotSearchItem>> {
        decode_hot_search(&self.request(ep::HOT_SEARCH, json!({"type":1111})).await?)
    }

    pub async fn playlist_categories(&self) -> Result<Vec<PlaylistCategory>> {
        decode_playlist_categories(&self.request(ep::PLAYLIST_CATEGORIES, json!({})).await?)
    }

    pub async fn hot_playlist_categories(&self) -> Result<Vec<PlaylistCategory>> {
        decode_playlist_tag_categories(
            &self.request(ep::HOT_PLAYLIST_CATEGORIES, json!({})).await?,
            true,
        )
    }

    pub async fn high_quality_playlist_tags(&self) -> Result<Vec<String>> {
        decode_playlist_tags(&self.request(ep::HIGH_QUALITY_TAGS, json!({})).await?)
    }

    pub async fn category_playlists(
        &self,
        category: &str,
        offset: u64,
    ) -> Result<(Vec<Playlist>, bool)> {
        let body = self
            .request(
                ep::CATEGORY_PLAYLISTS,
                json!({"cat":category,"order":"hot","limit":30,"offset":offset,"total":true}),
            )
            .await?;
        decode_cloud_playlist_page(&body)
    }

    pub async fn high_quality_playlists(
        &self,
        category: &str,
        before: Option<i64>,
    ) -> Result<(Vec<Playlist>, Option<i64>, bool)> {
        let body = self
            .request(
                ep::HIGH_QUALITY_PLAYLISTS,
                json!({
                    "cat":category,
                    "limit":30,
                    "lasttime":before.unwrap_or_default(),
                    "total":true
                }),
            )
            .await?;
        decode_high_quality_playlists(&body)
    }

    pub async fn charts(&self) -> Result<Vec<Playlist>> {
        field(self.request(ep::TOPLIST, json!({})).await?, "list")
    }
    pub async fn search(
        &self,
        keywords: &str,
        kind: SearchKind,
        offset: u64,
    ) -> Result<SearchResult> {
        field(
            self.request(
                ep::SEARCH,
                json!({"s":keywords,"type":kind.code(),"limit":30,"offset":offset,"total":true}),
            )
            .await?,
            "result",
        )
    }

    /// The suggest route answers with one `result` object holding up to four row lists; a
    /// row may carry its label under `name` or `title`, and any list may be absent.
    pub async fn search_suggestions(&self, keywords: &str) -> Result<Vec<String>> {
        let body = self.request(ep::SEARCH_SUGGEST, json!({"s":keywords})).await?;
        Ok(decode_search_suggestions(&body))
    }

    pub async fn comments(
        &self,
        thread: CommentThread,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<Comment>, bool)> {
        let path = ep::comments_path(&thread.wire_id());
        let body = self
            .request_at(
                &path,
                Protocol::Weapi,
                json!({
                    "rid": thread.id(),
                    "limit": limit,
                    "offset": offset,
                    "beforeTime": 0,
                }),
            )
            .await?;
        decode_comments(&body, limit)
    }

    pub async fn playlist_comments(&self, id: i64, offset: u64) -> Result<(Vec<Comment>, bool)> {
        self.comments(CommentThread::Playlist(id), offset, 20).await
    }

    /// Likes or unlikes one comment. The two routes take the same typed thread id and comment id;
    /// the account's own state is what changes, so the caller gates this behind a session.
    pub async fn like_comment(
        &self,
        thread: CommentThread,
        comment_id: i64,
        like: bool,
    ) -> Result<()> {
        if thread.id() <= 0 || comment_id <= 0 {
            return Err(Error::InvalidResponse);
        }
        self.request(ep::comment_like_endpoint(like), comment_like_payload(thread, comment_id))
            .await?;
        Ok(())
    }

    /// A page in the requested ordering. The two routes take the same body; only the path
    /// differs, and the hot one names its array `hotComments`.
    pub async fn comments_page(
        &self,
        thread: CommentThread,
        sort: CommentSort,
        offset: u64,
        limit: u64,
    ) -> Result<CommentPage> {
        if thread.id() <= 0 {
            return Err(Error::InvalidResponse);
        }
        let wire_id = thread.wire_id();
        let path = if sort.is_hot() {
            ep::comment_hot_path(&wire_id)
        } else {
            ep::comments_path(&wire_id)
        };
        let body = self
            .request_at(
                &path,
                Protocol::Weapi,
                json!({
                    "rid": thread.id(),
                    "limit": limit,
                    "offset": offset,
                    "beforeTime": 0,
                }),
            )
            .await?;
        decode_comment_page(&body, sort, limit)
    }

    /// The replies under one comment. The floor route pages by a `time` cursor rather than an
    /// offset, so this asks for the first page only and stamps each row with its parent.
    pub async fn comment_replies(
        &self,
        thread: CommentThread,
        parent_id: i64,
        limit: u64,
    ) -> Result<CommentPage> {
        if thread.id() <= 0 || parent_id <= 0 {
            return Err(Error::InvalidResponse);
        }
        let body = self
            .request(
                ep::COMMENT_FLOOR,
                json!({
                    "parentCommentId": parent_id,
                    "threadId": thread.wire_id(),
                    "time": -1,
                    "limit": limit,
                }),
            )
            .await?;
        let page = decode_comment_page(&body, CommentSort::Time, limit)?;
        Ok(CommentPage {
            comments: page
                .comments
                .into_iter()
                .map(|row| ThreadComment {
                    parent_id: if row.parent_id == 0 { parent_id } else { row.parent_id },
                    ..row
                })
                .collect(),
            ..page
        })
    }

    /// Posts a comment, or a reply when `reply_to` names one. The server may echo the stored
    /// row; when it answers only a status the caller fills the content and author it sent.
    pub async fn post_comment(
        &self,
        thread: CommentThread,
        content: &str,
        reply_to: Option<i64>,
    ) -> Result<ThreadComment> {
        if thread.id() <= 0 || content.trim().is_empty() {
            return Err(Error::InvalidResponse);
        }
        let mut data = json!({
            "threadId": thread.wire_id(),
            "content": content,
            "resourceType": "0",
        });
        let endpoint = match reply_to {
            Some(comment_id) if comment_id > 0 => {
                data["commentId"] = json!(comment_id);
                ep::COMMENT_REPLY
            }
            Some(_) => return Err(Error::InvalidResponse),
            None => {
                data["expressionPicId"] = json!("-1");
                data["bubbleId"] = json!("-1");
                ep::COMMENT_ADD
            }
        };
        let body = self.request(endpoint, data).await?;
        let mut posted = ThreadComment {
            comment: decode_posted_comment(&body).unwrap_or_default(),
            ..ThreadComment::default()
        };
        if posted.comment.content.is_empty() {
            content.clone_into(&mut posted.comment.content);
        }
        if posted.parent_id == 0 {
            posted.parent_id = reply_to.unwrap_or_default();
        }
        Ok(posted)
    }

    /// Deletes one of the signed-in account's own comments. The caller gates it on a session,
    /// as with the like routes.
    pub async fn delete_comment(&self, thread: CommentThread, comment_id: i64) -> Result<()> {
        if thread.id() <= 0 || comment_id <= 0 {
            return Err(Error::InvalidResponse);
        }
        self.request(
            ep::COMMENT_DELETE,
            json!({"commentId": comment_id, "threadId": thread.wire_id()}),
        )
        .await?;
        Ok(())
    }
}

/// The like routes carry the thread as its typed wire id rather than a separate prefix field.
pub(super) fn comment_like_payload(thread: CommentThread, comment_id: i64) -> Value {
    json!({"threadId": thread.wire_id(), "commentId": comment_id, "csrf_token": ""})
}

pub(super) fn decode_hot_search(body: &Value) -> Result<Vec<HotSearchItem>> {
    body.get("result")
        .and_then(|result| result.get("hots"))
        .and_then(Value::as_array)
        .ok_or(Error::InvalidResponse)?
        .iter()
        .map(|item| {
            let word = item["first"]
                .as_str()
                .filter(|word| !word.is_empty())
                .ok_or(Error::InvalidResponse)?;
            let content = item["third"].as_str().unwrap_or_default().to_owned();
            Ok(HotSearchItem {
                word: word.to_owned(),
                content,
                score: number_as_u64(&item["second"]).unwrap_or_default(),
                icon_url: item.get("iconUrl").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect()
}

pub(super) fn decode_playlist_categories(body: &Value) -> Result<Vec<PlaylistCategory>> {
    let groups = body["categories"].as_object().ok_or(Error::InvalidResponse)?;
    let subcategories = body["sub"].as_array().ok_or(Error::InvalidResponse)?;
    subcategories
        .iter()
        .map(|subcategory| {
            let group_id = subcategory["category"]
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(Error::InvalidResponse)?;
            let group = groups
                .get(&group_id.to_string())
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(Error::InvalidResponse)?;
            let name = subcategory["name"]
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or(Error::InvalidResponse)?;
            Ok(PlaylistCategory {
                group_id,
                group: group.to_owned(),
                name: name.to_owned(),
                resource_count: number_as_u64(&subcategory["resourceCount"]).unwrap_or_default(),
                hot: subcategory["hot"].as_bool().unwrap_or(false),
                image_url: subcategory
                    .get("imgUrl")
                    .or_else(|| subcategory.get("imageUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub(super) fn decode_playlist_tag_categories(
    body: &Value,
    hot: bool,
) -> Result<Vec<PlaylistCategory>> {
    let tags = body["tags"].as_array().ok_or(Error::InvalidResponse)?;
    tags.iter()
        .map(|tag| {
            let name = tag["name"]
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or(Error::InvalidResponse)?;
            Ok(PlaylistCategory {
                group_id: 0,
                group: "热门".to_owned(),
                name: name.to_owned(),
                resource_count: tag
                    .get("usedCount")
                    .or_else(|| tag.get("playlistCount"))
                    .and_then(number_as_u64)
                    .unwrap_or_default(),
                hot: hot || tag["hot"].as_bool().unwrap_or(false),
                image_url: tag
                    .get("imgUrl")
                    .or_else(|| tag.get("imageUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub(super) fn decode_playlist_tags(body: &Value) -> Result<Vec<String>> {
    body["tags"]
        .as_array()
        .ok_or(Error::InvalidResponse)?
        .iter()
        .map(|tag| {
            tag["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or(Error::InvalidResponse)
        })
        .collect()
}

pub(super) fn decode_cloud_playlist_page(body: &Value) -> Result<(Vec<Playlist>, bool)> {
    let raw = body["playlists"].as_array().ok_or(Error::InvalidResponse)?;
    let playlists = raw.iter().map(decode_ref).collect::<Result<Vec<_>>>()?;
    let more = body["more"].as_bool().unwrap_or(raw.len() >= 30);
    Ok((playlists, more))
}

pub(super) fn decode_high_quality_playlists(
    body: &Value,
) -> Result<(Vec<Playlist>, Option<i64>, bool)> {
    let raw = body["playlists"].as_array().ok_or(Error::InvalidResponse)?;
    let playlists = raw.iter().map(decode_ref).collect::<Result<Vec<_>>>()?;
    let next_before = body.get("lasttime").and_then(number_as_i64).or_else(|| {
        raw.last().and_then(|playlist| playlist.get("updateTime")).and_then(number_as_i64)
    });
    let more = body["more"].as_bool().unwrap_or(raw.len() >= 30);
    Ok((playlists, next_before, more))
}

pub(super) fn decode_comments(body: &Value, limit: u64) -> Result<(Vec<Comment>, bool)> {
    let raw = body["comments"].as_array().ok_or(Error::InvalidResponse)?;
    let comments = raw.iter().map(|row| decode_thread_comment(row).comment).collect();
    let more =
        body["more"].as_bool().unwrap_or(u64::try_from(raw.len()).unwrap_or(u64::MAX) >= limit);
    Ok((comments, more))
}

/// A page's array sits at the top level on the read routes and under `data` on the floor and
/// v2 shapes; a hot page names it `hotComments`. Missing totals and `more` fall back to what
/// the page itself implies, so a terse server still pages correctly.
pub(super) fn decode_comment_page(
    body: &Value,
    sort: CommentSort,
    limit: u64,
) -> Result<CommentPage> {
    let payload = body.get("data").filter(|data| data.is_object()).unwrap_or(body);
    let entries = payload["comments"]
        .as_array()
        .or_else(|| payload["hotComments"].as_array())
        .or_else(|| body["comments"].as_array())
        .or_else(|| body["hotComments"].as_array())
        .ok_or(Error::InvalidResponse)?;
    let comments = entries
        .iter()
        .filter(|entry| entry.is_object())
        .map(decode_thread_comment)
        .collect::<Vec<_>>();
    let count = u64::try_from(comments.len()).unwrap_or(u64::MAX);
    let total = number_as_u64(&payload["total"])
        .or_else(|| number_as_u64(&body["total"]))
        .filter(|total| *total > 0)
        .unwrap_or(count);
    let more = payload["more"]
        .as_bool()
        .or_else(|| payload["moreHot"].as_bool())
        .or_else(|| body["more"].as_bool())
        .or_else(|| body["moreHot"].as_bool())
        .unwrap_or(u64::try_from(entries.len()).unwrap_or(u64::MAX) >= limit);
    Ok(CommentPage { comments, total, more, hot: sort.is_hot() })
}

/// One row, tolerantly. The preview's six fields come off the comment object; the thread fields
/// prefer what the row carries and fall back to the quoted comment (`beReplied`) or the floor
/// summary (`showFloorComment`), which is where a list page keeps them.
pub(super) fn decode_thread_comment(raw: &Value) -> ThreadComment {
    let replied = raw.get("beReplied").and_then(Value::as_array);
    let parent_id = number_as_i64(&raw["parentCommentId"]).unwrap_or_else(|| {
        replied
            .and_then(|rows| rows.first())
            .and_then(|row| number_as_i64(&row["commentId"]))
            .unwrap_or_default()
    });
    let reply_count = number_as_u64(&raw["showFloorComment"]["replyCount"])
        .or_else(|| number_as_u64(&raw["replyCount"]))
        .unwrap_or_default();
    ThreadComment {
        comment: Comment {
            id: number_as_i64(&raw["commentId"]).unwrap_or_default(),
            author: raw["user"]["nickname"]
                .as_str()
                .or_else(|| raw["user"]["remarkName"].as_str())
                .unwrap_or_default()
                .to_owned(),
            content: raw["content"].as_str().unwrap_or_default().to_owned(),
            time_ms: number_as_i64(&raw["time"]).unwrap_or_default(),
            liked: raw["liked"].as_bool().unwrap_or(false),
            liked_count: number_as_i64(&raw["likedCount"]).unwrap_or_default(),
        },
        parent_id,
        reply_count,
        author_id: number_as_i64(&raw["user"]["userId"]).unwrap_or_default(),
        avatar_url: raw["user"]["avatarUrl"].as_str().unwrap_or_default().to_owned(),
    }
}

/// The write routes answer a status with the stored row under `comment`, `data.comment`, or
/// `data` itself; a bare status returns `None` and the caller keeps what it sent.
pub(super) fn decode_posted_comment(body: &Value) -> Option<Comment> {
    let candidates = [
        body.get("comment"),
        body.get("data").and_then(|data| data.get("comment")),
        body.get("data"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|value| value.get("commentId").is_some() || value.get("content").is_some())
        .map(|value| decode_thread_comment(value).comment)
}

const SUGGESTION_LIMIT: usize = 8;

pub(super) fn decode_search_suggestions(body: &Value) -> Vec<String> {
    let result = body.get("result").unwrap_or(body);
    let mut seen = HashSet::new();
    let mut labels = Vec::with_capacity(SUGGESTION_LIMIT);
    for key in ["songs", "artists", "albums", "playlists"] {
        let Some(rows) = result.get(key).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            let Some(label) = row
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| row.get("title").and_then(Value::as_str))
            else {
                continue;
            };
            let label = label.trim();
            if label.is_empty() || !seen.insert(label) {
                continue;
            }
            labels.push(label.to_owned());
            if labels.len() == SUGGESTION_LIMIT {
                return labels;
            }
        }
    }
    labels
}
