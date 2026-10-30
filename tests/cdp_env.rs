//! Environment mutation is tested only in fresh, single-threaded child processes.

use serde_json::{Value, json};
use std::process::Command;
use std::time::Duration;
use tauri_plugin_sbrs_automation::cdp::CdpStealthConfig;
use tauri_plugin_sbrs_automation::webdriver::{Bridge, Driver, WebDriverError};

struct Null;
impl Bridge for Null {
    fn call(&self, _op: &str, _args: Value, _timeout: Duration) -> Result<Value, WebDriverError> {
        Ok(Value::Null)
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("probe") {
        probe();
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let run = |gate: &str, port: Option<&str>, webview2: &str, expect: &str| {
        let mut command = Command::new(&exe);
        command
            .arg("probe")
            .env("SBRS_AUTOMATION", gate)
            .env("SBRS_CDP_TEST_EXPECT", expect)
            .env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", webview2)
            .env_remove("SBRS_CDP_PORT")
            .env_remove("SBRS_CDP_TEST_PORT");
        if let Some(port) = port {
            command
                .env("SBRS_CDP_PORT", port)
                .env("SBRS_CDP_TEST_PORT", port);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "gate {gate:?}, port {port:?}, WebView2 {webview2:?}, expecting {expect}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    for port in ["9333", "invalid", "0"] {
        run("1", Some(port), "--lang=fr-FR", "on");
    }
    run("0", Some("9333"), "--lang=fr-FR", "off");
    // An environment that already carries a reserved switch, in any spelling,
    // is refused: the plugin cannot say which endpoint would be the app's.
    for existing in [
        "--remote-debugging-port=1234",
        "--REMOTE-DEBUGGING-PIPE",
        "/remote-debugging-address=0.0.0.0",
        "--remote-allow-origins=*",
        "--lang=fr-FR --Remote-Allow-Origins=*",
        "--remote-debugging-\"port\"=1234",
        "--edge-webview-switches=--remote-debugging-port=1",
    ] {
        run("1", None, existing, "off");
    }
}

fn probe() {
    let config = CdpStealthConfig::default();
    let initial = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    // SAFETY: this harness-free child has started no threads or runtime;
    // nothing else can be accessing the process environment.
    let args = unsafe { config.apply_env() };
    let driver = Driver::new(Null, "env-test");
    let reply = driver.handle("GET", "/env-test/cdp", None, b"");
    if std::env::var("SBRS_CDP_TEST_EXPECT").as_deref() == Ok("off") {
        assert!(args.is_empty(), "{args:?}");
        assert_eq!(
            std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default(),
            initial
        );
        assert_eq!(
            reply.body,
            json!({ "value": { "cdpEnabled": false, "cdpPort": null, "cdpUrl": null } })
        );
        return;
    }
    let expected = if std::env::var("SBRS_CDP_TEST_PORT").as_deref() == Ok("9333") {
        9333
    } else {
        9222
    };
    assert!(args.contains(&format!("--remote-debugging-port={expected}")));
    let appended = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap();
    assert!(appended.starts_with(&initial));
    assert!(appended.contains("--remote-debugging-address=127.0.0.1"));
    assert_eq!(
        reply.body,
        json!({ "value": {
            "cdpEnabled": true, "cdpPort": expected, "cdpUrl": format!("http://127.0.0.1:{expected}")
        } })
    );
}
