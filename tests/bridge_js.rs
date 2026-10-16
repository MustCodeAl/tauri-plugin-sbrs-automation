//! Runs the tests of the page script, `tests/bridge.test.mjs`, with Node.js:
//! they load `src/bridge.js` into a fake page and check its own logic.

use std::process::Command;

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
