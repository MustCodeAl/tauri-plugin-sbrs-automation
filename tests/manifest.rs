//! Guards of `Cargo.toml`: the plugin stays light. The seleniumbase-rs library
//! (a git dependency of about 90 packages, with a newer minimum Rust version,
//! that blocks publishing on crates.io) comes only with the opt-in `stealth`
//! feature; a default build and the `cdp` feature have no part of it. The
//! example app takes the library at the same tag.

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
        dependency.contains("tag = \"v"),
        "pinned to a release tag: {dependency}"
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

/// The example app (`examples/tauri-profile-manager`) passes its own
/// seleniumbase-rs `Fingerprint` to the plugin's `StealthPlan`, which compiles
/// only while both take the library at the same tag: one copy, one set of
/// types. Moving the plugin's pin moves the example's with it.
#[test]
fn the_example_takes_seleniumbase_rs_at_the_plugins_tag() {
    const EXAMPLE: &str = include_str!("../examples/tauri-profile-manager/src-tauri/Cargo.toml");
    let tag = |manifest: &str| -> Vec<String> {
        manifest
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("seleniumbase-rs") && line.contains('='))
            .map(|line| {
                let start = line.find("tag = \"").expect("a tag") + "tag = \"".len();
                let end = start + line[start..].find('"').expect("the tag's end");
                line[start..end].to_owned()
            })
            .collect()
    };
    let plugin = tag(MANIFEST);
    let example = tag(EXAMPLE);
    assert_eq!(plugin.len(), 1, "{plugin:?}");
    assert_eq!(
        example.len(),
        2,
        "a dependency and a dev-dependency: {example:?}"
    );
    assert!(
        example.iter().all(|tag| *tag == plugin[0]),
        "{example:?} against {plugin:?}"
    );
    assert!(
        EXAMPLE.contains("tauri-plugin-sbrs-automation = { path = \"../../..\""),
        "the example builds against this checkout of the plugin"
    );
}
