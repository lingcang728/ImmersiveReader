use super::*;
use crate::control::ControlDb;

#[test]
fn budget_rejection_never_copies_input() {
    let (root, locations, preview) = fixture("budget");
    let mut control =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let store = PodcastPreviewStore::default();
    store
        .insert(
            preview,
            PodcastPreviewOptions {
                translate: true,
                max_api_cost_cny: 0.0,
            },
        )
        .expect("preview must be stored");

    let error = super::super::add_podcast_files_at(
        &store,
        &mut control,
        &locations,
        &request(DuplicatePolicy::NewRevision, None, "request-budget"),
        |_| panic!("rejected request must not broadcast"),
    )
    .expect_err("budget must require confirmation");

    assert_eq!(error, "BUDGET_CONFIRMATION_REQUIRED");
    assert!(!locations.cache_root.exists());
    drop(control);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reuse_existing_returns_book_without_creating_task() {
    let (root, locations, mut preview) = fixture("duplicate");
    preview.files[0].duplicate_book_id = Some("existing-book".to_string());
    preview.budget.estimated_api_cost_upper_cny = 0.0;
    preview.budget.confirmation_required = false;
    let mut control =
        ControlDb::open(&root.join("control.db")).expect("control database must open");
    let store = PodcastPreviewStore::default();
    store
        .insert(
            preview,
            PodcastPreviewOptions {
                translate: false,
                max_api_cost_cny: 0.0,
            },
        )
        .expect("preview must be stored");

    let result = super::super::add_podcast_files_at(
        &store,
        &mut control,
        &locations,
        &request(DuplicatePolicy::ReuseExisting, None, "request-duplicate"),
        |_| panic!("reused book must not broadcast"),
    )
    .expect("existing book must be reused");

    assert!(result.tasks.is_empty());
    assert_eq!(result.existing_books, vec!["existing-book"]);
    assert!(!locations.cache_root.exists());
    drop(control);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
