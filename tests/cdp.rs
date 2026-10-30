use serde_json::{Value, json};
use std::time::Duration;
use tauri_plugin_sbrs_automation::webdriver::{Bridge, Driver, WebDriverError};

#[path = "support/devtools.rs"]
mod devtools;

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
    for path in ["/cdp", "/cdp-testX/cdp", "/cdp-tes/cdp", "//cdp-test/cdp"] {
        assert_eq!(driver.handle("GET", path, None, b"").status, 404, "{path}");
    }
    for origin in ["https://example.com", "null", ""] {
        let refused = driver.handle("GET", "/cdp-test/cdp", Some(origin), b"");
        assert_eq!(refused.status, 403, "Origin {origin:?}");
        // A web page learns nothing about the debugger.
        assert!(refused.body["value"]["cdpPort"].is_null());
        assert!(refused.body["value"]["cdpUrl"].is_null());
    }
    assert_eq!(
        driver.handle("GET", "/cdp-test/status", None, b"").body,
        json!({ "value": { "ready": true, "message": "tauri-plugin-sbrs-automation is ready" } })
    );
}

#[test]
fn discovery_says_off_when_the_launch_settings_enabled_no_port() {
    let driver = Driver::new(Null, "cdp-test");
    let reply = driver.handle("GET", "/cdp-test/cdp", None, b"");
    assert_eq!(
        reply.body,
        json!({ "value": {
            "cdpEnabled": false, "cdpPort": null, "cdpUrl": null, "cdpState": "disabled"
        } })
    );
}

#[test]
fn discovery_advertises_the_port_only_while_a_debugger_answers_there() {
    let port = devtools::serve_devtools();
    let driver = Driver::new(Null, "cdp-test").with_cdp_port(port);
    assert_eq!(
        driver.handle("GET", "/cdp-test/cdp", None, b"").body,
        json!({ "value": {
            "cdpEnabled": true, "cdpPort": port,
            "cdpUrl": format!("http://127.0.0.1:{port}"), "cdpState": "ready"
        } })
    );
    // Nothing answers on this port, as before Chromium has started, or when
    // another process took the port and this app's debugger never bound it.
    let driver = Driver::new(Null, "cdp-test").with_cdp_port(devtools::free_port());
    assert_eq!(
        driver.handle("GET", "/cdp-test/cdp", None, b"").body,
        json!({ "value": {
            "cdpEnabled": false, "cdpPort": null, "cdpUrl": null, "cdpState": "unreachable"
        } })
    );
}

#[test]
fn discovery_does_not_follow_a_listener_that_is_not_chromium() {
    let port = devtools::serve_raw(|_| devtools::http("HTTP/1.1 200 OK", r#"{"status":"ok"}"#));
    let driver = Driver::new(Null, "cdp-test").with_cdp_port(port);
    let reply = driver.handle("GET", "/cdp-test/cdp", None, b"");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["value"]["cdpEnabled"], false);
    assert_eq!(reply.body["value"]["cdpState"], "unreachable");
    assert!(reply.body["value"]["cdpUrl"].is_null());
}

#[cfg(feature = "cdp")]
#[test]
fn chromium_configuration_is_available_to_an_app() {
    // The plugin's gate decides (see `tests/cdp_env.rs`, which sets it in a
    // child process): skip when the environment of this run changes it.
    if std::env::var_os("SBRS_AUTOMATION").is_some() || !cfg!(debug_assertions) {
        return;
    }

    let args = tauri_plugin_sbrs_automation::cdp::CdpStealthConfig::default()
        .debugging_port(9333)
        .stealth_args(true)
        .chromium_args();
    assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
    assert!(args.contains(&"--remote-debugging-port=9333".to_owned()));
    assert!(args.contains(&"--disable-blink-features=AutomationControlled".to_owned()));
    // Site isolation of the app's own browser stays on.
    assert!(!args.iter().any(|arg| arg.contains("site-per-process")));
    assert!(!args.iter().any(|arg| arg.starts_with("--disable-features")));
}
