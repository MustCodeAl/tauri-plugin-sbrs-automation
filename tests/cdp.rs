use serde_json::{Value, json};
use std::time::Duration;
use tauri_plugin_sbrs_automation::webdriver::{Bridge, Driver, WebDriverError};

struct Null;

impl Bridge for Null {
    fn call(&self, _op: &str, _args: Value, _timeout: Duration) -> Result<Value, WebDriverError> {
        Ok(Value::Null)
    }
}

#[test]
fn cdp_discovery_uses_the_existing_authentication_guards() {
    let driver = Driver::new(Null, "cdp-test");
    let discovery = driver.handle("GET", "/cdp-test/cdp", None, b"");
    assert_eq!(discovery.status, 200);
    assert!(discovery.body["value"]["cdpEnabled"].is_boolean());
    for path in ["/cdp", "/cdp-testX/cdp"] {
        assert_eq!(driver.handle("GET", path, None, b"").status, 404);
    }
    for origin in ["https://example.com", "null", ""] {
        assert_eq!(
            driver
                .handle("GET", "/cdp-test/cdp", Some(origin), b"")
                .status,
            403
        );
    }
    assert_eq!(
        driver.handle("GET", "/cdp-test/status", None, b"").body,
        json!({ "value": { "ready": true, "message": "tauri-plugin-sbrs-automation is ready" } })
    );
}

#[cfg(feature = "cdp")]
#[test]
fn chromium_configuration_is_available_to_an_app() {
    let args = tauri_plugin_sbrs_automation::cdp::CdpStealthConfig::default()
        .debugging_port(9333)
        .chromium_args();
    assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
    assert!(args.contains(&"--remote-debugging-port=9333".to_owned()));
    for arg in seleniumbase_rs::stealth::engine_spoofing_args() {
        assert!(args.contains(&arg), "missing canonical argument: {arg}");
    }
}
