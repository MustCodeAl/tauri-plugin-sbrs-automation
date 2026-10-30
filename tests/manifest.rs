//! Guards of `Cargo.toml`: the plugin stays light, with no dependency on the
//! seleniumbase-rs library (a git dependency of about 90 packages that would
//! raise the minimum Rust version and block publishing on crates.io).

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn the_plugin_does_not_depend_on_seleniumbase_rs() {
    let dependency = MANIFEST
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("seleniumbase-rs") && line.contains('='));
    assert_eq!(
        dependency, None,
        "seleniumbase-rs must not be a dependency of the plugin"
    );
}

#[test]
fn the_cdp_feature_has_no_dependencies() {
    let feature = MANIFEST
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("cdp ="));
    assert_eq!(feature, Some("cdp = []"));
}

#[test]
fn the_minimum_rust_version_is_not_raised_for_cdp() {
    assert!(
        MANIFEST.contains("rust-version = \"1.88\""),
        "the cdp feature must build with the plugin's own Rust version"
    );
}
