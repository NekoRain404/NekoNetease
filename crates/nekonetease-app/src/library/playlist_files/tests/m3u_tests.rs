use super::{parse, serialize};
use nekonetease_core::entities::track::PlaylistExportRow;

fn row(
    path: &str,
    hash: Option<&str>,
    title: &str,
    artist: Option<&str>,
    ms: i64,
) -> PlaylistExportRow {
    PlaylistExportRow {
        file_path: path.to_owned(),
        file_hash: hash.map(ToOwned::to_owned),
        title: title.to_owned(),
        artist: artist.map(ToOwned::to_owned),
        duration_ms: ms,
    }
}

#[test]
fn serialize_emits_header_name_and_tags() {
    let tracks = [row("/music/Song.flac", Some("abc123"), "Get Lucky", Some("Daft Punk"), 248_000)];
    let text = serialize("Roadtrip", &tracks);

    assert!(text.starts_with("#EXTM3U\n"));
    assert!(text.contains("#PLAYLIST:Roadtrip\n"));
    assert!(text.contains("#EXTINF:248,Daft Punk - Get Lucky\n"));
    assert!(text.contains("#NEKONETEASE-HASH:abc123\n"));
    assert!(text.contains("/music/Song.flac\n"));
    assert!(text.ends_with('\n'));
}

#[test]
fn serialize_omits_artist_and_hash_when_absent() {
    let tracks = [row("/music/Untitled.wav", None, "Untitled", None, 0)];
    let text = serialize("X", &tracks);

    // No artist => title only; unknown duration => -1; no hash line.
    assert!(text.contains("#EXTINF:-1,Untitled\n"));
    assert!(!text.contains("#NEKONETEASE-HASH"));
    assert!(!text.contains(" - "));
}

#[test]
fn round_trip_preserves_path_hash_duration_and_name() {
    let tracks = [
        row("/m/a.flac", Some("HASHA"), "A", Some("Artist A"), 200_000),
        row("/m/b.flac", None, "B", None, 0),
    ];
    let parsed = parse(&serialize("My List", &tracks));

    assert_eq!(parsed.playlist_name.as_deref(), Some("My List"));
    assert_eq!(parsed.entries.len(), 2);

    assert_eq!(parsed.entries[0].path, "/m/a.flac");
    assert_eq!(parsed.entries[0].hash.as_deref(), Some("hasha")); // lowercased
    assert_eq!(parsed.entries[0].duration_secs, Some(200));
    assert_eq!(parsed.entries[0].display.as_deref(), Some("Artist A - A"));

    assert_eq!(parsed.entries[1].path, "/m/b.flac");
    assert_eq!(parsed.entries[1].hash, None);
    assert_eq!(parsed.entries[1].duration_secs, None); // -1 => None
}

/// An ordinary path stays exactly what earlier writers produced: escaping is applied only where
/// the grammar demands it, so existing exports are not rewritten.
#[test]
fn an_ordinary_path_is_written_byte_identical() {
    let tracks = [row("/music/Song.flac", Some("abc"), "Song", Some("Artist"), 1000)];
    let text = serialize("List", &tracks);

    assert_eq!(
        text,
        "#EXTM3U\n#PLAYLIST:List\n#EXTINF:1,Artist - Song\n#NEKONETEASE-HASH:abc\n/music/Song.flac\n"
    );
    assert!(!text.contains("#NEKONETEASE-PATH:"));
}

#[test]
fn a_leading_space_path_round_trips_with_its_tags() {
    let tracks = [row(" leading.flac", Some("abc123"), "T", Some("A"), 90_000)];
    let text = serialize("Spaces", &tracks);

    assert!(text.contains("#NEKONETEASE-PATH:%20leading.flac\n"));
    assert!(!text.contains("\n leading.flac\n"));

    let parsed = parse(&text);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, " leading.flac");
    assert_eq!(parsed.entries[0].hash.as_deref(), Some("abc123"));
    assert_eq!(parsed.entries[0].duration_secs, Some(90));
}

#[test]
fn a_trailing_space_path_round_trips() {
    let tracks = [row("trailing.flac ", None, "T", None, 0)];
    let text = serialize("Spaces", &tracks);

    assert!(text.contains("#NEKONETEASE-PATH:trailing.flac%20\n"));

    let parsed = parse(&text);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "trailing.flac ");
}

/// Other players treat every leading `#` as a comment, so the escape is required for them even
/// where this parser would have accepted the raw line.
#[test]
fn a_hash_leading_path_round_trips() {
    let tracks = [row("#1 Hit.flac", None, "T", None, 0)];
    let text = serialize("Hashes", &tracks);

    assert!(text.contains("#NEKONETEASE-PATH:%231%20Hit.flac\n"));

    let parsed = parse(&text);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "#1 Hit.flac");
}

