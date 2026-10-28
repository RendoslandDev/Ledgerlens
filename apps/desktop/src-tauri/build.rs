fn main() {
    let mut attributes = tauri_build::Attributes::new();
    if windows_msvc() {
        // tauri-build embeds the Common Controls v6 manifest in the app binary only, so the test
        // executables load the old comctl32 and fail to start (STATUS_ENTRYPOINT_NOT_FOUND).
        // Embed it through the linker instead, which covers every executable this crate links.
        attributes = attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("tauri-build failed");
}

/// The build script runs on the host; what matters is the target being linked.
fn windows_msvc() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
}
