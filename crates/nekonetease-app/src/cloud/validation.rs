//! Input bounds checked before a typed request reaches the network.

use nekonetease_core::error::{AppError, AppResult};

const MAX_PLAYLIST_NAME_CHARS: usize = 40;
/// The server clears the description when this is empty, so zero is a valid length.
const MAX_PLAYLIST_DESC_CHARS: usize = 1000;
const MAX_PLAYLIST_TAGS: usize = 10;
const MAX_PLAYLIST_TAG_CHARS: usize = 20;
/// One manipulate request's ids. A larger edit is refused rather than split, because a reorder
/// only means anything to the server as one batch.
const MAX_PLAYLIST_TRACK_BATCH: usize = 1000;

pub(super) fn validate_id(label: &str, id: i64) -> AppResult<()> {
    if id <= 0 { Err(AppError::Validation(format!("{label} ID 无效"))) } else { Ok(()) }
}

pub(super) fn validate_ids(label: &str, ids: &[i64]) -> AppResult<()> {
    if ids.iter().any(|id| *id <= 0) {
        Err(AppError::Validation(format!("{label} ID 无效")))
    } else {
        Ok(())
    }
}

pub(super) fn validate_playlist_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("歌单名称不能为空".to_owned()));
    }
    if name.chars().count() > MAX_PLAYLIST_NAME_CHARS {
        return Err(AppError::Validation(format!(
            "歌单名称不能超过 {MAX_PLAYLIST_NAME_CHARS} 个字符"
        )));
    }
    Ok(name)
}

pub(super) fn validate_playlist_description(desc: &str) -> AppResult<()> {
    if desc.chars().count() > MAX_PLAYLIST_DESC_CHARS {
        return Err(AppError::Validation(format!(
            "歌单简介不能超过 {MAX_PLAYLIST_DESC_CHARS} 个字符"
        )));
    }
    Ok(())
}

pub(super) fn validate_playlist_tags(tags: &[String]) -> AppResult<Vec<String>> {
    if tags.len() > MAX_PLAYLIST_TAGS {
        return Err(AppError::Validation(format!("歌单标签最多 {MAX_PLAYLIST_TAGS} 个")));
    }
    tags.iter()
        .map(|tag| {
            let tag = tag.trim();
            if tag.is_empty() {
                return Err(AppError::Validation("歌单标签不能为空".to_owned()));
            }
            if tag.contains(',') {
                // The wire joins tags with commas and the server splits on them, so a tag
                // containing one would silently become two.
                return Err(AppError::Validation("歌单标签不能包含英文逗号".to_owned()));
            }
            if tag.chars().count() > MAX_PLAYLIST_TAG_CHARS {
                return Err(AppError::Validation(format!(
                    "歌单标签不能超过 {MAX_PLAYLIST_TAG_CHARS} 个字符"
                )));
            }
            Ok(tag.to_owned())
        })
        .collect()
}

pub(super) fn validate_track_batch(track_ids: &[i64]) -> AppResult<()> {
    if track_ids.is_empty() {
        return Err(AppError::Validation("请选择要操作的歌曲".to_owned()));
    }
    if track_ids.len() > MAX_PLAYLIST_TRACK_BATCH {
        return Err(AppError::Validation(format!(
            "单次最多操作 {MAX_PLAYLIST_TRACK_BATCH} 首歌曲"
        )));
    }
    validate_ids("歌曲", track_ids)
}

pub(super) fn validate_discovery_category(category: &str) -> AppResult<&str> {
    let category = category.trim();
    if category.is_empty() {
        return Err(AppError::Validation("歌单分类不能为空".to_owned()));
    }
    if category.chars().count() > 100 {
        return Err(AppError::Validation("歌单分类不能超过 100 个字符".to_owned()));
    }
    Ok(category)
}

pub(super) fn validate_nonempty(label: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        Err(AppError::Validation(format!("{label}不能为空")))
    } else {
        Ok(())
    }
}
