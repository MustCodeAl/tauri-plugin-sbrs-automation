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
        .find(|line| line.starts_with("seleniumbase-rs =") && line.contains('='))
        .expect("the stealth feature's dependency");
    assert!(dependency.contains("optional = true"), "{dependency}");
    assert!(
        dependency.contains("default-features = false"),
        "{dependency}"
    );
    assert!(
        dependency.contains("git = \"https://github.com/MustCodeAl/seleniumbase-rs\""),
        "tracks seleniumbase-rs main via plain git URL: {dependency}"
    );
    let evasion_dep = MANIFEST
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("seleniumbase-rs-evasion") && line.contains('='))
        .expect("the stealth feature's evasion dependency");
    assert!(evasion_dep.contains("optional = true"), "{evasion_dep}");
    assert!(
        evasion_dep.contains("git = \"https://github.com/MustCodeAl/seleniumbase-rs-stealth\""),
        "tracks seleniumbase-rs-stealth main via plain git URL: {evasion_dep}"
    );
    let feature = MANIFEST
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("stealth ="));
    assert_eq!(
        feature,
        Some(r#"stealth = ["dep:seleniumbase-rs", "dep:seleniumbase-rs-evasion"]"#)
    );
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
        MANIFEST.contains("rust-version = \"1.100\""),
        "the plugin, the cdp feature included, needs the newest stable Rust, 1.100"
    );
}

/// The example app (`examples/tauri-profile-manager`) passes its own
/// seleniumbase-rs `Fingerprint` to the plugin's `StealthPlan`, which compiles
/// only while both resolve the library to the same commit: one copy, one set of
/// types. Moving the plugin's pin moves the example's with it.
#[test]
fn the_example_resolves_seleniumbase_rs_to_the_plugins_commit() {
    const EXAMPLE: &str = include_str!("../examples/tauri-profile-manager/src-tauri/Cargo.toml");
    let git_urls = |manifest: &str, name: &str| -> Vec<String> {
        let prefix = format!("{name} =");
        manifest
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with(&prefix))
            .map(|line| {
                let marker = "git = \"";
                let start = line.find(marker).expect("git url") + marker.len();
                let end = start + line[start..].find('"').expect("the url's end");
                line[start..end].to_owned()
            })
            .collect()
    };
    let core_url = "https://github.com/MustCodeAl/seleniumbase-rs";
    let stealth_url = "https://github.com/MustCodeAl/seleniumbase-rs-stealth";
    assert_eq!(git_urls(MANIFEST, "seleniumbase-rs"), [core_url]);
    assert_eq!(
        git_urls(EXAMPLE, "seleniumbase-rs"),
        [core_url, core_url],
        "a dependency and a dev-dependency"
    );
    assert_eq!(git_urls(MANIFEST, "seleniumbase-rs-evasion"), [stealth_url]);
    assert_eq!(git_urls(EXAMPLE, "seleniumbase-rs-evasion"), [stealth_url]);
    assert!(
        EXAMPLE.contains("tauri-plugin-sbrs-automation = { path = \"../../..\""),
        "the example builds against this checkout of the plugin"
    );

    const PLUGIN_LOCK: &str = include_str!("../Cargo.lock");
    const EXAMPLE_LOCK: &str =
        include_str!("../examples/tauri-profile-manager/src-tauri/Cargo.lock");
    assert_eq!(
        package_source(PLUGIN_LOCK, "seleniumbase-rs"),
        package_source(EXAMPLE_LOCK, "seleniumbase-rs"),
        "the plugin and the example must resolve seleniumbase-rs to one commit"
    );
    assert_eq!(
        package_source(PLUGIN_LOCK, "seleniumbase-rs-evasion"),
        package_source(EXAMPLE_LOCK, "seleniumbase-rs-evasion"),
        "the plugin and the example must resolve seleniumbase-rs-evasion to one commit"
    );
}

/// The `source` line of the named package in a lockfile.
fn package_source(lock: &str, name: &str) -> String {
    let lines: Vec<&str> = lock.lines().collect();
    let target = format!("name = \"{name}\"");
    let index = lines
        .iter()
        .position(|line| *line == target)
        .unwrap_or_else(|| panic!("{name} in the lockfile"));
    lines[index..]
        .iter()
        .take(6)
        .find_map(|line| line.strip_prefix("source = ").map(ToOwned::to_owned))
        .unwrap_or_else(|| panic!("a git source for {name}"))
}