#[test]
fn a_path_that_looks_like_a_directive_round_trips() {
    let tracks = [row("#EXTRA.flac", None, "T", None, 0)];
    let text = serialize("Directive", &tracks);

    let parsed = parse(&text);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "#EXTRA.flac");
}

#[test]
fn parse_tolerates_foreign_m3u_directives_it_does_not_know() {
    // VLC/foobar-style: no #PLAYLIST, no #NEKONETEASE-HASH, extra #EXT directives.
    let text = "#EXTM3U\n\
                #EXTINF:181,Some Artist - Some Title\n\
                #EXTVLCOPT:network-caching=1000\n\
                /home/u/Music/song1.mp3\n\
                #EXTGRP:Rock\n\
                /home/u/Music/song2.mp3\n";
    let parsed = parse(text);

    assert_eq!(parsed.playlist_name, None);
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].path, "/home/u/Music/song1.mp3");
    assert_eq!(parsed.entries[0].duration_secs, Some(181));
    assert_eq!(parsed.entries[0].hash, None);
    assert_eq!(parsed.entries[1].path, "/home/u/Music/song2.mp3");
    // The unknown directive between the two paths must not attach to song2.
    assert_eq!(parsed.entries[1].display, None);
}

/// A `#` line that is not directive-shaped is a path. Treating every `#` line as a comment is
/// what made such a path impossible to express.
#[test]
fn parse_reads_a_non_directive_hash_line_as_a_path() {
    let text = "#EXTM3U\n#1 Hit.mp3\n/m/a.mp3\n";
    let parsed = parse(text);

    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].path, "#1 Hit.mp3");
    assert_eq!(parsed.entries[1].path, "/m/a.mp3");
}

#[test]
fn parse_strips_leading_bom() {
    let text = "\u{FEFF}#EXTM3U\n#PLAYLIST:BOMmy\n/m/a.mp3\n";
    let parsed = parse(text);
    assert_eq!(parsed.playlist_name.as_deref(), Some("BOMmy"));
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "/m/a.mp3");

    // A BOM before the first path of a headerless file is the same case.
    let parsed = parse("\u{FEFF}/m/a.mp3\n");
    assert_eq!(parsed.entries[0].path, "/m/a.mp3");
}

/// `lines()` would hide this, so the reader strips a trailing `\r` itself and leaves every space
/// alone: a space may be part of a path.
#[test]
fn parse_handles_crlf_line_endings_without_trimming_the_path() {
    let text = "#EXTM3U\r\n#PLAYLIST:CRLF\r\n#EXTINF:90,T\r\n/m/a.flac \r\n";
    let parsed = parse(text);
    assert_eq!(parsed.playlist_name.as_deref(), Some("CRLF"));
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "/m/a.flac ");
    assert_eq!(parsed.entries[0].duration_secs, Some(90));
}

#[test]
fn parse_does_not_trim_a_path_line() {
    let parsed = parse("  /m/a.mp3  \n");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].path, "  /m/a.mp3  ");
}

#[test]
fn parse_empty_and_header_only_yield_no_entries() {
    assert_eq!(parse("").entries, []);
    assert_eq!(parse("#EXTM3U\n").entries, []);
    assert_eq!(parse("\n\n\n").entries, []);
}

#[test]
fn parse_extinf_with_float_and_attributes() {
    let text = "#EXTM3U\n#EXTINF:213.7 tvg-id=\"x\",Artist - Title\n/m/a.mp3\n";
    let parsed = parse(text);
    assert_eq!(parsed.entries.len(), 1);
    // Leading integer part is taken; float fraction and attributes ignored.
    assert_eq!(parsed.entries[0].duration_secs, Some(213));
    assert_eq!(parsed.entries[0].display.as_deref(), Some("Artist - Title"));
}

#[test]
fn parse_garbage_extinf_duration_is_none_not_panic() {
    let text = "#EXTM3U\n#EXTINF:notanumber,T\n/m/a.mp3\n";
    let parsed = parse(text);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].duration_secs, None);
    assert_eq!(parsed.entries[0].display.as_deref(), Some("T"));
}

#[test]
fn serialize_neutralizes_newlines_in_title() {
    let tracks = [row("/m/a.mp3", None, "Line1\nLine2", Some("Art\rist"), 1000)];
    let text = serialize("Na\nme", &tracks);
    // The #PLAYLIST and #EXTINF lines must each stay single-line.
    assert!(text.contains("#PLAYLIST:Na me\n"));
    assert!(text.contains("#EXTINF:1,Art ist - Line1 Line2\n"));
}
