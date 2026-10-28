//! The flag struct behind the online lyrics setting.

use serde::{Deserialize, Serialize};

/// Whether lyrics run at all, whether they may be looked up online, and how a line is drawn.
///
/// **Three switches because they sell different things.** The first is the feature: off, the Now
/// Playing column is Up Next and nothing reads, fetches or holds a sheet. The lookup is traffic,
/// and turning it off leaves a sheet already in the store perfectly readable, which is what "no
/// traffic" means and what "no lyrics" would not. Romanization is neither: it is how a line the
/// panel already has is drawn, so it costs nothing and reaches nothing.
///
/// **The first two are off and the third is on**, which is why the `Default` is written out rather
/// than derived. A panel nobody asked for should not take the Up Next column on upgrade, and the
/// shipped package description promises that every online feature is a setting the user controls;
/// but a reader who has turned lyrics on and is looking at a script they cannot sound out wanted
/// this before they knew to ask. It draws nothing at all for a Latin sheet, so "on" costs the other
/// libraries nothing.
///
/// **The three are independent, and the third one especially.** Turning lyrics off and on again
/// says nothing about romanization, so a reader who switched it off gets it back off.
///
/// `settings.json` rather than `views.json` for the feature switch too, for
/// `VisualizerFlags::viz_enabled`'s reason: it is the other Now Playing preference flipped from that
/// view's own header, and a `views.json` flag may not be a bool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsFlags {
    /// Read under its old name too: v0.13.0 shipped it as the column preference it grew out of.
    #[serde(alias = "lyrics_panel_shown")]
    pub lyrics_enabled: bool,
    pub lyrics_online_enabled: bool,
    pub lyrics_romanization_shown: bool,
}

impl Default for LyricsFlags {
    fn default() -> Self {
        Self {
            lyrics_enabled: false,
            lyrics_online_enabled: false,
            lyrics_romanization_shown: true,
        }
    }
}
