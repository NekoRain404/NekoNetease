//! Extended-M3U8 serialization and parsing.
//!
//! Pure, IO-free: [`serialize`] turns a playlist name + ordered tracks into
//! Extended-M3U8 text, [`parse`] turns text back into a [`ParsedM3u`].
//!
//! A path is written verbatim unless the format's own grammar would read it back differently:
//! one with leading or trailing whitespace, a leading `#`, or an embedded line break. Those go
//! out as a single `#NEKONETEASE-PATH:<percent-encoded>` line, which preserves the exact path
//! and which other players ignore as a comment. Ordinary paths stay byte-identical to what
//! earlier versions wrote. A `#` line the parser does not recognise as an extended-M3U directive
//! (`#EXT…`) or as one of this module's own is a path, never a comment — which is what makes the
//! escape necessary and unambiguous in both directions.
//!
//! The parser still reads foreign files: a leading UTF-8 BOM is dropped, and only a trailing
//! `\r` is removed from a line. It never trims a path, because a space can be part of one.
//!
//! NekoNetease's own exports add one non-standard line per track,
//! `#NEKONETEASE-HASH:<blake3-hex>`, which other players ignore as a comment.
//! That hash is what lets a restored playlist re-match a track even after its
//! file was moved.

use std::borrow::Cow;

use nekonetease_core::entities::track::PlaylistExportRow;

const HEADER: &str = "#EXTM3U";
const PLAYLIST_TAG: &str = "#PLAYLIST:";
const EXTINF_TAG: &str = "#EXTINF:";
const HASH_TAG: &str = "#NEKONETEASE-HASH:";
/// The writer's escape for a path line the grammar would otherwise misread. Deliberately
/// starts with `#` so every other player skips it rather than importing a mangled path.
const PATH_TAG: &str = "#NEKONETEASE-PATH:";

/// One parsed playlist entry: a path line plus the comment tags that
/// preceded it. Every field except `path` is best-effort — foreign files
/// may omit any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    /// The path line, verbatim (absolute/relative, native or foreign).
    pub path: String,
    /// BLAKE3 hex from `#NEKONETEASE-HASH` (lowercased), if present.
    pub hash: Option<String>,
    /// Whole-second duration from `#EXTINF` (informational; `None` if
    /// missing, unparseable, or negative/unknown).
    pub duration_secs: Option<i64>,
    /// "Artist - Title" display string from `#EXTINF` (diagnostics only).
    pub display: Option<String>,
}

/// A parsed Extended-M3U8 file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedM3u {
    /// Playlist name from `#PLAYLIST:`, if the file carried one.
    pub playlist_name: Option<String>,
    pub entries: Vec<ParsedEntry>,
}

/// Tags collected since the last path line, waiting for one to attach to.
#[derive(Default)]
struct PendingTags {
    hash: Option<String>,
    duration_secs: Option<i64>,
    display: Option<String>,
}

impl PendingTags {
    fn take_entry(&mut self, path: String) -> ParsedEntry {
        ParsedEntry {
            path,
            hash: self.hash.take(),
            duration_secs: self.duration_secs.take(),
            display: self.display.take(),
        }
    }
}

/// Replace CR/LF with a space so a stray newline in a title/name can't
/// break the single-line tag format.
fn one_line(s: &str) -> Cow<'_, str> {
    if s.contains(['\r', '\n']) {
        Cow::Owned(s.replace(['\r', '\n'], " "))
    } else {
        Cow::Borrowed(s)
    }
}

/// Whether a path can be written raw. Escaping is also what keeps the exported playlist readable
/// in other players: every one of them treats any leading `#` as a comment, even though this
/// parser would accept the line.
fn path_is_plain(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('#')
        && !path.starts_with('\u{FEFF}')
        && !path.starts_with(char::is_whitespace)
        && !path.ends_with(char::is_whitespace)
        && !path.contains(['\r', '\n'])
}

fn escaped_path(path: &str) -> Cow<'_, str> {
    if path_is_plain(path) {
        return Cow::Borrowed(path);
    }
    let mut out = String::with_capacity(PATH_TAG.len() + path.len());
    out.push_str(PATH_TAG);
    percent_encode_into(path, &mut out);
    Cow::Owned(out)
}

