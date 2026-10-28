use super::favorites::decode_saved_albums_page;
use serde_json::json;

#[test]
fn collection_page_preserves_large_ids_and_server_order() -> Result<(), Box<dyn std::error::Error>>
{
    let (albums, more) = decode_saved_albums_page(&json!({
        "data": [
            {"id": 4_000_000_001_i64, "name": "Latest", "artists": [{"id": 7, "name": "Artist"}]},
            {"id": 4_000_000_000_i64, "name": "Earlier", "picUrl": "https://p1.music.126.net/a.jpg"}
        ],
        "hasMore": true
    }))?;
    assert_eq!(
        albums.iter().map(|album| album.id).collect::<Vec<_>>(),
        [4_000_000_001, 4_000_000_000]
    );
    assert_eq!(albums[0].artists[0].name, "Artist");
    assert!(more);
    Ok(())
}

#[test]
fn explicit_end_of_collection_wins_over_a_full_page() -> Result<(), Box<dyn std::error::Error>> {
    let page = (1..=30).map(|id| json!({"id": id, "name": "Album"})).collect::<Vec<_>>();
    let (_, more) = decode_saved_albums_page(&json!({"data": page, "hasMore": false}))?;
    assert!(!more);
    Ok(())
}

#[test]
fn collection_page_uses_more_or_page_size_when_has_more_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let (_, more) = decode_saved_albums_page(&json!({"data": [], "more": true}))?;
    assert!(more);
    let (_, more) = decode_saved_albums_page(&json!({"data": []}))?;
    assert!(!more);
    let page = (1..=30).map(|id| json!({"id": id, "name": "Album"})).collect::<Vec<_>>();
    let (_, more) = decode_saved_albums_page(&json!({"data": page}))?;
    assert!(more);
    assert!(decode_saved_albums_page(&json!({"hasMore": false})).is_err());
    Ok(())
}

#[test]
fn subscribe_and_unsubscribe_use_distinct_write_routes() {
    let saved = super::ep::album_subscription(true);
    let removed = super::ep::album_subscription(false);
    assert_eq!(saved.path, "/api/album/sub");
    assert_eq!(removed.path, "/api/album/unsub");
    assert!(!saved.read_only);
    assert!(!removed.read_only);
    assert_eq!(removed.protocol, super::ep::Protocol::Weapi);
}
