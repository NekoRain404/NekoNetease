# NekoNetease direct NetEase API

Protocol reference: [api-enhanced](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/tree/2db6684453297ee86dec0492842f2320b08182d9), commit `2db6684453297ee86dec0492842f2320b08182d9`.
The endpoint registry is `nekonetease-net::netease::endpoints`. YesPlayMusic's wrapper routes are not wire URLs.

| Capability | Wire path | Protocol | Result |
| --- | --- | --- | --- |
| QR login | `/api/login/qrcode/{unikey,client/login}` | EAPI | key, QR state; direct desktop EAPI context, without anonymous-device registration |
| Phone / email | `/api/w/login{,/cellphone}` | WEAPI / EAPI | session, Account |
| Account | `/api/nuser/account/get` | WEAPI | Account; `vipType` and `level` decoded |
| Discovery / charts | `/api/personalized/playlist`, `/api/toplist`, `/api/search/hot` | WEAPI / EAPI | recommended playlists, hot searches; `/api/toplist` has no UI yet |
| Search | `/api/cloudsearch/pc` | EAPI | SearchResult; `type` 1/10/100/1000, `limit`, `offset`, `total=true` |
| Playlist | `/api/v6/playlist/detail` | EAPI | Playlist, ordered songs |
| Playlist comments | `/api/v1/resource/comments/A_PL_0_{id}` | WEAPI | PlaylistComment list; playlist only, preview |
| Song detail | `/api/v3/song/detail` | WEAPI | songs, privileges |
| Album / artist | `/api/v1/{album,artist}/{id}` | WEAPI | detail, songs |
| Playback | `/api/song/enhance/player/url/v1` | XEAPI | transient StreamTicket; `ids`, `level`, `encodeType=flac` |
| Playback fallback | `/api/song/enhance/player/url` | EAPI | transient StreamTicket; `ids`, `br` |
| Lyrics | `/api/song/lyric` | EAPI | original, translation, romanization |
| Daily / FM | `/api/v3/discovery/recommend/songs`, `/api/v1/radio/get` | WEAPI | songs |
| Favorites | `/api/song/like/get`, `/api/radio/like` | WEAPI | IDs, acknowledgement |
| Listening history | `https://clientlog.music.163.com/eapi/feedback/weblog` | EAPI | sequential `startplay`, `play` acknowledgements |
| Saved content | `/api/{user/playlist,album/sublist,artist/sublist}` | WEAPI | entity lists |
| Playlist create | `/api/playlist/create` | WEAPI | playlist id; `{name, privacy, type}` |
| Playlist delete | `/api/playlist/remove` | WEAPI | acknowledgement; `{ids:"[1,2]"}` |
| Playlist rename / description / tags | `/api/playlist/update/name`, `/api/playlist/desc/update`, `/api/playlist/tags/update` | EAPI | acknowledgement |
| Playlist tracks | `/api/playlist/manipulate/tracks` | EAPI | acknowledgement; `{op:"add"\|"del"\|"update", pid, trackIds:"[1,2]", imme:true}` |
| Playlist unsubscribe | `/api/playlist/unsubscribe` | EAPI | acknowledgement; subscribe remains `/api/playlist/subscribe` (WEAPI) |

Search sends `type: kind.code()` with `limit: 30`, `offset` and `total: true`; `SearchResult`
carries the per-kind server counts, and the search page owns the offset, the page summary and
the previous/next state. The four kinds are songs (1), albums (10), artists (100) and playlists
(1000). The search page's empty state also shows the public hot-search feed and recommended
playlists. That section is in the working tree and its UI tests pass; its content and
ordering have not been checked against a real account.

Playback calls `STREAM_V1` (`/api/song/enhance/player/url/v1`, XEAPI) with `ids`, `level` and
`encodeType=flac`. When XEAPI is unavailable, transport-invalid or times out, it retries once
through `STREAM` (`/api/song/enhance/player/url`, EAPI) with `ids` and `br`. The two routes
answer with the same `data[]` row shape. This is the resolution of the earlier documentation
drift: the registry's `STREAM_V1` is the current route, `STREAM` is the fallback, and this file
now matches the code.

`nekonetease_core::entities::cloud::PlaybackRestriction` decodes the server's verdict from
`freeTrialInfo` and `fee`: `Trial { seconds }`, `VipRequired`, `AlbumPurchaseRequired`, and
`Unavailable { reason }` where `UnavailableReason` is `NoCopyright` (the song detail carries a
`noCopyrightRcmd`), `NotAvailable` (`privilege.st < 0`, or `pl == 0` with `dl == 0`) or `Unknown`.
A refused row comes back with an empty URL rather than an error, carrying the `fee`-derived
verdict; the app joins it with the song detail through `cloud_playback/refusal.rs` before it
toasts, so a payable track whose stream row omitted `fee` still reports the right reason.
**A region lock is not distinguishable on the wire** — it is IP-based, answers with the same null
URL as any refusal, and its detail often still grants `pl`/`dl` — so it lands in
`UnavailableReason::Unknown` rather than being guessed at.

