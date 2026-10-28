//! What a decoded stream of samples can come out of, and the vocabulary above it.
//!
//! [`audio`] is the interface the rest of the chain is written against, and it imports nothing.
//! [`decode`] is the Symphonia probe, track pick, decoder build and packet cursor. [`file_decode`]
//! gives it a seekable file source. [`opus`] is the one codec that registry carries itself,
//! Symphonia shipping none.

pub mod aac_config;
pub mod aac_trim;
pub mod audio;
pub mod buffered;
pub mod decode;
pub mod file_decode;
pub mod mkv_trim;
pub mod opus;

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) mod helpers;
}
