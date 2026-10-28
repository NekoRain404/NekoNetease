use std::sync::Arc;
use std::time::Duration;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

fn runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()
}

#[test]
fn a_player_is_send_and_sync() {
    const _: fn() = || {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VideoPlayer>();
        assert_send_sync::<Arc<VideoPlayer>>();
    };
}

#[test]
fn the_hardware_decoding_mirror_moves_before_the_worker_does() {
    let player = VideoPlayer::spawn();
    assert!(player.hwdec_enabled(), "hardware decoding is the default");
    player.set_hwdec(false);
    assert!(!player.hwdec_enabled());
    player.set_hwdec(true);
    assert!(player.hwdec_enabled());
    player.shutdown();
}

#[test]
fn shutdown_is_idempotent_and_commands_after_it_are_closed() {
    let player = VideoPlayer::spawn();
    player.shutdown();
    player.shutdown();
    if player.available() {
        assert!(matches!(
            player.open("https://example.test/none.mp4", 0.5),
            Err(VideoError::Closed)
        ));
    }
}

#[test]
fn an_unopenable_url_reaches_failed_without_a_display() -> Result<(), Box<dyn std::error::Error>> {
    let player = VideoPlayer::spawn();
    if !player.available() {
        return Ok(());
    }
    assert!(player.open("file:///nonexistent/nekonetease-mv-test.mp4", 0.5).is_ok());
    let mut state = player.state();
    let failed = runtime()?.block_on(async {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if matches!(&*state.borrow_and_update(), VideoState::Failed(_)) {
                    return true;
                }
                if state.changed().await.is_err() {
                    return false;
                }
            }
        })
        .await
        .unwrap_or(false)
    });
    player.shutdown();
    assert!(failed, "mpv reports a missing file as Failed");
    Ok(())
}

#[test]
fn a_raw_video_file_yields_a_capped_rgba_frame() -> Result<(), Box<dyn std::error::Error>> {
    let player = VideoPlayer::spawn();
    if !player.available() {
        return Ok(());
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("frame.y4m");
    std::fs::write(&path, y4m(1280, 720, 40))?;

    let url = format!("file://{}", path.display());
    assert!(player.open(&url, 0.0).is_ok());

    let mut frames = player.frames();
    let frame = runtime()?.block_on(async {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if let Some(frame) = &*frames.borrow_and_update() {
                    return Some(Arc::clone(frame));
                }
                if frames.changed().await.is_err() {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    });
    player.shutdown();

    let frame = frame.ok_or("the render thread published no frame")?;
    assert!(frame.width <= 960 && frame.height <= 540, "{frame:?}");
    assert_eq!((frame.width, frame.height), (960, 540));
    assert_eq!(frame.pixels.len(), frame.byte_len());
    assert_eq!(frame.byte_len(), 960 * 540 * 4);
    Ok(())
}

fn y4m(width: u32, height: u32, frames: u32) -> Vec<u8> {
    let luma = width * height;
    let chroma = (width / 2) * (height / 2);
    let mut bytes = Vec::with_capacity(64 + frames as usize * (luma + 2 * chroma) as usize);
    bytes.extend_from_slice(
        format!("YUV4MPEG2 W{width} H{height} F25:1 Ip A1:1 C420jpeg\n").as_bytes(),
    );
    let mut picture = Vec::with_capacity((luma + 2 * chroma) as usize);
    picture.extend(std::iter::repeat_n(16_u8, luma as usize));
    picture.extend(std::iter::repeat_n(128_u8, 2 * chroma as usize));
    for _ in 0..frames {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&picture);
    }
    bytes
}
