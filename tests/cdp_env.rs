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
        let config = CdpStealthConfig::default();
        let initial = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        // SAFETY: this harness-free child has started no threads or runtime;
        // nothing else can be accessing the process environment.
        let args = unsafe { config.apply_env() };
        let disabled = std::env::var("SBRS_AUTOMATION").as_deref() == Ok("0");
        let driver = Driver::new(Null, "env-test");
        let reply = driver.handle("GET", "/env-test/cdp", None, b"");
        if disabled {
            assert!(args.is_empty());
            assert_eq!(
                std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default(),
                initial
            );
            assert_eq!(
                reply.body,
                json!({ "value": { "cdpEnabled": false, "cdpPort": null, "cdpUrl": null } })
            );
        } else {
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
        return;
    }
    for (gate, port) in [("1", "9333"), ("1", "invalid"), ("1", "0"), ("0", "9333")] {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("probe")
            .env("SBRS_AUTOMATION", gate)
            .env("SBRS_CDP_PORT", port)
            .env("SBRS_CDP_TEST_PORT", port)
            .env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--lang=fr-FR")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
