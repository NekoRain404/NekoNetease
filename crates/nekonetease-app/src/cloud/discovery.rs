//! Anonymous discovery endpoints, implemented over the public request gate.

use nekonetease_core::entities::cloud::{
    Account, Album, Artist, Comment, CommentThread, Lyrics, Playlist, Quality, SearchKind,
    SearchResult, Song, StreamTicket,
};
use nekonetease_core::entities::comment::{CommentPage, CommentSort, ThreadComment};
use nekonetease_core::entities::netease_discovery::{HotSearchItem, PlaylistCategory};
use nekonetease_core::error::{AppError, AppResult};

use super::CloudService;
use super::validation::{validate_discovery_category, validate_id, validate_ids};

impl CloudService {
    /// Resolve a short-lived CDN ticket under the current account task generation. Session changes
    /// cancel the HTTP future before its result can reach a player.
    pub async fn stream_ticket(&self, song_id: i64, quality: Quality) -> AppResult<StreamTicket> {
        validate_id("歌曲", song_id)?;
        self.public_request(self.client.stream(song_id, quality)).await
    }

    pub async fn discover(&self) -> AppResult<Vec<Playlist>> {
        self.public_request(self.client.discover()).await
    }

    pub async fn hot_search(&self) -> AppResult<Vec<HotSearchItem>> {
        self.public_request(self.client.hot_search()).await
    }

    pub async fn playlist_categories(&self) -> AppResult<Vec<PlaylistCategory>> {
        self.public_request(self.client.playlist_categories()).await
    }

    pub async fn hot_playlist_categories(&self) -> AppResult<Vec<PlaylistCategory>> {
        self.public_request(self.client.hot_playlist_categories()).await
    }

    pub async fn high_quality_playlist_tags(&self) -> AppResult<Vec<String>> {
        self.public_request(self.client.high_quality_playlist_tags()).await
    }

    pub async fn category_playlists(
        &self,
        category: &str,
        offset: u64,
    ) -> AppResult<(Vec<Playlist>, bool)> {
        let category = validate_discovery_category(category)?;
        self.public_request(self.client.category_playlists(category, offset)).await
    }

    pub async fn high_quality_playlists(
        &self,
        category: &str,
        before: Option<i64>,
    ) -> AppResult<(Vec<Playlist>, Option<i64>, bool)> {
        let category = validate_discovery_category(category)?;
        if before.is_some_and(|cursor| cursor < 0) {
            return Err(AppError::Validation("歌单分页游标无效".to_owned()));
        }
        self.public_request(self.client.high_quality_playlists(category, before)).await
    }

    pub async fn charts(&self) -> AppResult<Vec<Playlist>> {
        self.public_request(self.client.charts()).await
    }

    pub async fn search(
        &self,
        keywords: &str,
        kind: SearchKind,
        offset: u64,
    ) -> AppResult<SearchResult> {
        let keywords = validate_keywords(keywords)?;
        self.public_request(self.client.search(keywords, kind, offset)).await
    }

    /// Suggestions reuse the search box's bounds so a typo past the cap fails where the
    /// committed search would, rather than sending a request it will reject.
    pub async fn search_suggestions(&self, keywords: &str) -> AppResult<Vec<String>> {
        let keywords = validate_keywords(keywords)?;
        self.public_request(self.client.search_suggestions(keywords)).await
    }

