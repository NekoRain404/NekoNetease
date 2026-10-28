use serde_json::json;

use super::charts::decode_chart_catalogue;

#[test]
fn chart_catalogue_keeps_every_entry_and_its_tags_in_server_order() -> crate::netease::Result<()> {
    let charts = decode_chart_catalogue(&json!({
        "code": 200,
        "list": [
            {
                "id": 19_723_756,
                "name": "飙升榜",
                "coverImgUrl": "https://p1.music.126.net/rising.jpg",
                "trackCount": 100,
                "tags": [],
                "ToplistType": "S"
            },
            {
                "id": 71_384_707,
                "name": "古典音乐榜",
                "updateFrequency": "每周更新",
                "tags": ["古典", "榜单"],
                "trackCount": 100
            }
        ]
    }))?;

    assert_eq!(charts.len(), 2);
    assert_eq!(charts[0].id, 19_723_756);
    assert_eq!(charts[0].name, "飙升榜");
    assert_eq!(charts[0].tags, Vec::<String>::new());
    assert_eq!(charts[0].cover_url.as_deref(), Some("https://p1.music.126.net/rising.jpg"));
    assert_eq!(charts[1].tags, vec!["古典".to_owned(), "榜单".to_owned()]);
    Ok(())
}

#[test]
fn chart_catalogue_rejects_a_response_without_the_list() {
    assert!(decode_chart_catalogue(&json!({"code": 200})).is_err());
}
