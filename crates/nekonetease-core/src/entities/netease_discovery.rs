//! Boundary models for `NetEase` search and playlist discovery responses.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotSearchItem {
    #[serde(default, rename = "first")]
    pub word: String,
    #[serde(default, rename = "second")]
    pub content: String,
    #[serde(default, rename = "third")]
    pub score: u64,
    #[serde(default, rename = "iconUrl")]
    pub icon_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistCategory {
    pub group_id: u8,
    pub group: String,
    pub name: String,
    pub resource_count: u64,
    pub hot: bool,
    pub image_url: Option<String>,
}
