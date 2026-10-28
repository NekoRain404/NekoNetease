//! `NetEase` identities and display data. Temporary CDN URLs never belong in persisted queues.

use serde::{Deserialize, Serialize};

/// Library categories share one playlist grid and detail surface.
///
/// Recommendations is not one of them: the account's suggestions are the Recommended page's,
/// which paints the same special cards and card grid without a category index to keep in step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MusicLibraryCategory {
    #[default]
    Collected,
    Created,
    Roaming,
}

impl MusicLibraryCategory {
    /// The indices are the persisted `music_library_category` values. A file written before
    /// Recommendations retired is folded into this scheme on read
    /// (`services::view_state::fold_retired_music_library_category`).
    pub const fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Created,
            2 => Self::Roaming,
            _ => Self::Collected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoamingMode {
    Personal,
    Heart,
    Similar,
}

/// Synthetic identity outside `NetEase`'s entity range, preserving the shared detail's ID contract.
pub const DAILY_PLAYLIST_ID: i64 = 9_000_000_000_000_001;
pub const PERSONAL_RADAR_ID: i64 = 3_136_952_023;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: i64,
    #[serde(deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "picUrl")]
    pub cover_url: Option<String>,
    #[serde(default, rename = "img1v1Url")]
    pub alternate_cover_url: Option<String>,
}

impl Artist {
    pub fn image_url(&self) -> Option<&str> {
        self.cover_url.as_deref().or(self.alternate_cover_url.as_deref())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: i64,
    #[serde(deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "picUrl")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub id: i64,
    #[serde(deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "ar", alias = "artists")]
    pub artists: Vec<Artist>,
    #[serde(default, rename = "al", alias = "album")]
    pub album: Album,
    #[serde(default, rename = "dt", alias = "duration")]
    pub duration_ms: u64,
    #[serde(default)]
    pub fee: i32,
    #[serde(default)]
    pub privilege: Option<Privilege>,
    /// Set when the service reports that it holds no licence for the recording. `noCopyrightRcmd`
    /// is `null` otherwise, and only its presence matters: its contents are an operator notice.
    #[serde(
        default,
        rename = "noCopyrightRcmd",
        skip_serializing,
        deserialize_with = "deserialize_present"
    )]
    pub no_copyright: bool,
    /// Alternative titles (`alia`) and translated titles (`tns`). Both are shown next to the
    /// primary name rather than replacing it, since search matches either.
    #[serde(default, rename = "alia", alias = "alias")]
    pub alias: Vec<String>,
    #[serde(default, rename = "tns")]
    pub translated_names: Vec<String>,
    /// MV id, `0` when the recording has none.
    #[serde(default)]
    pub mv: i64,
    /// Server popularity score, `0` when the response doesn't carry one.
    #[serde(default, deserialize_with = "crate::entities::tolerant::deserialize_score")]
    pub pop: i32,
}

/// A song entry from the account's special "liked songs" playlist.
///
/// `favorite_order` is the one-based position in the `trackIds` array returned by `NetEase` for
/// that playlist. `added_at_ms` keeps the server's favorite timestamp when the endpoint supplies
/// it; the client must not infer favorite order from a later song-detail response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LikedSong {
    pub song: Song,
    pub favorite_order: i32,
    pub added_at_ms: Option<i64>,
}

/// A song in the account's `NetEase` recent-play history. `played_at_ms` and the server's list
/// position are authoritative; views must preserve the returned order instead of re-sorting it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentSong {
    pub song: Song,
    pub played_at_ms: i64,
}

/// One page of the account's uploaded cloud-disk songs. `total` is the server's whole count, not
/// the length of `songs`, so a paged list knows when to stop asking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudDiskPage {
    pub songs: Vec<Song>,
    pub total: u64,
}

