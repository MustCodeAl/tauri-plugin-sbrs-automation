//! Drive a Tauri app's webview from [seleniumbase-rs](https://github.com/MustCodeAl/seleniumbase-rs)
//! tests, on macOS, Windows and Linux.
//!
//! Tauri apps use the system webview. Windows (WebView2) and Linux (WebKitGTK)
//! have WebDriver drivers, but macOS (WKWebView) has none. This plugin runs a
//! small W3C WebDriver endpoint inside the app, so `BaseCase` and `sbrs` drive
//! the app's own window the same way everywhere.
//!
//! ```no_run
//! # fn main() {
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_sbrs_automation::init())
//! #   ;
//! # }
//! ```
//!
//! The plugin also needs the `sbrs-automation:default` permission in the app's
//! capability file. It is on in debug builds only, unless `SBRS_AUTOMATION`
//! says otherwise: `1`, `true` or `on` turn it on, any other value (`0`, or a
//! typo) turns it off. It listens on `127.0.0.1` port
//! `SBRS_AUTOMATION_PORT` (default 4445), at a path that starts with a secret
//! token: `SBRS_AUTOMATION_TOKEN`, or a random one printed at start-up. With
//! `SBRS_AUTOMATION_TRACE_WINDOW` set, the window commands print the window's
//! state on stderr, for finding out why a resize did not stick.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use serde_json::Value;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Runtime, State, WebviewWindow};

pub mod cdp;
mod native;
#[cfg(target_os = "macos")]
mod screenshot;
mod server;
mod trace;
pub mod webdriver;
mod window;

use native::Native;
use webdriver::{Bridge, Driver, ViewportRect, WebDriverError, WindowCommand, WindowRect};
use window::{RectRequest, SystemClock};

/// The default port of the WebDriver endpoint.
pub const DEFAULT_PORT: u16 = 4445;

/// Initializes the plugin. See the [crate] documentation for the settings.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    let settings = Settings::from_env();
    let mut builder = tauri::plugin::Builder::<R>::new("sbrs-automation")
        .invoke_handler(tauri::generate_handler![reply]);
    if !settings.enabled {
        return builder.build();
    }
    builder = builder
        .initialization_script(include_str!("bridge.js").to_owned())
        .setup(move |app, _api| {
            let pending = Arc::new(Pending::default());
            app.manage(Arc::clone(&pending));
            let bridge = TauriBridge {
                app: app.clone(),
                pending,
            };
            let addr = SocketAddr::from(([127, 0, 0, 1], settings.port));
            // A port in use is reported, not fatal: the app keeps running.
            let driver = Driver::new(bridge, settings.token.clone());
            // The debugging port is read once, here, from the launch settings
            // of this process. The environment is never asked, so a variable
            // inherited from a parent cannot name a port.
            let driver = match cdp::enabled_port() {
                Some(port) => driver.with_cdp_port(port),
                None => driver,
            };
            server::start(addr, driver, &settings.token);
            Ok(())
        });
    builder.build()
}

struct Settings {
    enabled: bool,
    port: u16,
    token: String,
}

impl Settings {
    fn automation_enabled() -> bool {
        automation_gate(
            std::env::var_os("SBRS_AUTOMATION").as_deref(),
            cfg!(debug_assertions),
        )
    }

    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            enabled: Self::automation_enabled(),
            port: var("SBRS_AUTOMATION_PORT")
                .and_then(|port| port.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            token: var("SBRS_AUTOMATION_TOKEN").unwrap_or_else(random_token),
        }
    }
}

/// Whether automation is on, given the value of `SBRS_AUTOMATION` (`None`
/// when unset) and whether this is a debug build. Unset follows the build
/// (on in debug builds only); `1`, `true` and `on` turn it on, in any case;
/// every other value turns it off.
fn automation_gate(value: Option<&std::ffi::OsStr>, debug_build: bool) -> bool {
    // Fail closed: a value that is set turns automation on only when it says
    // so. A typo (`ture`), an empty value or text that is not UTF-8 is off.
    value.map_or(debug_build, |value| {
        value.to_str().is_some_and(|value| {
            ["1", "true", "on"]
                .iter()
                .any(|on| value.eq_ignore_ascii_case(on))
        })
    })
}

