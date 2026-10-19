fn main() {
    // P2-20: embed the custom application manifest — tauri-build's default
    // plus `<longPathAware>` (see app.manifest). include_str! keeps the file
    // on cargo's dep-info list so edits retrigger the resource build.
    let windows_attributes = tauri_build::WindowsAttributes::new_without_app_manifest();
    let attributes = tauri_build::Attributes::new().windows_attributes(windows_attributes);
    tauri_build::try_build(attributes).expect("failed to run tauri build script");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Tauri's resource manifest applies only to app binaries. Native IPC
        // tests also link TaskDialogIndirect and need Common Controls v6.
        let manifest =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"))
                .join("app.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
