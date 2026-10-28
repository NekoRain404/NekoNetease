use serde_json::{Value, json};

use nekonetease_core::entities::song_about::SongAbout;

use super::wiki::parse_about;

/// Shaped from a live page: the basic block carries tag resources, language and tempo links, and
/// the review as descriptions two levels down. The memory block above it also holds a
/// `songComment`, and only the basic block's is about the recording.
fn live_page() -> Value {
    json!({
        "code": 200,
        "data": { "blocks": [
            {
                "code": "SONG_PLAY_ABOUT_MUSIC_MEMORY",
                "creatives": [{
                    "creativeType": "songComment",
                    "resources": [{ "uiElement": {
                        "descriptions": [{ "description": "一个听众的记忆" }]
                    }}]
                }]
            },
            {
                "code": "SONG_PLAY_ABOUT_SONG_BASIC",
                "creatives": [
                    {
                        "creativeType": "songTag",
                        "resources": [
                            { "uiElement": { "mainTitle": { "title": "流行" } } },
                            { "uiElement": { "mainTitle": { "title": "流行" } } }
                        ]
                    },
                    { "creativeType": "language", "uiElement": { "textLinks": [{ "text": "国语" }] } },
                    { "creativeType": "bpm", "uiElement": { "textLinks": [{ "text": "120 BPM" }] } },
                    {
                        "creativeType": "songComment",
                        "resources": [
                            { "uiElement": { "descriptions": [
                                { "description": " 第一段 ", "tag": "review" },
                                { "description": "" }
                            ] } },
                            { "uiElement": { "descriptions": [{ "description": "第二段" }] } }
                        ]
                    },
                    {
                        "creativeType": "memoryGrid",
                        "resources": [{ "uiElement": {
                            "descriptions": [{ "description": "不是介绍" }]
                        }}]
                    },
                    { "creativeType": "sheet", "uiElement": { "textLinks": [{ "text": "简谱" }] } }
                ]
            }
        ]}
    })
}

#[test]
fn the_basic_block_yields_review_prose_and_facts() {
    let about = parse_about(&live_page());
    let expected = SongAbout {
        facts: vec!["流行".to_owned(), "国语".to_owned(), "120 BPM".to_owned()],
        reviews: vec!["第一段".to_owned(), "第二段".to_owned()],
    };
    assert_eq!(about, expected);
}

#[test]
fn another_blocks_prose_and_link_modules_stay_out() {
    let about = parse_about(&live_page());
    assert!(!about.reviews.iter().any(|line| line == "一个听众的记忆"));
    assert!(!about.facts.iter().any(|fact| fact == "简谱"));
    assert!(!about.reviews.iter().any(|line| line == "不是介绍"));
}

#[test]
fn a_body_with_no_page_is_empty_rather_than_an_error() {
    let cases = [
        json!({}),
        json!({ "data": { "blocks": [] } }),
        json!({ "data": { "blocks": [{ "code": "SONG_PLAY_ABOUT_SONG_BASIC" }] } }),
        json!({ "data": { "blocks": [{
            "code": "SONG_PLAY_ABOUT_SONG_BASIC",
            "creatives": [{}, { "creativeType": "songTag" }, { "creativeType": "songComment" }]
        }] } }),
    ];
    for body in cases {
        assert!(parse_about(&body).is_empty(), "{body}");
    }
}

#[test]
fn creative_type_matching_ignores_case() {
    let body = json!({ "data": { "blocks": [{
        "code": "SONG_PLAY_ABOUT_SONG_BASIC",
        "creatives": [{
            "creativeType": "SongComment",
            "resources": [{ "uiElement": {
                "descriptions": [{ "description": "一段介绍" }]
            }}]
        }]
    }] } });
    assert_eq!(parse_about(&body).reviews, vec!["一段介绍".to_owned()]);
}
