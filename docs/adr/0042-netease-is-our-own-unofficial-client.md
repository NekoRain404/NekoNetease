# ADR 42: The NetEase client speaks the unofficial protocol itself, and decodes tolerantly

**Status:** Accepted, 2026-10-26

The application had a complete local player and no way to play the library it was named for. The
service publishes no client API, no SDK and no versioned contract for the endpoints a desktop client
uses; what exists is the web player's own traffic, whose request signing, envelope shape and field
types are all implementation detail.

Decision: `nekonetease-net` implements the protocol directly — WEAPI and EAPI request bodies, the
`x-music-u` header path the newer endpoints require, and the cookie the login flow returns — and
every response is decoded through the tolerant visitors in `nekonetease-core`'s
`entities/tolerant.rs`. A field that arrives as a float, a string, `null` or is absent decodes to a
sensible value instead of failing the page.

Alternatives: a third-party SDK crate, a CLI or sidecar process, and treating the protocol as
closed by shipping local playback only.

Trade: the protocol is unversioned and can change under a release with no notice and no changelog.
An SDK crate moves that risk to someone else but moves the fix there too, and the maintained ones
track the web player rather than a desktop client, so their shape (a browser session, a JS bridge)
is not the shape of this tree. The tolerant decoder is the answer to the part of that risk we can
hold: the service already returns counts as floats and flags as `null`, and a strict `u64` turned
every such page into "unrecognised data" with no clue which field disagreed. It now names the field.
What tolerance cannot fix is a response that genuinely changes meaning, so refusals that matter are
typed — `PlaybackRestriction::{Trial, VipRequired, AlbumPurchaseRequired, Unavailable}` — and shown
as their own toast rather than as a network error.

The client never follows a redirect. The session cookie rides an `x-music-u` header, and following
one would carry it to whatever host the response named; `redirect::Policy::none()` is a property of
the client rather than of each call site, so a new endpoint cannot opt out by forgetting.

Nothing here is verified against the service by the test suite. CI has no account and no network
policy that would allow one, so the protocol's fixtures are captured payloads, and the only real
check is the read-only probe in `crates/nekonetease-app/examples/netease_live_probe.rs`, run by hand
against a live session. A change that matters to a user — a field that moved, a tier that is no
longer served, a write that is now refused — is found there or by a user, not by a green build.
That is the cost of building on an interface nobody promised us, and it is worth stating because it
is the reason a failing feature is often a wire problem rather than a bug in this tree.
