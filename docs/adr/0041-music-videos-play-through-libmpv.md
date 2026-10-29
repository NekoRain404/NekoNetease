# ADR 41: Music videos play through libmpv, into a frame the app draws itself

**Status:** Accepted, 2026-10-25

There was no way to watch a music video. The catalogue pages knew an MV's name, cover and comment
thread, and the one thing a viewer came for was missing. Everything else in the app decodes through
Symphonia into a `Sample` stream, and a video has no such shape here: it is a container of
compressed pictures whose decode is a codec's job, arriving at whatever rate the file wants rather
than at a sound card's clock.

Decision: `nekonetease-video` owns one libmpv handle per process and publishes decoded frames as
RGBA buffers through a `watch`. Two threads: an event thread that owns the handle and blocks in
`mpv_wait_event`, and a render thread that owns a software render context and exports the newest
frame. The Slint bridge in `nekonetease-views` copies each frame into a `SharedPixelBuffer`, which is
the only copy of a frame that is unavoidable.

Alternatives: a pure-Rust decoder stack, spawning `mpv` as a separate process against an embedded
window handle, and GStreamer.

Trade: libmpv is the only media C dependency in a binary whose case for existing is that it has
none ([ADR 2](0002-native-rust-desktop-app.md)), and the binding, `libmpv2`, is LGPL-2.1 — one more
licence that every package format owes a text for ([ADR 6](0006-agpl-and-what-artifacts-owe.md)).
Against that: a pure-Rust path would have meant DECODING H.264, H.265 and the container parsing
around them ourselves, which is a project rather than a feature. There is no maintained Rust decoder
to reach for, and the one shape that exists is a binding to the C library under another name.

Embedding a child `mpv` process was the cheap version and the one that does not survive the
platform it is most needed on. Passing a window id is an X11 or Win32 spelling; on Wayland the
surface belongs to the parent's connection, and an external process cannot be handed one. It also
puts the player's window in another process's hands, which is exactly what a Video page, a
fullscreen overlay and a transport built from the app's own components cannot live with.

GStreamer was the remaining C stack and costs plugins rather than a library: the set that must be
installed for an H.264 stream is a distribution's decision, not ours, and "the video page does not
work on this machine" would be a support question with no answer in the log.

What the software render path costs is a copy per frame and no zero-copy handoff into the GPU.
Hardware decoding is still on by default — `hwdec=auto-safe`, which only uses decoders mpv itself
considers safe — and it is what makes a 1080p stream affordable on a laptop; the frame is mapped
back to system memory for export either way. The setting exists because a broken driver is common
enough that the answer cannot be "buy a different GPU": turning it off is one switch, and the
property the UI reads back is the mode that was *requested* rather than the one the decoder
negotiated, because the negotiated answer changes per file and would read as the switch moving on
its own.

The crate deliberately names no UI type. That is what keeps the video dependency out of
`nekonetease-views`' tests and lets the two threads be argued where they are, and it is the same
direction the audio stack already holds
([ADR 8](0008-capability-shaped-playback-sources.md)): the player publishes state and frames, and
whoever owns the surface decides what to do with them.
