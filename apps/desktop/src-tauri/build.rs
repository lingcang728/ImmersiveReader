fn main() {
    // P2-20: embed the custom application manifest — tauri-build's default
    // plus `<longPathAware>` (see app.manifest). include_str! keeps the file
    // on cargo's dep-info list so edits retrigger the resource build.
    let windows_attributes =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));
    let attributes = tauri_build::Attributes::new().windows_attributes(windows_attributes);
    tauri_build::try_build(attributes).expect("failed to run tauri build script");
}
