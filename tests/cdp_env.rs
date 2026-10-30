//! Environment mutation is tested only in fresh, single-threaded child processes.

use serde_json::{Value, json};
use std::ffi::OsStr;
use std::process::Command;
use std::time::Duration;
use tauri_plugin_sbrs_automation::cdp::{CdpStealthConfig, enabled_port};
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
    let run = |gate: Option<&str>, port: Option<&str>, webview2: &OsStr, expect: &str| {
        let mut command = Command::new(&exe);
        command
            .arg("probe")
            .env("SBRS_CDP_TEST_EXPECT", expect)
            .env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", webview2)
            .env_remove("SBRS_AUTOMATION")
            .env_remove("SBRS_CDP_PORT")
            .env_remove("SBRS_CDP_TEST_PORT");
        if let Some(gate) = gate {
            command.env("SBRS_AUTOMATION", gate);
        }
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
    let fr = OsStr::new("--lang=fr-FR");
    for port in ["9333", "invalid", "0"] {
        run(Some("1"), Some(port), fr, "on");
    }
    run(Some("true"), Some("9333"), fr, "on");
    // A port in the environment is a request for the launch settings, not an
    // endpoint: an app that did not set Chromium up through the plugin, but
    // inherited the variable from its parent, advertises nothing.
    run(Some("1"), Some("9333"), fr, "inherited");

    // Without the variable the build profile decides, as in the plugin.
    run(
        None,
        Some("9333"),
        fr,
        if cfg!(debug_assertions) {
            "on"
        } else {
            "gated"
        },
    );
    // Automation off: no switch is produced, not even by `chromium_args`.
    // Only 1, true and on turn it on; a typo or an empty value does not.
    for gate in ["0", "off", "ture", "yes", ""] {
        run(Some(gate), Some("9333"), fr, "gated");
    }
    // Text that is not UTF-8 is not a reason to skip the check.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let existing = OsStr::from_bytes(b"--remote-debugging-port=1234 \xff");
        run(Some("1"), None, existing, "refused");
    }
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
        run(Some("1"), None, OsStr::new(existing), "refused");
    }
}

fn probe() {
    let expect = std::env::var("SBRS_CDP_TEST_EXPECT").unwrap();
    let exported = std::env::var("SBRS_CDP_PORT").ok();
    let initial = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    let config = CdpStealthConfig::default();
    let args = if expect == "inherited" {
        // An app that never set up Chromium through the plugin.
        Vec::new()
    } else {
        // SAFETY: this harness-free child has started no threads or runtime;
        // nothing else can be accessing the process environment.
        unsafe { config.apply_env() }
    };
    // What the plugin does when it starts: read the port once from the launch
    // settings, never from the environment.
    let port = enabled_port();
    let driver = Driver::new(Null, "env-test");
    let driver = match port {
        Some(port) => driver.with_cdp_port(port),
        None => driver,
    };
    let reply = driver.handle("GET", "/env-test/cdp", None, b"");
    // Nothing leaves the debugger's port in the environment for child
    // processes to inherit: the variable is what the parent set, or unset.
    assert_eq!(std::env::var("SBRS_CDP_PORT").ok(), exported);
    // The launch settings answer to the automation gate, `apply_env` or not.
    let direct = if expect == "inherited" {
        Vec::new()
    } else {
        config.chromium_args()
    };
    if expect != "on" {
        // `gated`: automation is off, so no switch is produced at all.
        // `refused`: the environment already holds a reserved switch, so
        // `apply_env` leaves it alone, though the arguments could be built.
        // `inherited`: SBRS_CDP_PORT is set, but nothing here enabled a port.
        assert_eq!(direct.is_empty(), expect != "refused", "{direct:?}");
        assert!(args.is_empty(), "{args:?}");
        assert_eq!(port, None);
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
    for args in [&direct, &args] {
        assert!(args.contains(&format!("--remote-debugging-port={expected}")));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
    }
    assert_eq!(port, Some(expected));
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