/// Percent-encodes every byte outside the URI unreserved set, keeping `/` readable. The reader's
/// decoder is the exact inverse, so the path survives byte for byte on every platform.
fn percent_encode_into(path: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in path.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decodes [`percent_encode_into`]'s output. A malformed escape is left literal and an invalid
/// UTF-8 run is replaced rather than rejected: a foreign file must never fail an import.
fn percent_decode(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The comment forms the parser knows. Anything else beginning with `#` is a path, so a track
/// whose name starts with `#` can be represented at all.
fn is_directive(line: &str) -> bool {
    line.starts_with("#EXT") || line.starts_with(PLAYLIST_TAG) || line.starts_with(HASH_TAG)
}

/// Serialize a playlist (name + ordered tracks) to Extended-M3U8 text.
///
/// Emits `\n` line endings and a trailing newline, UTF-8 without a BOM.
/// `#EXTINF` duration is `duration_ms` rounded to whole seconds (`-1` when
/// unknown); the title field is `"<artist> - <title>"`, or just `"<title>"`
/// when the track has no artist. The `#NEKONETEASE-HASH` line is omitted for
/// tracks with no `file_hash`.
pub fn serialize(playlist_name: &str, tracks: &[PlaylistExportRow]) -> String {
    // Rough capacity: header + name + ~3 lines * ~80 bytes per track.
    let mut out = String::with_capacity(64 + playlist_name.len() + tracks.len() * 240);
    out.push_str(HEADER);
    out.push('\n');
    out.push_str(PLAYLIST_TAG);
    out.push_str(&one_line(playlist_name));
    out.push('\n');

    for t in tracks {
        let secs = if t.duration_ms > 0 { (t.duration_ms + 500) / 1000 } else { -1 };
        out.push_str(EXTINF_TAG);
        out.push_str(&secs.to_string());
        out.push(',');
        match &t.artist {
            Some(a) if !a.is_empty() => {
                out.push_str(&one_line(a));
                out.push_str(" - ");
                out.push_str(&one_line(&t.title));
            }
            _ => out.push_str(&one_line(&t.title)),
        }
        out.push('\n');

        if let Some(h) = &t.file_hash
            && !h.is_empty()
        {
            out.push_str(HASH_TAG);
            out.push_str(&one_line(h));
            out.push('\n');
        }

        out.push_str(&escaped_path(&t.file_path));
        out.push('\n');
    }

    out
}

/// Parse `#EXTINF:<duration>,<display>` content (the part after the tag).
/// Returns `(duration_secs, display)`. Duration tolerates floats and
/// trailing attributes (`#EXTINF:213 tvg-id="x",Title`) by taking the
/// leading integer part; negative/unknown (`-1`) maps to `None`.
fn parse_extinf(rest: &str) -> (Option<i64>, Option<String>) {
    let (dur_str, disp) = match rest.split_once(',') {
        Some((d, t)) => (d, Some(t.trim())),
        None => (rest, None),
    };
    let duration_secs = dur_str
        .split_whitespace()
        .next()
        .and_then(|tok| tok.split('.').next())
        .and_then(|int_part| int_part.parse::<i64>().ok())
        .filter(|&d| d >= 0);
    let display = disp.filter(|d| !d.is_empty()).map(ToOwned::to_owned);
    (duration_secs, display)
}

/// Parse Extended-M3U8 text tolerantly. Strips a leading UTF-8 BOM and a trailing `\r` from each
/// line, ignores blank lines and directives, and treats every other line as a path — including
/// one beginning with `#` that is not a directive. Pending `#EXTINF` / `#NEKONETEASE-HASH` tags
/// attach to the next path, escaped or not. Never errors: an unrecognised file just yields zero
/// entries and the caller decides whether that is fatal.
pub fn parse(content: &str) -> ParsedM3u {
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);

    let mut result = ParsedM3u::default();
    let mut pending = PendingTags::default();

    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() {
            continue;
        }

        if let Some(encoded) = line.strip_prefix(PATH_TAG) {
            let path = percent_decode(encoded);
            result.entries.push(pending.take_entry(path));
            continue;
        }

        if is_directive(line) {
            if let Some(name) = line.strip_prefix(PLAYLIST_TAG) {
                let name = name.trim();
                if !name.is_empty() {
                    result.playlist_name = Some(name.to_owned());
                }
            } else if let Some(rest) = line.strip_prefix(EXTINF_TAG) {
                let (duration_secs, display) = parse_extinf(rest);
                pending.duration_secs = duration_secs;
                pending.display = display;
            } else if let Some(rest) = line.strip_prefix(HASH_TAG) {
                let hash = rest.trim();
                if !hash.is_empty() {
                    pending.hash = Some(hash.to_ascii_lowercase());
                }
            }
            continue;
        }

        result.entries.push(pending.take_entry(line.to_owned()));
    }

    result
}

#[cfg(test)]
#[path = "tests/m3u_tests.rs"]
mod tests;
