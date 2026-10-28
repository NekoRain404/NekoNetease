//! Everything that opens a socket: shared HTTP primitives and the artwork and lyrics fetchers.
//!
//! `http_url` and `read_capped` are the two primitives every fetch shares, and the reason this
//! is a crate rather than a directory: a URL from outside the app is *parsed* rather than
//! prefix-tested, and a body is streamed under a cap rather than allocated and measured after.
//! Both are violable from any file and neither fails visibly, so the corpus walks in
//! `crates/nekonetease/tests/net_primitives.rs` hold them.

pub mod services {
    pub mod net;
}

pub mod media {
    pub mod fetch;
}

pub mod netease;
