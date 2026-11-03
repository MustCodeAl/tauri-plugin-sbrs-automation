fn main() {
    // `capabilities/e2e/` grants the permission of the plugin's WebDriver
    // endpoint, which only the `e2e` feature registers (`run` in `lib.rs`), so
    // the folder is read only with the feature: other builds do not grant
    // the permission at all.
    let capabilities = if std::env::var_os("CARGO_FEATURE_E2E").is_some() {
        "./capabilities/**/*"
    } else {
        "./capabilities/*.json"
    };
    // A custom pattern makes watching the folder our job.
    println!("cargo:rerun-if-changed=capabilities");
    let attributes = tauri_build::Attributes::new().capabilities_path_pattern(capabilities);
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("{error:#}");
    }
}
