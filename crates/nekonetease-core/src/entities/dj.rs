//! `NetEase` podcast / DJ-radio identities. A programme is a track-shaped row whose playable
//! identity is its `mainSong`, never the programme id.

use serde::{Deserialize, Serialize};

use super::cloud::Song;

/// The account that publishes a radio station. `dj` is the wire key, so the field keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjHost {
    #[serde(default, rename = "userId")]
    pub id: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_name")]
    pub nickname: String,
    #[serde(default, rename = "avatarUrl")]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjRadio {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "picUrl", alias = "coverUrl")]
    pub cover_url: Option<String>,
    #[serde(
        default,
        rename = "desc",
        alias = "description",
        deserialize_with = "deserialize_nullable_name"
    )]
    pub description: String,
    #[serde(default, rename = "programCount", deserialize_with = "deserialize_count")]
    pub program_count: u64,
    #[serde(
        default,
        rename = "subCount",
        alias = "subscriberCount",
        deserialize_with = "deserialize_count"
    )]
    pub subscriber_count: u64,
    #[serde(default, rename = "playCount", deserialize_with = "deserialize_count")]
    pub play_count: u64,
    #[serde(default, deserialize_with = "deserialize_nullable_name")]
    pub category: String,
    #[serde(default, rename = "categoryId")]
    pub category_id: i64,
    #[serde(default, deserialize_with = "deserialize_flag")]
    pub subscribed: bool,
    #[serde(default)]
    pub dj: Option<DjHost>,
}

impl DjRadio {
    pub fn host_name(&self) -> &str {
        self.dj.as_ref().map_or("", |host| host.nickname.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjProgram {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "deserialize_nullable_name")]
    pub name: String,
    #[serde(default, rename = "coverUrl", alias = "picUrl")]
    pub cover_url: Option<String>,
    #[serde(
        default,
        rename = "description",
        alias = "desc",
        deserialize_with = "deserialize_nullable_name"
    )]
    pub description: String,
    #[serde(default, rename = "duration", deserialize_with = "deserialize_count")]
    pub duration_ms: u64,
    #[serde(default, rename = "listenerCount", deserialize_with = "deserialize_count")]
    pub listener_count: u64,
    #[serde(default, rename = "serialNum", deserialize_with = "deserialize_count")]
    pub serial_num: u64,
    #[serde(default, rename = "createTime", deserialize_with = "deserialize_count")]
    pub create_time_ms: u64,
    /// The audio to play. A programme without one is listed but inert.
    #[serde(default, rename = "mainSong")]
    pub main_song: Option<Song>,
    #[serde(default)]
    pub radio: Option<DjRadio>,
    #[serde(default, deserialize_with = "deserialize_flag")]
    pub subscribed: bool,
}

impl DjProgram {
    /// The song id the cloud playback path queues, which is not the programme id.
    pub fn song_id(&self) -> Option<i64> {
        self.main_song.as_ref().map(|song| song.id).filter(|id| *id > 0)
    }

    pub fn artwork_url(&self) -> Option<&str> {
        self.cover_url
            .as_deref()
            .or_else(|| self.radio.as_ref().and_then(|radio| radio.cover_url.as_deref()))
    }

    pub fn host_name(&self) -> &str {
        self.radio.as_ref().map_or("", DjRadio::host_name)
    }
}

/// One page of the hot-radio listing. `more` is the server's answer, or the count comparison when
/// the endpoint omits the flag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DjRadioPage {
    pub radios: Vec<DjRadio>,
    pub more: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DjProgramPage {
    pub programs: Vec<DjProgram>,
    pub more: bool,
}

/// Counts arrive as floats or strings often enough that a strict integer field rejects a response
/// the UI would render fine. Anything unreadable reads as zero, matching `entities::cloud`.
struct CountVisitor;

impl<'de> serde::de::Visitor<'de> for CountVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a count")
    }

    fn visit_u64<E>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_i64<E>(self, value: i64) -> Result<u64, E> {
        Ok(u64::try_from(value).unwrap_or(0))
    }

    fn visit_f64<E>(self, value: f64) -> Result<u64, E> {
        Ok(float_count(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<u64, E> {
        Ok(value.trim().parse().unwrap_or(0))
    }

    fn visit_unit<E>(self) -> Result<u64, E> {
        Ok(0)
    }

    fn visit_none<E>(self) -> Result<u64, E> {
        Ok(0)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<u64, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts are below 2^53 on the wire; the clamp keeps every cast defined"
)]
fn float_count(value: f64) -> u64 {
    value.max(0.0) as u64
}

fn deserialize_count<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(CountVisitor)
}

struct FlagVisitor;

impl<'de> serde::de::Visitor<'de> for FlagVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a flag")
    }

    fn visit_bool<E>(self, value: bool) -> Result<bool, E> {
        Ok(value)
    }

    fn visit_u64<E>(self, value: u64) -> Result<bool, E> {
        Ok(value != 0)
    }

    fn visit_i64<E>(self, value: i64) -> Result<bool, E> {
        Ok(value != 0)
    }

    fn visit_f64<E>(self, value: f64) -> Result<bool, E> {
        Ok(value != 0.0)
    }

    fn visit_str<E>(self, value: &str) -> Result<bool, E> {
        Ok(value == "true" || value == "1")
    }

    fn visit_unit<E>(self) -> Result<bool, E> {
        Ok(false)
    }

    fn visit_none<E>(self) -> Result<bool, E> {
        Ok(false)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

fn deserialize_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(FlagVisitor)
}

fn deserialize_nullable_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}
