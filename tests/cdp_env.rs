//! Environment mutation is tested only in fresh, single-threaded child processes.

use serde_json::{Value, json};
use std::ffi::OsStr;
use std::net::{Ipv4Addr, TcpListener};
use std::process::Command;
use std::time::Duration;
use tauri_plugin_sbrs_automation::cdp::{CdpStealthConfig, enabled_port};
use tauri_plugin_sbrs_automation::webdriver::{Bridge, Driver, WebDriverError};

#[path = "support/devtools.rs"]
mod devtools;

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
    let free = || devtools::free_port().to_string();
    // A port that is asked for, and free, is used.
    run(Some("1"), Some(&free()), fr, "on");
    run(Some("true"), Some(&free()), fr, "on");
    // No usable port asked for: a free one is picked.
    for port in ["invalid", "0"] {
        run(Some("1"), Some(port), fr, "auto");
    }
    run(Some("1"), None, fr, "auto");
    // A port that is taken enables nothing.
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let taken_port = taken.local_addr().unwrap().port().to_string();
    run(Some("1"), Some(&taken_port), fr, "taken");
    drop(taken);
    // A port in the environment is a request for the launch settings, not an
    // endpoint: an app that did not set Chromium up through the plugin, but
    // inherited the variable from its parent, advertises nothing.
    run(Some("1"), Some("9333"), fr, "inherited");
    // Without the variable the build profile decides, as in the plugin.
    run(
        None,
        None,
        fr,
        if cfg!(debug_assertions) {
            "auto"
        } else {
            "gated"
        },
    );
    // Automation off: no switch is produced, not even by `chromium_args`.
    // Only 1, true and on turn it on; a typo or an empty value does not.
    for gate in ["0", "off", "ture", "yes", ""] {
        run(Some(gate), Some(&free()), fr, "gated");
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
    let ask = || driver.handle("GET", "/env-test/cdp", None, b"").body;
    // Nothing leaves the debugger's port in the environment for child
    // processes to inherit: the variable is what the parent set, or unset.
    assert_eq!(std::env::var("SBRS_CDP_PORT").ok(), exported);
    // The launch settings answer to the automation gate, `apply_env` or not.
    let direct = if expect == "inherited" {
        Vec::new()
    } else {
        config.chromium_args()
    };
    // The port alone, for an app that configures CEF's remote debugging itself.
    let own = if expect == "inherited" {
        None
    } else {
        config.port()
    };

    if !matches!(expect.as_str(), "on" | "auto") {
        // `gated`: automation is off, so no switch is produced at all.
        // `taken`: the port asked for is in use, so nothing is enabled.
        // `refused`: the environment already holds a reserved switch, so
        // `apply_env` leaves it alone, though the arguments could be built.
        // `inherited`: SBRS_CDP_PORT is set, but nothing here enabled a port.
        assert_eq!(direct.is_empty(), expect != "refused", "{direct:?}");
        assert!(args.is_empty(), "{args:?}");
        assert_eq!(port, None);
        assert_eq!(own.is_some(), expect == "refused", "{own:?}");

        assert_eq!(
            std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default(),
            initial
        );
        assert_eq!(
            ask(),
            json!({ "value": {
                "cdpEnabled": false, "cdpPort": null, "cdpUrl": null, "cdpState": "disabled"
            } })
        );
        return;
    }
    // The port is the one asked for, or else a free one the plugin picked.
    let chosen = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--remote-debugging-port="))
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap();
    assert_ne!(chosen, 0);
    if expect == "on" {
        let asked: u16 = std::env::var("SBRS_CDP_TEST_PORT")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(chosen, asked);
    }
    for args in [&direct, &args] {
        assert!(args.contains(&format!("--remote-debugging-port={chosen}")));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
    }
    assert_eq!(port, Some(chosen));
    assert_eq!(own, Some(chosen));

    let appended = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap();
    assert!(appended.starts_with(&initial));
    assert!(appended.contains("--remote-debugging-address=127.0.0.1"));
    // Chromium has not started yet: nothing is advertised.
    assert_eq!(
        ask(),
        json!({ "value": {
            "cdpEnabled": false, "cdpPort": null, "cdpUrl": null, "cdpState": "unreachable"
        } })
    );
    // Once a DevTools endpoint answers on that port, it is.
    devtools::serve_devtools_on(chosen);
    assert_eq!(
        ask(),
        json!({ "value": {
            "cdpEnabled": true, "cdpPort": chosen,
            "cdpUrl": format!("http://127.0.0.1:{chosen}"), "cdpState": "ready"
        } })
    );
}