/// 128 bits from the standard library's randomly seeded hasher keys: enough
/// for a loopback endpoint of a debug build.
fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    (0..2u8)
        .map(|part| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u8(part);
            hasher.write_u128(nanos);
            hasher.write_u32(std::process::id());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// The operations waiting for the page's answer.
#[derive(Default)]
struct Pending {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Result<Value, WebDriverError>>>>,
}

impl Pending {
    fn finish(&self, id: u64, ok: bool, value: Value) {
        let sender = self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        if let Some(sender) = sender {
            let result = if ok {
                Ok(value)
            } else {
                let field = |key: &str| {
                    value
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let code = field("error");
                Err(WebDriverError::new(
                    if code.is_empty() {
                        "javascript error"
                    } else {
                        &code
                    },
                    field("message"),
                ))
            };
            // The caller may have stopped waiting; then nobody needs the answer.
            let _ = sender.send(result);
        }
    }

    fn forget(&self, id: u64) {
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

/// The page's answer to one operation, sent by `bridge.js`.
#[tauri::command]
fn reply(pending: State<'_, Arc<Pending>>, id: u64, ok: bool, value: Value) {
    pending.finish(id, ok, value);
}

/// Runs operations in the app's `main` window (or its only one).
struct TauriBridge<R: Runtime> {
    app: AppHandle<R>,
    pending: Arc<Pending>,
}

impl<R: Runtime> TauriBridge<R> {
    /// The window the endpoint drives: `main`, or else any.
    fn driven_window(&self) -> Result<WebviewWindow<R>, WebDriverError> {
        let mut windows = self.app.webview_windows();
        windows
            .remove("main")
            .or_else(|| windows.into_values().next())
            .ok_or_else(|| WebDriverError::new("no such window", "the app has no window"))
    }
}

impl<R: Runtime> Bridge for TauriBridge<R> {
    fn call(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, WebDriverError> {
        let window = self.driven_window()?;
        let id = self.pending.next.fetch_add(1, Ordering::Relaxed);
        let (sender, answer) = mpsc::channel();
        self.pending
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, sender);
        // Both values are JSON literals, so they are valid JavaScript as they are.
        let script = format!(
            "window.__sbrs && window.__sbrs.run({id}, {}, {args});",
            Value::from(op)
        );
        if let Err(error) = window.eval(script.as_str()) {
            self.pending.forget(id);
            return Err(WebDriverError::new(
                "unknown error",
                format!("could not reach the webview: {error}"),
            ));
        }
        let result = answer.recv_timeout(timeout);
        self.pending.forget(id);
        result.unwrap_or_else(|_| {
            Err(WebDriverError::new(
                "timeout",
                format!("the page did not answer `{op}` in time"),
            ))
        })
    }

    fn window(&self, command: WindowCommand) -> Result<WindowRect, WebDriverError> {
        let window = self.driven_window()?;
        let native = Native::new(&window);
        let clock = SystemClock::new();
        // Each command waits until the window has changed before it answers
        // (see `window.rs`): Tauri only queues a change for the main thread.
        match command {
            WindowCommand::Rect => window::current_rect(&native),
            WindowCommand::SetRect {
                x,
                y,
                width,
                height,
            } => window::set_rect(
                &native,
                &clock,
                RectRequest {
                    x,
                    y,
                    width,
                    height,
                },
            ),
            WindowCommand::Maximize => window::maximize(&native, &clock),
            WindowCommand::Minimize => window::minimize(&native, &clock),
            WindowCommand::Fullscreen => window::fullscreen(&native, &clock),
        }
    }

    #[cfg(target_os = "macos")]
    fn screenshot(&self, area: Option<ViewportRect>) -> Result<String, WebDriverError> {
        screenshot::take(&self.driven_window()?, area)
    }

    #[cfg(not(target_os = "macos"))]
    fn screenshot(&self, area: Option<ViewportRect>) -> Result<String, WebDriverError> {
        let _ = area;
        Err(WebDriverError::new(
            "unsupported operation",
            "screenshots are taken on macOS only so far (WebView2 and WebKitGTK are not done)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::automation_gate;
    use std::ffi::OsStr;

    fn gate(value: &str, debug_build: bool) -> bool {
        automation_gate(Some(OsStr::new(value)), debug_build)
    }

    #[test]
    fn unset_follows_the_build_profile() {
        assert!(automation_gate(None, true));
        assert!(!automation_gate(None, false));
    }

    #[test]
    fn only_one_true_and_on_turn_it_on() {
        for value in ["1", "true", "on", "TRUE", "On"] {
            assert!(gate(value, false), "{value} in a release build");
            assert!(gate(value, true), "{value} in a debug build");
        }
    }

    #[test]
    fn anything_else_turns_it_off_even_in_a_debug_build() {
        for value in [
            "0", "false", "off", "OFF", "no", "yes", "enabled", "2", "maybe", "", " ", " 1", "1 ",
            "truee", "-1", "y",
        ] {
            assert!(!gate(value, true), "{value:?} in a debug build");
            assert!(!gate(value, false), "{value:?} in a release build");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_text_turns_it_off() {
        use std::os::unix::ffi::OsStrExt;
        let value = OsStr::from_bytes(&[0x31, 0xff]);
        assert!(!automation_gate(Some(value), true));
    }
}