impl Song {
    pub fn artist_names(&self) -> String {
        self.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" / ")
    }

    /// The music video this recording has, or `None` for the wire's `0`.
    pub fn mv_id(&self) -> Option<i64> {
        (self.mv > 0).then_some(self.mv)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Privilege {
    pub id: i64,
    /// Negative when the service has withdrawn the recording; the magnitude is undocumented.
    #[serde(default)]
    pub st: i32,
    /// Highest playable bitrate the account is granted, `0` when it may not play the recording.
    #[serde(default)]
    pub pl: i64,
    /// Highest downloadable bitrate, `0` when it may not download it.
    #[serde(default)]
    pub dl: i64,
    #[serde(default)]
    pub fee: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    #[serde(default, rename = "coverImgUrl", alias = "picUrl")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(
        default,
        rename = "trackCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub track_count: u64,
    #[serde(default)]
    pub creator: Option<PlaylistCreator>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(
        default,
        rename = "playCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub play_count: u64,
    /// False for a playlist the account owns, true for one it only collected.
    #[serde(default, deserialize_with = "crate::entities::tolerant::deserialize_flag")]
    pub subscribed: bool,
    #[serde(
        default,
        rename = "subscribedCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub subscribed_count: u64,
}

/// The public identity of a playlist's owner. The id is what decides whether the signed-in
/// account may edit the playlist; the name and avatar are display data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistCreator {
    #[serde(default, rename = "userId")]
    pub id: i64,
    #[serde(default)]
    pub nickname: String,
    #[serde(default, rename = "avatarUrl")]
    pub avatar_url: Option<String>,
    /// Non-zero when the account holds a paid membership.
    #[serde(default, rename = "vipType")]
    pub vip_type: i32,
    #[serde(default)]
    pub level: i32,
}

/// A public comment on a `NetEase` resource.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The thread-local comment id the like routes address.
    #[serde(default, rename = "commentId")]
    pub id: i64,
    pub author: String,
    pub content: String,
    #[serde(default)]
    pub time_ms: i64,
    #[serde(default)]
    pub liked: bool,
    #[serde(default, rename = "likedCount")]
    pub liked_count: i64,
}

/// The resource a comment thread hangs off. `NetEase` names each family in the wire path itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentThread {
    Song(i64),
    Album(i64),
    Playlist(i64),
    Mv(i64),
}

impl CommentThread {
    /// The typed resource id the comments route expects, such as `R_SO_4_123`.
    pub fn wire_id(self) -> String {
        let (prefix, id) = match self {
            Self::Song(id) => ("R_SO_4", id),
            Self::Album(id) => ("R_AL_3", id),
            Self::Playlist(id) => ("A_PL_0", id),
            Self::Mv(id) => ("R_MV_5", id),
        };
        format!("{prefix}_{id}")
    }

    /// The numeric id carried as `rid` beside the wire id.
    pub const fn id(self) -> i64 {
        match self {
            Self::Song(id) | Self::Album(id) | Self::Playlist(id) | Self::Mv(id) => id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    #[serde(rename = "userId")]
    pub id: i64,
    pub nickname: String,
    #[serde(default, rename = "avatarUrl")]
    pub avatar_url: Option<String>,
    /// Non-zero when the account holds a paid membership.
    #[serde(default, rename = "vipType")]
    pub vip_type: i32,
    #[serde(default)]
    pub level: i32,
}

/// The account's 云贝 balance and whether today's check-in has been claimed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloudBeans {
    pub balance: u64,
    pub signed_today: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Standard,
    Higher,
    #[default]
    Exhigh,
    Lossless,
    Hires,
}

impl Quality {
    /// Bitrate used by the song URL endpoint. `YesPlayMusic` requests 350000 for
    /// lossless and 999000 for Hi-Res instead of sending a format name.
    pub const fn bitrate(self) -> u32 {
        match self {
            Self::Standard => 128_000,
            Self::Higher => 192_000,
            Self::Exhigh => 320_000,
            Self::Lossless => 350_000,
            Self::Hires => 999_000,
        }
    }

    pub const fn level(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Higher => "higher",
            Self::Exhigh => "exhigh",
            Self::Lossless => "lossless",
            Self::Hires => "hires",
        }
    }

