//! Read-only protocol smoke test. No account credentials or CDN URLs are printed.
#![allow(clippy::print_stdout)]

use nekonetease_core::entities::cloud::{Quality, SearchKind};
use nekonetease_net::netease::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    match client.initialize_guest().await {
        Ok(()) => println!("Guest protocol initialized"),
        Err(error) => println!("Guest protocol: {error}"),
    }
    let playlists = client.discover().await?;
    println!("Discovery: {} playlists", playlists.len());
    let results = client.search("晴天", SearchKind::Songs, 0).await?;
    println!("Search: {} songs", results.songs.len());
    if let Some(song) = results.songs.first() {
        let details = client.songs(&[song.id]).await?;
        println!("Details: {} songs", details.len());
        let lyrics = client.lyrics(song.id).await?;
        println!("Lyrics: {} characters", lyrics.original.len());
        match client.stream(song.id, Quality::Exhigh).await {
            Ok(ticket) => println!("Stream: format={}, trial={}", ticket.format, ticket.trial),
            Err(error) => println!("Stream permission: {error}"),
        }
    }
    Ok(())
}
