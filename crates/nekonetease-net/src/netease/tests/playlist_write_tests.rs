use super::playlist_write::{
    comma_tags, cover_update_payload, decode_cover_token, decode_created_playlist_id, id_list,
    manipulate_tracks_payload, validate_track_ids, validate_write_id,
};
use super::{Client, ep};
use crate::netease::endpoints::Protocol;
use crate::netease::{Error, Result};
use serde_json::json;

#[test]
fn playlist_writes_use_the_registered_routes() {
    for (endpoint, path, protocol) in [
        (ep::PLAYLIST_CREATE, "/api/playlist/create", Protocol::Weapi),
        (ep::PLAYLIST_REMOVE, "/api/playlist/remove", Protocol::Weapi),
        (ep::PLAYLIST_MANIPULATE_TRACKS, "/api/playlist/manipulate/tracks", Protocol::Eapi),
        (ep::PLAYLIST_NAME_UPDATE, "/api/playlist/update/name", Protocol::Eapi),
        (ep::PLAYLIST_DESC_UPDATE, "/api/playlist/desc/update", Protocol::Eapi),
        (ep::PLAYLIST_TAGS_UPDATE, "/api/playlist/tags/update", Protocol::Eapi),
        (ep::PLAYLIST_UNSUBSCRIBE, "/api/playlist/unsubscribe", Protocol::Eapi),
        (ep::NOS_TOKEN_ALLOC, "/api/nos/token/alloc", Protocol::Weapi),
        (ep::PLAYLIST_COVER_UPDATE, "/api/playlist/cover/update", Protocol::Weapi),
    ] {
        assert_eq!(endpoint.path, path);
        assert_eq!(endpoint.protocol, protocol);
        assert!(!endpoint.read_only);
    }
}

#[test]
fn track_manipulation_payload_encodes_ids_as_a_json_string() -> Result<()> {
    let payload = manipulate_tracks_payload("add", 42, &[7, 9])?;
    assert_eq!(payload["op"], "add");
    assert_eq!(payload["pid"], 42);
    assert_eq!(payload["trackIds"], "[7,9]");
    assert_eq!(payload["imme"], json!(true));
    assert_eq!(id_list(&[])?, "[]");
    Ok(())
}

#[test]
fn tags_are_comma_joined_for_the_wire() {
    assert_eq!(comma_tags(&["rock".to_owned(), "pop".to_owned()]), "rock,pop");
    assert_eq!(comma_tags(&[]), "");
}

#[test]
fn created_playlist_id_prefers_the_top_level_field_and_falls_back_to_the_nested_one() -> Result<()>
{
    assert_eq!(decode_created_playlist_id(&json!({"id": 12, "playlist": {"id": 34}}))?, 12);
    assert_eq!(decode_created_playlist_id(&json!({"playlist": {"id": 34}}))?, 34);
    assert!(decode_created_playlist_id(&json!({"id": 0})).is_err());
    assert!(decode_created_playlist_id(&json!({})).is_err());
    Ok(())
}

#[test]
fn write_guards_reject_empty_ids_and_slices() {
    assert!(validate_write_id(0).is_err());
    assert!(validate_write_id(1).is_ok());
    assert!(validate_track_ids(1, &[]).is_err());
    assert!(validate_track_ids(1, &[1, 0]).is_err());
    assert!(validate_track_ids(1, &[1, 2]).is_ok());
}

#[tokio::test]
async fn write_operations_reject_local_inputs_before_any_request() {
    let client = Client::new();
    assert!(matches!(client.create_playlist("  ").await, Err(Error::InvalidResponse)));
    assert!(matches!(client.remove_playlists(&[]).await, Err(Error::InvalidResponse)));
    assert!(matches!(client.remove_playlists(&[1, -1]).await, Err(Error::InvalidResponse)));
    assert!(matches!(client.update_playlist_name(0, "name").await, Err(Error::InvalidResponse)));
    assert!(matches!(client.update_playlist_name(1, " ").await, Err(Error::InvalidResponse)));
    assert!(matches!(client.update_playlist_desc(0, "").await, Err(Error::InvalidResponse)));
    assert!(matches!(client.update_playlist_tags(0, &[]).await, Err(Error::InvalidResponse)));
    for (id, tracks) in [(0_i64, vec![1]), (1, vec![]), (1, vec![0])] {
        assert!(matches!(
            client.add_playlist_tracks(id, &tracks).await,
            Err(Error::InvalidResponse)
        ));
        assert!(matches!(
            client.remove_playlist_tracks(id, &tracks).await,
            Err(Error::InvalidResponse)
        ));
        assert!(matches!(
            client.reorder_playlist_tracks(id, &tracks).await,
            Err(Error::InvalidResponse)
        ));
    }
    assert!(matches!(client.unsubscribe_playlist(0).await, Err(Error::InvalidResponse)));
}

#[test]
fn cover_token_reads_both_text_credentials_and_either_doc_id_shape() -> Result<()> {
    let numeric = decode_cover_token(&json!({
        "result": {"objectKey": "yyimgs/cover.jpg", "token": "secret", "docId": 77}
    }))?;
    assert_eq!(numeric.object_key, "yyimgs/cover.jpg");
    assert_eq!(numeric.doc_id(), 77);
    let textual = decode_cover_token(&json!({
        "result": {"objectKey": "yyimgs/cover.png", "token": "secret", "docId": "78"}
    }))?;
    assert_eq!(textual.doc_id(), 78);
    Ok(())
}

#[test]
fn cover_token_rejects_a_missing_result_credential_or_positive_doc_id() {
    assert!(decode_cover_token(&json!({})).is_err());
    assert!(decode_cover_token(&json!({"result": {"token": "secret", "docId": 1}})).is_err());
    assert!(decode_cover_token(&json!({"result": {"objectKey": "k", "docId": 1}})).is_err());
    assert!(
        decode_cover_token(&json!({"result": {"objectKey": "", "token": "s", "docId": 1}}))
            .is_err()
    );
    assert!(
        decode_cover_token(&json!({"result": {"objectKey": "k", "token": "", "docId": 1}}))
            .is_err()
    );
    assert!(
        decode_cover_token(&json!({"result": {"objectKey": "k", "token": "s", "docId": 0}}))
            .is_err()
    );
}

#[test]
fn playlist_cover_update_names_the_uploaded_doc_id() {
    assert_eq!(cover_update_payload(42, 9001), json!({"id": 42, "coverImgId": 9001}));
}

#[tokio::test]
async fn cover_writes_reject_local_inputs_before_any_request() {
    let client = Client::new();
    assert!(matches!(client.alloc_cover_token("", "jpg").await, Err(Error::InvalidResponse)));
    assert!(matches!(
        client.alloc_cover_token("cover.jpg", " ").await,
        Err(Error::InvalidResponse)
    ));
    assert!(matches!(client.set_playlist_cover(0, 1).await, Err(Error::InvalidResponse)));
    assert!(matches!(client.set_playlist_cover(1, 0).await, Err(Error::InvalidResponse)));
}
