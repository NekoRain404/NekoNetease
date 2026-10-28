//! The wrapper routes in `YesPlayMusic` are not upstream URLs. This table records the wire paths.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Weapi,
    Eapi,
    Xeapi,
}

#[derive(Clone, Copy, Debug)]
pub struct Endpoint<'a> {
    pub path: &'a str,
    pub protocol: Protocol,
    pub read_only: bool,
}

impl<'a> Endpoint<'a> {
    pub const fn read(path: &'a str, protocol: Protocol) -> Self {
        Self { path, protocol, read_only: true }
    }

    pub const fn write(path: &'a str, protocol: Protocol) -> Self {
        Self { path, protocol, read_only: false }
    }
}

use Protocol::{Eapi, Weapi, Xeapi};

pub const QR_KEY: Endpoint<'static> = Endpoint::read("/api/login/qrcode/unikey", Eapi);
pub const QR_CHECK: Endpoint<'static> = Endpoint::read("/api/login/qrcode/client/login", Eapi);
pub const REGISTER_ANONYMOUS: Endpoint<'static> = Endpoint::write("/api/register/anonimous", Xeapi);
pub const PHONE_LOGIN: Endpoint<'static> = Endpoint::write("/api/w/login/cellphone", Weapi);
pub const EMAIL_LOGIN: Endpoint<'static> = Endpoint::write("/api/w/login", Eapi);
pub const ACCOUNT: Endpoint<'static> = Endpoint::read("/api/nuser/account/get", Weapi);
/// 云贝 balance and signed state. `docs/netease-api.md` predates the currency, so the route is
/// pinned from api-enhanced's `module/yunbei_info.js` and chaunsin's Go SDK (`YunBeiUserInfo`).
pub const CLOUD_BEANS: Endpoint<'static> = Endpoint::read("/api/v1/user/info", Weapi);
/// Claims today's 云贝 check-in. The same two clients name the path; the SDK's `YunBeiSignIn` is
/// the one that spells it WEAPI, and a repeated claim answers `code: 200` with `data.sign: false`.
pub const CLOUD_CHECK_IN: Endpoint<'static> = Endpoint::write("/api/pointmall/user/sign", Weapi);
pub const REFRESH: Endpoint<'static> = Endpoint::write("/api/login/token/refresh", Weapi);
pub const LOGOUT: Endpoint<'static> = Endpoint::write("/api/logout", Weapi);
pub const DISCOVER: Endpoint<'static> = Endpoint::read("/api/personalized/playlist", Weapi);
pub const TOPLIST: Endpoint<'static> = Endpoint::read("/api/toplist", Eapi);
pub const SEARCH: Endpoint<'static> = Endpoint::read("/api/cloudsearch/pc", Eapi);
pub const SEARCH_SUGGEST: Endpoint<'static> = Endpoint::read("/api/search/suggest/web", Weapi);
pub const HOT_SEARCH: Endpoint<'static> = Endpoint::read("/api/search/hot", Weapi);
pub const DEFAULT_SEARCH: Endpoint<'static> =
    Endpoint::read("/api/search/defaultkeyword/get", Weapi);
pub const PLAYLIST_CATEGORIES: Endpoint<'static> = Endpoint::read("/api/playlist/catalogue", Eapi);
pub const HOT_PLAYLIST_CATEGORIES: Endpoint<'static> =
    Endpoint::read("/api/playlist/hottags", Weapi);
pub const HIGH_QUALITY_PLAYLISTS: Endpoint<'static> =
    Endpoint::read("/api/playlist/highquality/list", Weapi);
pub const HIGH_QUALITY_TAGS: Endpoint<'static> =
    Endpoint::read("/api/playlist/highquality/tags", Weapi);
pub const CATEGORY_PLAYLISTS: Endpoint<'static> = Endpoint::read("/api/playlist/list", Weapi);
pub const PLAYLIST: Endpoint<'static> = Endpoint::read("/api/v6/playlist/detail", Eapi);
pub const COMMENTS: Endpoint<'static> = Endpoint::read("/api/v1/resource/comments", Weapi);
pub const PLAYLIST_COMMENTS: Endpoint<'static> =
    Endpoint::read("/api/v1/resource/comments/A_PL_0", Weapi);
