//! Which directories a pass walks, and which window each entry point spends.
//!
//! The gates themselves are pinned a layer down in `nekonetease-artwork`, and the reference set a
//! layer sideways in `queries::artwork`. What only this layer can answer is what the two entry
//! points *spend* them on, and both failures are silent: a store dropped from `run`'s vec leaks
//! forever.

use std::path::Path;
use std::time::SystemTime;

use tempfile::TempDir;

use super::*;

/// Older than either window, so nothing but the reference set can save a file aged with it.
const PAST_EVERY_WINDOW: Duration = Duration::from_hours(2);

/// A library rooted in a temp dir, with both stores created — `collect_candidates` sweeps
/// nothing on a missing directory, so an assertion against one would pass without this.
struct Library {
    db: DbPool,
    paths: Paths,
    _tmp: TempDir,
}

impl Library {
    async fn new() -> Result<Self, AppError> {
        let tmp = TempDir::new()?;
        let paths = Paths::rooted_at(tmp.path().to_path_buf());
        paths.create_dirs()?;
        Ok(Self { db: DbPool::test_pool().await?, paths, _tmp: tmp })
    }
}

/// A stored file in `dir`, backdated by `age`. `sweep_stores` takes its own `SystemTime::now()`,
/// so the file's mtime is the only handle this layer has on either window.
///
/// The name is spelled to satisfy `is_stored_name` rather than asked: that predicate is
/// `pub(super)` in `nekonetease-artwork` and has its own suite there.
fn aged(dir: &Path, name: &str, age: Duration) -> Result<PathBuf, AppError> {
    let path = dir.join(name);
    std::fs::write(&path, b"stored artwork")?;
    std::fs::File::options().write(true).open(&path)?.set_modified(SystemTime::now() - age)?;
    Ok(path)
}

/// One store dropped from `run`'s vec leaks every orphan in it forever, and shows up nowhere:
/// the pass still returns `Ok` and still logs the other store.
#[tokio::test]
async fn a_pass_reaches_every_store_the_paths_name() -> Result<(), AppError> {
    let library = Library::new().await?;
    let orphans = [
        aged(&library.paths.artwork_dir, "33fb807d1f1b7cbb.jpg", PAST_EVERY_WINDOW)?,
        aged(&library.paths.artists_dir, "4cccaf4d4b4cea11.jpg", PAST_EVERY_WINDOW)?,
    ];

    run(&library.db, &library.paths).await?;

    for orphan in &orphans {
        assert!(!orphan.exists(), "{} survived a pass nothing referenced it in", orphan.display());
    }
    Ok(())
}

/// A track row keeps its artwork alive regardless of which current artwork store contains it.
#[tokio::test]
async fn artwork_named_by_a_track_is_still_held_by_its_row() -> Result<(), AppError> {
    let library = Library::new().await?;
    let artwork = aged(&library.paths.artwork_dir, "6ee0cf6f6d6e0c33.png", PAST_EVERY_WINDOW)?;
    let track_id = nekonetease_store::database::queries::fixtures::insert_test_track(
        &library.db,
        "/music/test-track.mp3",
        "Test Track",
        "Test Artist",
        "Test Album",
        "Rock",
    )
    .await?;
    sqlx::query("UPDATE tracks SET artwork_path = ? WHERE id = ?")
        .bind(artwork.to_string_lossy().as_ref())
        .bind(track_id)
        .execute(library.db.write())
        .await?;

    run(&library.db, &library.paths).await?;

    assert!(artwork.exists(), "a track's artwork must not be swept while its row names it");
    Ok(())
}

/// What a scan pass spared for being young is what brings the sweep back an hour later, and a
/// library with nothing left to scan runs no other.
#[tokio::test]
async fn a_pass_answers_how_many_orphans_the_window_spared() -> Result<(), AppError> {
    let library = Library::new().await?;
    aged(&library.paths.artwork_dir, "33fb807d1f1b7cbb.jpg", Duration::ZERO)?;
    aged(&library.paths.artists_dir, "4cccaf4d4b4cea11.jpg", PAST_EVERY_WINDOW)?;

    let deferred = run(&library.db, &library.paths).await?;

    assert_eq!(deferred, 1);
    Ok(())
}

/// The covers a folder removal released, named by the reference set read ahead of its delete.
fn released(names: &[&str]) -> Reach {
    Reach::Released(names.iter().map(|name| (*name).to_owned()).collect())
}

/// Every file this reaches was named by a committed row the delete removed, so the window could
/// only hold it back: a library removed minutes after its scan kept its covers for the hour.
#[tokio::test]
async fn a_folder_removal_retires_what_it_released_whatever_its_age() -> Result<(), AppError> {
    let library = Library::new().await?;
    let cover = aged(&library.paths.artwork_dir, "33fb807d1f1b7cbb.jpg", Duration::ZERO)?;

    sweep_stores(&library.db, all_stores(&library.paths), released(&["33fb807d1f1b7cbb.jpg"]))
        .await?;

    assert!(!cover.exists());
    Ok(())
}

/// The store is shared, so a cover the removed folder's tracks held may still be another row's.
#[tokio::test]
async fn a_folder_removal_keeps_a_released_file_another_row_still_names() -> Result<(), AppError> {
    let library = Library::new().await?;
    let shared = aged(&library.paths.artwork_dir, "33fb807d1f1b7cbb.jpg", PAST_EVERY_WINDOW)?;
    let track_id = nekonetease_store::database::queries::fixtures::insert_test_track(
        &library.db,
        "/music/shared-track.mp3",
        "Shared Track",
        "Test Artist",
        "Test Album",
        "Rock",
    )
    .await?;
    sqlx::query("UPDATE tracks SET artwork_path = ? WHERE id = ?")
        .bind(shared.to_string_lossy().as_ref())
        .bind(track_id)
        .execute(library.db.write())
        .await?;

    sweep_stores(&library.db, all_stores(&library.paths), released(&["33fb807d1f1b7cbb.jpg"]))
        .await?;

    assert!(shared.exists());
    Ok(())
}

/// Skipping the window is safe only for what the delete released. A young orphan beside it may
/// be a cover whose row hasn't committed yet.
#[tokio::test]
async fn a_folder_removal_leaves_an_orphan_it_did_not_release() -> Result<(), AppError> {
    let library = Library::new().await?;
    let unrelated = aged(&library.paths.artwork_dir, "4cccaf4d4b4cea11.jpg", Duration::ZERO)?;

    sweep_stores(&library.db, all_stores(&library.paths), released(&["33fb807d1f1b7cbb.jpg"]))
        .await?;

    assert!(unrelated.exists());
    Ok(())
}
