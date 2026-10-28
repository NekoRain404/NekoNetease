# Changelog

All notable changes to NekoNetease are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version numbers match
`[workspace.package] version` in `Cargo.toml`.

## [0.1.5]

First tagged release of the NekoNetease line, built on Melodia's player core.

### Added

- **Search and discovery**: songs, albums, artists and playlists with server-side paging,
  a Top Result card, entity navigation into cloud details, search suggestions, hot searches,
  recommended playlists and the full chart catalogue. Charts and roaming are pages of their own,
  and Recommended is a sidebar entry onto the daily playlist.
- **Cloud library**: collected and created playlists, liked songs, recently played, daily
  recommendations, personal FM, heart mode and similar-song roaming. Owned playlists can be
  created, renamed, deleted, reordered by drag, edited (description, tags, cover) and collected.
- **Playback**: direct cloud streaming at a chosen quality tier, with the tier the server
  actually served shown on the Now Playing chips; trial, membership, purchase and unavailable
  refusals are typed and toasted; expired tickets refresh; listening reports persist offline and
  replay.
- **Video**: music videos as a full page (page transport, quality tiers, fullscreen, seek,
  keyboard), a similar-video rail, per-MV like/collect/share and their comment thread.
- **Podcasts**: DJ radio stations, programme lists and programme playback.
- **Comments**: a full thread surface (hot/newest, paging, replies, posting, deleting, liking,
  avatars) reachable from cloud albums, playlists, songs and MVs.
- **Social**: user profiles, follow/unfollow, activity, private messages and notifications.
  Messages are read-only and never persisted.
- **Account**: QR, phone, email and SMS-captcha login, risk-control messages, cloud beans, daily
  check-in, device sessions.
- **Storage**: a bounded offline cache for streamed audio, cloud-disk listing, cloud-disk to local
  matching and a reviewed cloud-to-local playlist sync.
- **Desktop**: tray, media keys, mini player, themes (including Material You and KDE palettes),
  equalizer, ReplayGain, gapless and crossfade playback, visualizer, updater.

### Fixed

- Wire-shape drift no longer blanks a page: counts that arrive as floats or strings and flags
  that arrive as `null` decode tolerantly, and the decode error states which field disagreed.
- Deleting the playing track keeps the queue index and the deck in step, so end-of-stream no
  longer skips the following entry.
- A failed play-count flush keeps its batch for the next attempt; a failed file-watch event is
  retried from a bounded queue before falling back to a full scan.
- A back press with no history target no longer cancels an in-flight detail open.
- Comment and notification paging failures show an inline retry instead of hiding the rows that
  did load.
- The cloud API client never follows a redirect, so the account cookie cannot be carried off-host
  by the protocol's own header.
- Non-UTF-8 file arguments no longer abort startup, and a launch with a very large file selection
  forwards it in several frames rather than dropping it.

### Known limitations

- aarch64 is not built; the packaging steps name `x86_64` throughout.
- Region-locked songs are reported as unavailable without naming the region, because the service
  does not distinguish it on the wire.
- Song wiki is not implemented: no captured response confirms where the prose lives.
- A merged relevance ordering across search kinds is not implementable from these endpoints.
