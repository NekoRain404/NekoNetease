use super::object_url;

#[test]
fn object_url_pins_the_nos_bucket_and_completion_query() {
    assert_eq!(
        object_url("ab/cover.jpg").as_ref().map(reqwest::Url::as_str),
        Some("https://nosup-hz1.127.net/yyimgs/ab/cover.jpg?offset=0&complete=true&version=1.0")
    );
}

#[test]
fn an_object_key_cannot_rewrite_the_query() -> Result<(), &'static str> {
    let url = object_url("cover.jpg?version=2#fragment").ok_or("object URL must parse")?;
    assert_eq!(url.query(), Some("offset=0&complete=true&version=1.0"));
    assert_eq!(
        url.path_segments().and_then(|mut segments| segments.next_back()),
        Some("cover.jpg%3Fversion=2%23fragment")
    );
    Ok(())
}
