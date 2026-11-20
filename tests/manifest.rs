//! Guards of `Cargo.toml`: the plugin stays light. The seleniumbase-rs library
//! (a git dependency of about 90 packages, with a newer minimum Rust version,
//! that blocks publishing on crates.io) comes only with the opt-in `stealth`
//! feature; a default build and the `cdp` feature have no part of it. The
//! example app resolves the library to the same commit.

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
        dependency.contains("branch = \"main\""),
        "tracks seleniumbase-rs main: {dependency}"
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
fn the_minimum_rust_version_is_the_newest_stable() {
    assert!(
        MANIFEST.contains("rust-version = \"1.99\""),
        "the plugin, the cdp feature included, needs the newest stable Rust, 1.99"
    );
}

/// The example app (`examples/tauri-profile-manager`) passes its own
/// seleniumbase-rs `Fingerprint` to the plugin's `StealthPlan`, which compiles
/// only while both resolve the library to the same commit: one copy, one set of
/// types. Moving the plugin's pin moves the example's with it.
#[test]
fn the_example_resolves_seleniumbase_rs_to_the_plugins_commit() {
    const EXAMPLE: &str = include_str!("../examples/tauri-profile-manager/src-tauri/Cargo.toml");
    let branch = |manifest: &str| -> Vec<String> {
        manifest
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("seleniumbase-rs") && line.contains('='))
            .map(|line| {
                let marker = "branch = \"";
                let start = line.find(marker).expect("branch = \"main\"") + marker.len();
                let end = start + line[start..].find('"').expect("the branch's end");
                line[start..end].to_owned()
            })
            .collect()
    };
    let plugin = branch(MANIFEST);
    let example = branch(EXAMPLE);
    assert_eq!(plugin, ["main".to_owned()]);
    assert_eq!(
        example,
        ["main".to_owned(), "main".to_owned()],
        "a dependency and a dev-dependency"
    );
    assert!(
        EXAMPLE.contains("tauri-plugin-sbrs-automation = { path = \"../../..\""),
        "the example builds against this checkout of the plugin"
    );

    const PLUGIN_LOCK: &str = include_str!("../Cargo.lock");
    const EXAMPLE_LOCK: &str =
        include_str!("../examples/tauri-profile-manager/src-tauri/Cargo.lock");
    assert_eq!(
        seleniumbase_rs_source(PLUGIN_LOCK),
        seleniumbase_rs_source(EXAMPLE_LOCK),
        "the plugin and the example must resolve seleniumbase-rs to one commit"
    );
}

/// The `source` line of the `seleniumbase-rs` package in a lockfile.
fn seleniumbase_rs_source(lock: &str) -> String {
    let lines: Vec<&str> = lock.lines().collect();
    let index = lines
        .iter()
        .position(|line| *line == "name = \"seleniumbase-rs\"")
        .expect("seleniumbase-rs in the lockfile");
    lines[index..]
        .iter()
        .take(6)
        .find_map(|line| line.strip_prefix("source = ").map(ToOwned::to_owned))
        .expect("a git source for seleniumbase-rs")
}