/// The same page in popularity order. It spells the array `hotComments`; `comment_hot_path`
/// appends the typed resource id exactly as `/api/v1/resource/comments` does.
pub const COMMENT_HOT: Endpoint<'static> = Endpoint::read("/api/v1/resource/hotcomments", Weapi);
/// Replies under one comment, cursor-paged by `time` rather than by `offset`.
pub const COMMENT_FLOOR: Endpoint<'static> =
    Endpoint::read("/api/resource/comment/floor/get", Weapi);
/// The account's like state for a whole thread, with the counts its header shows. The body
/// names the typed resource id (`threadid`), which is why the route outlives one resource family.
pub const COMMENT_THREAD_INFO: Endpoint<'static> =
    Endpoint::read("/api/comment/commentthread/info", Weapi);
/// The three write routes follow API Enhanced's `xeapi` spelling. The maintained
/// `NeteaseCloudMusicApi` fork sends the same paths over `weapi` (and its generic route over
/// `eapi`), so the routes are corroborated while the protocol is the reference's pick.
pub const COMMENT_ADD: Endpoint<'static> = Endpoint::write("/api/resource/comments/add", Xeapi);
pub const COMMENT_REPLY: Endpoint<'static> =
    Endpoint::write("/api/v1/resource/comments/reply", Xeapi);
pub const COMMENT_DELETE: Endpoint<'static> =
    Endpoint::write("/api/resource/comments/delete", Xeapi);
pub const COMMENT_LIKE: Endpoint<'static> = Endpoint::write("/api/comment/like", Weapi);
pub const COMMENT_UNLIKE: Endpoint<'static> = Endpoint::write("/api/comment/unlike", Weapi);

/// The two comment like routes differ only in their path; both take the same thread and id.
pub const fn comment_like_endpoint(like: bool) -> Endpoint<'static> {
    if like { COMMENT_LIKE } else { COMMENT_UNLIKE }
}

/// The route takes the whole typed resource id, such as `R_SO_4_42`, as one path segment.
pub fn comments_path(wire_id: &str) -> String {
    format!("{}/{wire_id}", COMMENTS.path)
}

/// The hot page's typed path, the counterpart of [`comments_path`].
pub fn comment_hot_path(wire_id: &str) -> String {
    format!("{}/{wire_id}", COMMENT_HOT.path)
}

/// Playlist comment resources append the playlist id to the typed resource prefix.
pub fn playlist_comments_path(id: i64) -> String {
    format!("{}_{id}", PLAYLIST_COMMENTS.path)
}

pub const SONGS: Endpoint<'static> = Endpoint::read("/api/v3/song/detail", Weapi);
pub const STREAM: Endpoint<'static> = Endpoint::read("/api/song/enhance/player/url", Eapi);
/// The current playback route. `STREAM` stays registered as the fallback for sessions XEAPI
/// refuses; the two answer with the same row shape.
pub const STREAM_V1: Endpoint<'static> = Endpoint::read("/api/song/enhance/player/url/v1", Xeapi);
pub const LYRIC: Endpoint<'static> = Endpoint::read("/api/song/lyric", Eapi);
pub const DAILY: Endpoint<'static> = Endpoint::read("/api/v3/discovery/recommend/songs", Weapi);
pub const RECOMMENDATIONS: Endpoint<'static> =
    Endpoint::read("/api/v1/discovery/recommend/resource", Weapi);
pub const INTELLIGENCE: Endpoint<'static> =
    Endpoint::read("/api/playmode/intelligence/list", Weapi);
pub const SIMILAR_SONGS: Endpoint<'static> = Endpoint::read("/api/v1/discovery/simiSong", Weapi);
/// MV routes. `mv/detail` is the v1 path the api-enhanced fork serves; `mv/url` still lives under
/// the song-enhance family and takes the quality hint `r`, not a bitrate.
pub const MV_DETAIL: Endpoint<'static> = Endpoint::read("/api/v1/mv/detail", Weapi);
pub const MV_URL: Endpoint<'static> = Endpoint::read("/api/song/enhance/play/mv/url", Weapi);
/// Paginated artist filmography: `artistId` + `limit` + `offset`, answered with `mvs`/`hasMore`.
pub const ARTIST_MVS: Endpoint<'static> = Endpoint::read("/api/artist/mvs", Weapi);
/// The song wiki's "About this song" block page, EAPI with the song's numeric id.
pub const SONG_WIKI_PAGE: Endpoint<'static> =
    Endpoint::read("/api/song/play/about/block/page", Eapi);