    pub async fn songs(&self, ids: &[i64]) -> AppResult<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        validate_ids("歌曲", ids)?;
        self.public_request(self.client.songs(ids)).await
    }

    pub async fn playlist(&self, id: i64) -> AppResult<(Playlist, Vec<Song>)> {
        validate_id("歌单", id)?;
        self.public_request(self.client.playlist(id)).await
    }

    /// The first page of a resource's comment thread. Public on `NetEase`, so the guest session
    /// reaches it and a failed request never needs a login prompt.
    pub async fn comments(
        &self,
        thread: CommentThread,
        offset: u64,
    ) -> AppResult<(Vec<Comment>, bool)> {
        validate_id(thread_label(thread), thread.id())?;
        self.public_request(self.client.comments(thread, offset, 20)).await
    }

    pub async fn playlist_comments(&self, id: i64, offset: u64) -> AppResult<(Vec<Comment>, bool)> {
        self.comments(CommentThread::Playlist(id), offset).await
    }

    /// One page in the chosen ordering. The two routes count offsets independently, so the
    /// caller restarts at zero rather than carrying a cursor across a switch.
    pub async fn comment_page(
        &self,
        thread: CommentThread,
        sort: CommentSort,
        offset: u64,
    ) -> AppResult<CommentPage> {
        validate_id(thread_label(thread), thread.id())?;
        self.public_request(self.client.comments_page(thread, sort, offset, COMMENT_PAGE_SIZE))
            .await
    }

    /// The floor under one comment. Read-only, so the guest session reaches it.
    pub async fn comment_replies(
        &self,
        thread: CommentThread,
        parent_id: i64,
    ) -> AppResult<CommentPage> {
        validate_id(thread_label(thread), thread.id())?;
        validate_id("评论", parent_id)?;
        self.public_request(self.client.comment_replies(thread, parent_id, COMMENT_PAGE_SIZE)).await
    }

    /// Posts a comment or reply under the signed-in account. The server may echo the stored
    /// row; when it doesn't, the text and account the request carried fill it, so the surface
    /// appends exactly what it sent rather than an empty row.
    pub async fn post_comment(
        &self,
        thread: CommentThread,
        content: &str,
        reply_to: Option<i64>,
    ) -> AppResult<ThreadComment> {
        validate_id(thread_label(thread), thread.id())?;
        let content = validate_comment_content(content)?;
        if let Some(parent_id) = reply_to {
            validate_id("评论", parent_id)?;
        }
        let posted =
            self.private_request(self.client.post_comment(thread, content, reply_to)).await?;
        let account = self.status().account;
        Ok(complete_posted_comment(posted, content, reply_to, account.as_ref()))
    }

    /// Deletes one of the account's own comments. The wire names the thread and the comment;
    /// whether the row is the viewer's is the caller's display decision, and the server
    /// refuses anyone else's.
    pub async fn delete_comment(&self, thread: CommentThread, comment_id: i64) -> AppResult<()> {
        validate_id(thread_label(thread), thread.id())?;
        validate_id("评论", comment_id)?;
        self.private_request(self.client.delete_comment(thread, comment_id)).await
    }

    /// Liking a comment changes the signed-in account, so it is a private request; both ids are
    /// checked before the login gate so a stale row fails without prompting.
    pub async fn like_comment(
        &self,
        thread: CommentThread,
        comment_id: i64,
        like: bool,
    ) -> AppResult<()> {
        validate_id(thread_label(thread), thread.id())?;
        validate_id("评论", comment_id)?;
        self.private_request(self.client.like_comment(thread, comment_id, like)).await
    }

    pub async fn album(&self, id: i64) -> AppResult<(Album, Vec<Song>)> {
        validate_id("专辑", id)?;
        self.public_request(self.client.album(id)).await
    }

    pub async fn artist(&self, id: i64) -> AppResult<(Artist, Vec<Song>)> {
        validate_id("歌手", id)?;
        self.public_request(self.client.artist(id)).await
    }

    pub async fn lyrics(&self, song_id: i64) -> AppResult<Lyrics> {
        validate_id("歌曲", song_id)?;
        self.public_request(self.client.lyrics(song_id)).await
    }
}

/// The page size both comment reads ask for. A hot page may be shorter, which is why the
/// cursor advances by what the server returned.
const COMMENT_PAGE_SIZE: u64 = 20;
/// `NetEase` refuses a comment past this length, so the check fails at the gate rather than
/// after a round trip.
const MAX_COMMENT_CHARS: usize = 140;

fn validate_comment_content(content: &str) -> AppResult<&str> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::Validation("评论内容不能为空".to_owned()));
    }
    if content.chars().count() > MAX_COMMENT_CHARS {
        return Err(AppError::Validation(format!("评论不能超过 {MAX_COMMENT_CHARS} 个字符")));
    }
    Ok(content)
}

/// Fills what a terse add response omits. The author comes from the account that posted,
/// and the timestamp is local because the row has to sort and render before the thread is
/// fetched again.
fn complete_posted_comment(
    mut posted: ThreadComment,
    content: &str,
    reply_to: Option<i64>,
    account: Option<&Account>,
) -> ThreadComment {
    if posted.comment.content.is_empty() {
        content.clone_into(&mut posted.comment.content);
    }
    if posted.comment.author.is_empty()
        && let Some(account) = account
    {
        posted.comment.author.clone_from(&account.nickname);
        if posted.author_id == 0 {
            posted.author_id = account.id;
        }
    }
    if posted.avatar_url.is_empty()
        && let Some(avatar) = account.and_then(|account| account.avatar_url.as_deref())
    {
        avatar.clone_into(&mut posted.avatar_url);
    }
    if posted.comment.time_ms <= 0 {
        posted.comment.time_ms = chrono::Utc::now().timestamp_millis();
    }
    if posted.parent_id == 0 {
        posted.parent_id = reply_to.unwrap_or_default();
    }
    posted
}

fn validate_keywords(keywords: &str) -> AppResult<&str> {
    if keywords.trim().is_empty() {
        return Err(AppError::Validation("搜索内容不能为空".to_owned()));
    }
    if keywords.chars().count() > 200 {
        return Err(AppError::Validation("搜索内容不能超过 200 个字符".to_owned()));
    }
    Ok(keywords.trim())
}

/// The user-facing noun for the resource a comment thread hangs off.
fn thread_label(thread: CommentThread) -> &'static str {
    match thread {
        CommentThread::Song(_) => "歌曲",
        CommentThread::Album(_) => "专辑",
        CommentThread::Playlist(_) => "歌单",
        CommentThread::Mv(_) => "MV",
    }
}
