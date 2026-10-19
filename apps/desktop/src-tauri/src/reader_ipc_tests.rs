use super::*;

#[test]
fn actual_tauri_dispatch_round_trips_locator_and_bookmarks() {
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            save_reader_locator,
            load_reader_locator,
            add_reader_bookmark,
            list_reader_bookmarks,
            remove_reader_bookmark
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock window");
    let call = |cmd: &str, body: serde_json::Value| {
        tauri::test::get_ipc_response(
            &window,
            tauri::webview::InvokeRequest {
                cmd: cmd.to_string(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().expect("url"),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .map(|body| {
            body.deserialize::<serde_json::Value>()
                .expect("response JSON")
        })
    };
    let id = format!("ipc-test-{}", uuid::Uuid::new_v4());
    let locator = serde_json::json!({"schemaVersion": 1, "bookId": id, "chapterId": "chapter2", "anchor": {"kind": "text", "quote": "继续阅读", "offset": 120}, "ratio": 0.57, "updated": "2026-10-18T13:00:00Z"});
    let args = serde_json::json!({"bookId": id, "locatorJson": locator.to_string()});
    call("save_reader_locator", args.clone()).expect("save through real invoke handler");
    let loaded = call("load_reader_locator", serde_json::json!({"bookId": id})).expect("load");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(loaded.as_str().expect("JSON string"))
            .expect("locator"),
        locator
    );
    let bookmark = call(
        "add_reader_bookmark",
        serde_json::json!({"bookId": id, "locatorJson": locator.to_string(), "label": "这里"}),
    )
    .expect("add");
    let bookmarks = call("list_reader_bookmarks", serde_json::json!({"bookId": id})).expect("list");
    assert_eq!(bookmarks.as_array().expect("rows").len(), 1);
    assert_eq!(bookmarks[0]["locatorJson"], locator.to_string());
    call(
        "remove_reader_bookmark",
        serde_json::json!({"bookId": id, "bookmarkId": bookmark["bookmarkId"]}),
    )
    .expect("remove");
    assert!(call(
        "save_reader_locator",
        serde_json::json!({"bookId": id, "locator": locator})
    )
    .is_err());
    assert!(call("get_reader_locator", serde_json::json!({"bookId": id})).is_err());
    reader_db::ReaderDb::open_current()
        .expect("qa db")
        .remove_book(&id)
        .expect("cleanup");
}
