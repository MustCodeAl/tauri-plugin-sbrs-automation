//! Guards of `Cargo.toml`: the plugin stays light. The seleniumbase-rs library
//! (a git dependency of about 90 packages, with a newer minimum Rust version,
//! that blocks publishing on crates.io) comes only with the opt-in `stealth`
//! feature; a default build and the `cdp` feature have no part of it.

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn seleniumbase_rs_is_only_an_optional_dependency_of_the_stealth_feature() {
    let dependency = MANIFEST
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("seleniumbase-rs") && line.contains('='))
        .expect("the stealth feature's dependency");
    assert!(dependency.contains("optional = true"), "{dependency}");
    assert!(
        dependency.contains("default-features = false"),
        "{dependency}"
    );
    assert!(
        dependency.contains("rev = \""),
        "pinned to a commit: {dependency}"
    );
    let feature = MANIFEST
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("stealth ="));
    assert_eq!(feature, Some(r#"stealth = ["dep:seleniumbase-rs"]"#));
    assert!(
        MANIFEST.lines().any(|line| line.trim() == "default = []"),
        "a default build has neither feature"
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
