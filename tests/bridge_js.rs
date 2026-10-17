//! Runs the tests of the page script, `tests/bridge.test.mjs`, with Node.js:
//! they load `src/bridge.js` into a fake page and check its own logic.

use sha2::{Digest, Sha256};
use std::process::Command;

/// `src/bridge.js` is generated from `src/bridge.ts`; its first line holds the
/// SHA-256 of the source it was built from.
#[test]
fn the_generated_page_script_matches_its_typescript_source() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let source = std::fs::read(format!("{dir}/src/bridge.ts")).expect("src/bridge.ts exists");
    let generated =
        std::fs::read_to_string(format!("{dir}/src/bridge.js")).expect("src/bridge.js exists");
    let expected = format!("{:x}", Sha256::digest(&source));
    let header = generated.lines().next().unwrap_or_default();
    let recorded = header
        .split("(sha256: ")
        .nth(1)
        .and_then(|rest| rest.split(')').next());
    assert_eq!(
        recorded,
        Some(expected.as_str()),
        "src/bridge.js is out of date with src/bridge.ts: run `bun run bridge` and commit the result"
    );
}

#[test]
fn the_page_script_passes_its_tests() {
    let tests = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/bridge.test.mjs");
    let output = Command::new("node")
        .arg("--test")
        .arg(tests)
        .output()
        .expect("Node.js (18 or later) on PATH runs the tests of src/bridge.js");
    assert!(
        output.status.success(),
        "the tests of src/bridge.js failed ({}):\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