The playlist write routes above have an endpoint registration, typed client methods and
`CloudService` methods, and the owned-playlist UI exposes create/rename/delete, add/remove,
tags, subscribe and drag reorder (optimistic, with rollback). They are never automatically
retried. Permissions, failure rollback and request generations still need a real account.

Playlist comments use the typed resource prefix `/api/v1/resource/comments/A_PL_0`, with the
playlist id appended as `_{id}`. The cloud playlist detail previews the four most recent
comments and has loading, error and retry states. Song, album and MV comments are not
implemented. The hot-search feed and recommended-playlist strip are the only discovery UI;
rankings remain registry-only.

XEAPI bootstraps its public key from `/api/bsr/sk/get`, verifies the response HMAC,
then wraps a random AES key with ephemeral X25519 and AES-GCM. Guest sessions
use `/api/register/anonimous`. Credentials are attached only to NetEase API calls,
never to a returned CDN address. Safe reads receive at most two attempts; writes
are never automatically retried. Session generations reject stale responses.

Listening reporting follows API Enhanced's [scrobble module](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/main/module/scrobble.js)
and the local YesPlayMusic `src/electron/scrobble.js`: use the clientlog host, sign
`/api/feedback/weblog` with EAPI, and use its macOS desktop reporting profile.
The ordinary music/WEAPI feedback endpoint is not used for listening history.

`nekonetease-app::tasks::netease_listening` samples the existing player state every 500 ms.
Only signed-in cloud songs with real position advancement count; local database IDs,
pause/buffering time, seek jumps and system suspension do not. A successful cloud-track
start carries a listening generation so repeating the same song creates a new listen,
while seeking or changing quality keeps the current listen. Time is measured in actual
seconds at the selected playback speed, rather than the absolute track position.
On track change, stop, or clean exit the tap queues one `startplay` / `play` pair,
with unknown source ID `0`. Pausing retains the session until it resumes or ends.
The API facade rejects queued records from a previous account generation and clears its
recent-song cache after successful reporting. Failures only produce a sanitized log warning.

Reporting is best effort at the tap: sampling can miss sub-second boundaries and listens
shorter than one second are ignored. A completed listen is attempted immediately through a
bounded 32-slot channel. When that attempt fails, the channel overflows, or exit finds the
channel still holding reports, the listen is appended to `listen-queue.json` under the data
root instead of being dropped. The file is `{ "version": 1, "entries": [...] }` with
`song_id`, `seconds`, `captured_at_ms` and the `account_id` that heard it per entry; it holds
at most 200 entries (oldest evicted first), drops entries older than seven days on load, and is
written through the atomic whole-file writer, so a crash leaves the previous copy intact.
Consecutive identical reports captured within five seconds collapse into one.

A retry pass starts with the app, delivers entries oldest-first for the signed-in account,
stops at the first authentication or transport failure, and backs off from 30 s to 10 min. It
wakes on a new queued report, a session change and at boot with the restored session, and is
cancelled with the account's task token. An entry is never handed to a different account: the
stored `account_id` must match the signed-in one. On clean exit the tap waits up to 1.5 s for
its final report and then writes whatever is still pending.

What can still be lost: a listen the sampler never observed, one that completed while signed
out (there is no account to attribute it to), one still in flight when a forced exit ends the
shutdown budget, and anything evicted by the seven-day or 200-entry bound, or by a queue write
that fails. What can be counted twice: a request that timed out ambiguously, and a delivery
whose queue-file removal did not reach disk before a crash, are retried on the next launch. A
successful acknowledgement confirms receipt, not immediate inclusion in every NetEase
statistic; account totals/footprints require verification in the official client after real
playback and remain subject to the service's counting rules.

QR login follows the YesPlayMusic/API Enhanced request defaults: it requests a QR key
directly over EAPI with the desktop cookie profile and iPhone user agent. It must not
register an anonymous XEAPI session before creating the QR key; that changes the device
context and can trigger the phone's environment-risk rejection.

`nekonetease-app::cloud::CloudService` owns login and account transitions. Successful
sessions store only the NetEase cookie and device identifier in the OS credential
store; passwords are never persisted. When the system store is unavailable, login
works for the current process and the UI receives a notice. Startup restoration
retries transient network failures and removes credentials only after the server
rejects the session. Account transitions cancel the prior account task token.

Run unit tests with `cargo test --locked -p nekonetease-net --lib`.
Run session state checks with `cargo test --locked -p nekonetease-app --lib cloud::tests`.
Run the public protocol probe with `cargo run --locked -p nekonetease-net --example netease_probe`.
The probe prints counts and permission status, never cookies or signed URLs.
Password/QR account validation remains a manual release check with a user's own account.
The playlist write routes, playlist comments, search paging and restriction toasts have
decode/unit tests only; they still need a real-account probe before being called stable.