pub const SIMILAR_MVS: Endpoint<'static> = Endpoint::read("/api/discovery/simiMV", Weapi);
/// Collecting an MV is its own pair, not the album/playlist subscription family; the body names
/// the video's own numeric id.
pub const MV_SUB: Endpoint<'static> = Endpoint::write("/api/mv/sub", Weapi);
pub const MV_UNSUB: Endpoint<'static> = Endpoint::write("/api/mv/unsub", Weapi);
/// Liking a resource is one family keyed by the typed thread id, so the MV's own like route is
/// the same pair a song's comments use, carrying `R_MV_5_<id>`.
pub const MV_LIKE: Endpoint<'static> = Endpoint::write("/api/resource/like", Weapi);
pub const MV_UNLIKE: Endpoint<'static> = Endpoint::write("/api/resource/unlike", Weapi);
pub const FM: Endpoint<'static> = Endpoint::read("/api/v1/radio/get", Weapi);
pub const FM_TRASH: Endpoint<'static> = Endpoint::write("/api/radio/trash/add", Weapi);
pub const USER_PLAYLISTS: Endpoint<'static> = Endpoint::read("/api/user/playlist", Weapi);
pub const LIKED: Endpoint<'static> = Endpoint::read("/api/song/like/get", Weapi);
pub const RECENT_SONGS: Endpoint<'static> = Endpoint::read("/api/play-record/song/list", Weapi);
pub const CLOUD_DISK: Endpoint<'static> = Endpoint::read("/api/v1/cloud/get", Weapi);
pub const WEBLOG: Endpoint<'static> = Endpoint::write("/api/feedback/weblog", Eapi);
pub const LIKE: Endpoint<'static> = Endpoint::write("/api/radio/like", Weapi);
pub const PLAYLIST_SUBSCRIBE: Endpoint<'static> = Endpoint::write("/api/playlist/subscribe", Weapi);
pub const ALBUMS: Endpoint<'static> = Endpoint::read("/api/album/sublist", Weapi);
pub const ARTISTS: Endpoint<'static> = Endpoint::read("/api/artist/sublist", Weapi);
pub const ALBUM_UNSUBSCRIBE: Endpoint<'static> = Endpoint::write("/api/album/unsub", Weapi);
pub const ALBUM_SUBSCRIBE: Endpoint<'static> = Endpoint::write("/api/album/sub", Weapi);
pub const PLAYLIST_UNSUBSCRIBE: Endpoint<'static> =
    Endpoint::write("/api/playlist/unsubscribe", Eapi);
pub const PLAYLIST_CREATE: Endpoint<'static> = Endpoint::write("/api/playlist/create", Weapi);
pub const PLAYLIST_REMOVE: Endpoint<'static> = Endpoint::write("/api/playlist/remove", Weapi);
pub const PLAYLIST_MANIPULATE_TRACKS: Endpoint<'static> =
    Endpoint::write("/api/playlist/manipulate/tracks", Eapi);
pub const PLAYLIST_NAME_UPDATE: Endpoint<'static> =
    Endpoint::write("/api/playlist/update/name", Eapi);
pub const PLAYLIST_DESC_UPDATE: Endpoint<'static> =
    Endpoint::write("/api/playlist/desc/update", Eapi);
pub const PLAYLIST_TAGS_UPDATE: Endpoint<'static> =
    Endpoint::write("/api/playlist/tags/update", Eapi);
/// Cover writes are a two-step pair: allocate a NOS token, then point the playlist at the
/// uploaded object's `docId`. Both are WEAPI, as in API Enhanced and ncm-api-rs.
pub const NOS_TOKEN_ALLOC: Endpoint<'static> = Endpoint::write("/api/nos/token/alloc", Weapi);
pub const PLAYLIST_COVER_UPDATE: Endpoint<'static> =
    Endpoint::write("/api/playlist/cover/update", Weapi);
pub const ARTIST_SUBSCRIBE: Endpoint<'static> = Endpoint::write("/api/artist/sub", Weapi);

/// Collection writes use separate upstream routes, rather than a `t` flag on the subscribe route.
pub const fn album_subscription(saved: bool) -> Endpoint<'static> {
    if saved { ALBUM_SUBSCRIBE } else { ALBUM_UNSUBSCRIBE }
}

pub const fn mv_subscription(subscribed: bool) -> Endpoint<'static> {
    if subscribed { MV_SUB } else { MV_UNSUB }
}

pub const fn mv_like_endpoint(like: bool) -> Endpoint<'static> {
    if like { MV_LIKE } else { MV_UNLIKE }
}
