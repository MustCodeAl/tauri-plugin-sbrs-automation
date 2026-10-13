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
//! capability file. It is on in debug builds only, unless `SBRS_AUTOMATION=1`
//! turns it on (or `SBRS_AUTOMATION=0` off). It listens on `127.0.0.1` port
//! `SBRS_AUTOMATION_PORT` (default 4445), at a path that starts with a secret
//! token: `SBRS_AUTOMATION_TOKEN`, or a random one printed at start-up.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use serde_json::Value;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Runtime, State};

mod server;
pub mod webdriver;

use webdriver::{Bridge, Driver, WebDriverError};

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
            let local = server::spawn(addr, Driver::new(bridge, settings.token.clone()))?;
            eprintln!(
                "tauri-plugin-sbrs-automation: WebDriver at http://{local}/{}",
                settings.token
            );
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
    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let enabled = match var("SBRS_AUTOMATION").as_deref() {
            Some("0" | "false" | "off") => false,
            Some(_) => true,
            None => cfg!(debug_assertions),
        };
        Self {
            enabled,
            port: var("SBRS_AUTOMATION_PORT")
                .and_then(|port| port.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            token: var("SBRS_AUTOMATION_TOKEN").unwrap_or_else(random_token),
        }
    }
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

impl<R: Runtime> Bridge for TauriBridge<R> {
    fn call(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, WebDriverError> {
        let windows = self.app.webview_windows();
        let window = windows
            .get("main")
            .or_else(|| windows.values().next())
            .ok_or_else(|| WebDriverError::new("no such window", "the app has no window"))?;
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
}
