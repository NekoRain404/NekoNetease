use nekonetease_core::error::AppError;

use super::{MAX_PAGE_LIMIT, page_limit};

#[test]
fn page_limit_accepts_the_first_and_last_bounded_page() {
    assert_eq!(page_limit(1).ok(), Some(1));
    assert_eq!(page_limit(MAX_PAGE_LIMIT).ok(), Some(MAX_PAGE_LIMIT));
    assert_eq!(page_limit(30).ok(), Some(30));
}

#[test]
fn page_limit_refuses_zero_and_a_page_above_the_cap() {
    assert!(matches!(page_limit(0), Err(AppError::Validation(_))));
    assert!(matches!(page_limit(MAX_PAGE_LIMIT + 1), Err(AppError::Validation(_))));
}
