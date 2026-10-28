use crate::netease::Result;
use crate::netease::endpoints::Protocol;
use serde_json::json;

use super::cloud_disk::decode_cloud_disk_page;
use super::ep;

#[test]
fn cloud_disk_reads_the_wrapped_song_and_the_whole_count() -> Result<()> {
    let body = json!({
        "code": 200,
        "count": 128,
        "size": 2,
        "data": [
            {
                "songId": 1,
                "fileName": "first.flac",
                "simpleSong": {
                    "id": 101,
                    "name": "First",
                    "ar": [{"id": 7, "name": "Artist"}],
                    "al": {"id": 8, "name": "Album", "picUrl": "https://p1.music.126.net/a.jpg"},
                    "dt": 215_000
                }
            },
            {
                "songId": 2,
                "simpleSong": {"id": 102, "name": "Second", "dt": 180_000}
            }
        ]
    });

    let page = decode_cloud_disk_page(&body)?;

    assert_eq!(page.total, 128);
    assert_eq!(page.songs.len(), 2);
    assert_eq!(page.songs[0].id, 101);
    assert_eq!(page.songs[0].name, "First");
    assert_eq!(page.songs[0].artist_names(), "Artist");
    assert_eq!(page.songs[0].album.name, "Album");
    assert_eq!(page.songs[0].duration_ms, 215_000);
    assert_eq!(page.songs[1].id, 102);
    Ok(())
}

#[test]
fn cloud_disk_counts_a_numeric_string_and_accepts_an_empty_page() -> Result<()> {
    let empty = json!({"code": 200, "count": "0", "data": []});
    let page = decode_cloud_disk_page(&empty)?;
    assert_eq!(page.total, 0);
    assert_eq!(page.songs.len(), 0);

    let missing_count = json!({"code": 200, "data": []});
    assert_eq!(decode_cloud_disk_page(&missing_count)?.total, 0);
    Ok(())
}

#[test]
fn cloud_disk_route_is_a_read_only_weapi_get() {
    assert_eq!(ep::CLOUD_DISK.path, "/api/v1/cloud/get");
    assert_eq!(ep::CLOUD_DISK.protocol, Protocol::Weapi);
    const { assert!(ep::CLOUD_DISK.read_only) };
}