    pub fn from_level(level: &str) -> Self {
        match level {
            "standard" => Self::Standard,
            "higher" => Self::Higher,
            "lossless" => Self::Lossless,
            "hires" => Self::Hires,
            _ => Self::Exhigh,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchKind {
    #[default]
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchKind {
    pub const fn code(self) -> u32 {
        match self {
            Self::Songs => 1,
            Self::Albums => 10,
            Self::Artists => 100,
            Self::Playlists => 1000,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub songs: Vec<Song>,
    #[serde(default)]
    pub albums: Vec<Album>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default, rename = "songCount")]
    pub song_count: u64,
    #[serde(default, rename = "albumCount")]
    pub album_count: u64,
    #[serde(default, rename = "artistCount")]
    pub artist_count: u64,
    #[serde(default, rename = "playlistCount")]
    pub playlist_count: u64,
}

impl SearchResult {
    pub const fn total(&self, kind: SearchKind) -> u64 {
        match kind {
            SearchKind::Songs => self.song_count,
            SearchKind::Albums => self.album_count,
            SearchKind::Artists => self.artist_count,
            SearchKind::Playlists => self.playlist_count,
        }
    }
}

/// One music video, as both the artist's listing and the detail route spell it. The listing
/// omits `desc` and often `artistId`, hence the defaults: a card still renders from name, cover,
/// artist and count, and the detail fills the rest in when it opens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mv {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "cover")]
    pub cover_url: Option<String>,
    #[serde(default, rename = "artistId")]
    pub artist_id: i64,
    #[serde(default, rename = "artistName", deserialize_with = "deserialize_nullable_name")]
    pub artist_name: String,
    /// Milliseconds, like `Song::duration_ms`; the wire's `duration` is a float on some rows.
    #[serde(
        default,
        rename = "duration",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub duration_ms: u64,
    #[serde(
        default,
        rename = "playCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub play_count: u64,
    /// Milliseconds since the Unix epoch. Kept numeric; the UI formats it for the locale.
    #[serde(
        default,
        rename = "publishTime",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub publish_time_ms: u64,
    #[serde(default, rename = "desc", deserialize_with = "deserialize_nullable_name")]
    pub description: String,
    #[serde(
        default,
        rename = "subCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub sub_count: u64,
    #[serde(
        default,
        rename = "shareCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub share_count: u64,
    #[serde(
        default,
        rename = "commentCount",
        deserialize_with = "crate::entities::tolerant::deserialize_count"
    )]
    pub comment_count: u64,
    /// The detail envelope's `subed`, which the detail object does not repeat; decoded beside
    /// the row rather than inside it, and never serialized into a queue or a cache.
    #[serde(skip)]
    pub subscribed: bool,
}

/// The playable stream for one MV. `url` is absent while the server refuses the request, which
/// the caller reports rather than treating as a decode failure.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvStream {
    #[serde(default)]
    pub url: Option<String>,
}

/// Why the server did not hand out unrestricted playback.
///
/// A trial still yields a playable URL, so that verdict rides beside playback. Every other
/// variant arrives with an empty URL, which [`StreamTicket::denial`] reports as an error.
///
/// `NetEase` has no field separating a regional block from a copyright-less or withdrawn
/// recording: all three come back as a null URL, and `st < 0`/`pl == 0` mean "not playable here"
/// without saying why. A region lock is therefore reported as [`UnavailableReason::NotAvailable`]
/// rather than guessed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackRestriction {
    /// Only the server's preview window is playable, in seconds.
    Trial { seconds: u64 },
    /// The recording requires a paid membership.
    VipRequired,
    /// The recording has to be bought individually or through its album.
    AlbumPurchaseRequired,
    /// The server withheld the stream for a reason no fee code states.
    Unavailable { reason: UnavailableReason },
}

/// What a refusal can still be shown to be from the song detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// `noCopyrightRcmd` was set: the service holds no licence for the recording.
    NoCopyright,
    /// The privilege row grants no play or download bitrate, or marks the recording withdrawn.
    /// A regional block reports the same shape, so the two are not told apart.
    NotAvailable,
    /// The detail said nothing either; only the stream row's refusal is known.
    Unknown,
}

/// Transient media URL plus the server's permission verdict. Never serialized.
#[derive(Clone)]
pub struct StreamTicket {
    pub song_id: i64,
    pub url: String,
    pub format: String,
    pub expires_after_seconds: u64,
    pub trial: bool,
    pub size: Option<u64>,
    /// The quality the server actually served, which may be below the requested tier.
    pub level: String,
    pub restriction: Option<PlaybackRestriction>,
}

impl StreamTicket {
    /// The reason the server withheld the URL. A playable ticket always carries one, so an empty
    /// `url` is the refusal marker and the decoder fills `restriction` beside it.
    pub fn denial(&self) -> Option<PlaybackRestriction> {
        if self.url.is_empty() { self.restriction } else { None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lyrics {
    pub original: String,
    pub translation: String,
    pub romanization: String,
}

// Tracks can retain their IDs while NetEase returns null artist/album names. Missing display
// metadata must not reject the whole song batch; other malformed field types still fail.
fn deserialize_nullable_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// Whether an object key was present with a body. `noCopyrightRcmd` is an object where there is a
/// licence problem and `null` otherwise, so decoding the body would buy nothing.
fn deserialize_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    Option::<serde::de::IgnoredAny>::deserialize(deserializer).map(|value| value.is_some())
}

#[cfg(test)]
#[path = "tests/cloud_tests.rs"]
mod tests;
