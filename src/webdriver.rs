//! The W3C WebDriver commands the plugin answers, independent of Tauri.
//!
//! [`Driver`] turns an HTTP request into one or more [`Bridge`] operations,
//! which the injected script (`bridge.js`) runs in the webview, and the result
//! into a WebDriver response. Only part of WebDriver is served; anything else
//! answers `unknown command` or `unsupported operation`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod actions;

/// The key WebDriver uses for an element reference (the W3C "web element
/// identifier").
pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// How long an operation other than a script may take in the page.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a lookup waits before it looks again, under an implicit wait.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// Runs one operation of the injected script in the webview, or asks the
/// app's window.
pub trait Bridge: Send + Sync {
    /// Runs `op` with `args` and returns its value, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// The WebDriver error the script reported, or `timeout`.
    fn call(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, WebDriverError>;

    /// Applies `command` to the app's window and returns the window's
    /// rectangle afterwards. The page cannot answer this: WKWebView reports
    /// `outerWidth` and `outerHeight` as 0.
    ///
    /// # Errors
    ///
    /// `no such window`, or what the window reported. The default answers
    /// `unsupported operation`.
    fn window(&self, command: WindowCommand) -> Result<WindowRect, WebDriverError> {
        let _ = command;
        Err(WebDriverError::new(
            "unsupported operation",
            "this bridge has no window",
        ))
    }

    /// Takes a screenshot of the webview, of `area` of it when given, and
    /// returns it as a base64-encoded PNG.
    ///
    /// # Errors
    ///
    /// What the webview reported. The default answers `unsupported
    /// operation`.
    fn screenshot(&self, area: Option<ViewportRect>) -> Result<String, WebDriverError> {
        let _ = area;
        Err(WebDriverError::new(
            "unsupported operation",
            "this bridge takes no screenshots",
        ))
    }
}

/// A rectangle of the webview in CSS pixels, from the top left corner of its
/// viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
    /// The width.
    pub width: f64,
    /// The height.
    pub height: f64,
}

/// What a window command asks of the app's window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowCommand {
    /// Only report the rectangle.
    Rect,
    /// Move and resize the window, in CSS pixels; `None` keeps that value.
    SetRect {
        /// The left edge of the window.
        x: Option<f64>,
        /// The top edge of the window.
        y: Option<f64>,
        /// The outer width of the window.
        width: Option<f64>,
        /// The outer height of the window.
        height: Option<f64>,
    },
    /// Maximize the window.
    Maximize,
    /// Minimize the window.
    Minimize,
    /// Make the window full screen.
    Fullscreen,
}

/// The app window's outer rectangle in CSS pixels, as WebDriver reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRect {
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
    /// The outer width.
    pub width: f64,
    /// The outer height.
    pub height: f64,
}

/// A WebDriver error: its code (such as `no such element`) and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebDriverError {
    /// The W3C error code.
    pub error: String,
    /// What went wrong.
    pub message: String,
}

impl WebDriverError {
    /// An error with the W3C code `error`.
    pub fn new(error: &str, message: impl Into<String>) -> Self {
        Self {
            error: error.to_owned(),
            message: message.into(),
        }
    }

    /// The HTTP status the W3C specification gives this error.
    fn status(&self) -> u16 {
        match self.error.as_str() {
            "invalid argument"
            | "invalid selector"
            | "element not interactable"
            | "element click intercepted"
            | "invalid element state" => 400,
            "no such element"
            | "no such window"
            | "no such frame"
            | "no such alert"
            | "stale element reference"
            | "invalid session id"
            | "unknown command" => 404,
            "unknown method" => 405,
            _ => 500,
        }
    }
}

/// The answer to one HTTP request.
#[derive(Debug, PartialEq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The JSON body.
    pub body: Value,
}

/// Answers WebDriver requests for one app.
pub struct Driver<B> {
    bridge: B,
    token: String,
    /// The debugging port of the app's own Chromium, when the app's launch
    /// settings turned one on in this process. Read once, when the plugin
    /// starts; `GET /<token>/cdp` reports it and nothing else.
    cdp_port: Option<u16>,

    session: Mutex<Option<String>>,
    sessions: AtomicU64,
    timeouts: Mutex<Timeouts>,
}

impl<B: Bridge> Driver<B> {
    /// A driver whose URLs start with `/<token>`.
    pub fn new(bridge: B, token: impl Into<String>) -> Self {
        Self {
            bridge,
            token: token.into(),
            cdp_port: None,
            session: Mutex::new(None),
            sessions: AtomicU64::new(0),
            timeouts: Mutex::new(Timeouts::default()),
        }
    }

    /// Reports `port` on `GET /<token>/cdp` as the debugging port of the app's
    /// own Chromium. The plugin passes the port its start-up read from the
    /// launch settings ([`crate::cdp::enabled_port`]); without this call the
    /// route says CDP is off, whatever the environment holds.
    #[must_use]
    pub fn with_cdp_port(mut self, port: u16) -> Self {
        self.cdp_port = Some(port);
        self
    }

    /// Answers one request. `origin` is the request's `Origin` header: browsers
    /// send one and WebDriver clients do not, so a request that has one is
    /// refused (a web page must not drive the app).
    pub fn handle(&self, method: &str, url: &str, origin: Option<&str>, body: &[u8]) -> Response {
        let result = if origin.is_some() {
            Err(WebDriverError::new(
                "unknown error",
                "requests from web pages are refused",
            ))
        } else {
            self.route(method, url, body)
        };
        match result {
            Ok(value) => Response {
                status: 200,
                body: json!({ "value": value }),
            },
            Err(error) => Response {
                status: if origin.is_some() {
                    403
                } else {
                    error.status()
                },
                body: json!({ "value": { "error": error.error, "message": error.message, "stacktrace": "" } }),
            },
        }
    }

    fn route(&self, method: &str, url: &str, body: &[u8]) -> Result<Value, WebDriverError> {
        let path = url.split('?').next().unwrap_or_default();
        let rest = strip_token(path, &self.token)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
            .ok_or_else(|| {
                WebDriverError::new("unknown command", "the URL does not start with the token")
            })?;
        let segments: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
        let body: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(body).map_err(|error| {
                WebDriverError::new("invalid argument", format!("the body is not JSON: {error}"))
            })?
        };
        match (method, segments.as_slice()) {
            ("GET", ["status"]) => {
                Ok(json!({ "ready": true, "message": "tauri-plugin-sbrs-automation is ready" }))
            }
            ("GET", ["cdp"]) => Ok(crate::cdp::discovery(self.cdp_port)),
            ("POST", ["session"]) => self.new_session(),
            ("DELETE", ["session", id]) => {
                self.check_session(id)?;
                *self
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                Ok(Value::Null)
            }
            (_, ["session", id, command @ ..]) => {
                self.check_session(id)?;
                self.command(method, command, &body)
            }
            _ => Err(unknown(method, rest)),
        }
    }

    /// Starts a session once the app's window has loaded its page. The plugin
    /// serves before the app has opened its window, and a session needs a page
    /// to drive, as a browser driver's does.
    fn new_session(&self) -> Result<Value, WebDriverError> {
        let timeouts = Timeouts::default();
        self.wait_until_loaded(timeouts.page_load()).map_err(|_| {
            WebDriverError::new(
                "session not created",
                "the app's window did not load its page in time",
            )
        })?;
        // A session starts in the top page, with no frame entered, no
        // dialog answer and no key or button held from the last session.
        self.call("reset", json!({})).map_err(|error| {
            WebDriverError::new(
                "session not created",
                format!("the page could not be reset: {}", error.message),
            )
        })?;
        let number = self.sessions.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("sbrs-automation-{number}");
        *self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id.clone());
        *self.timeouts_mut() = timeouts;
        Ok(json!({
            "sessionId": id,
            "capabilities": {
                "browserName": "tauri",
                "platformName": std::env::consts::OS,
                "acceptInsecureCerts": false,
                "setWindowRect": true,
                "timeouts": timeouts.to_json(),
            },
        }))
    }

    fn check_session(&self, id: &str) -> Result<(), WebDriverError> {
        let current = self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_deref() == Some(id) {
            Ok(())
        } else {
            Err(WebDriverError::new(
                "invalid session id",
                format!("no session `{id}`"),
            ))
        }
    }

    fn call(&self, op: &str, args: Value) -> Result<Value, WebDriverError> {
        self.bridge.call(op, args, CALL_TIMEOUT)
    }

    fn timeouts_mut(&self) -> std::sync::MutexGuard<'_, Timeouts> {
        self.timeouts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn timeouts(&self) -> Timeouts {
        *self.timeouts_mut()
    }

    fn command(
        &self,
        method: &str,
        command: &[&str],
        body: &Value,
    ) -> Result<Value, WebDriverError> {
        match (method, command) {
            ("GET", ["timeouts"]) => Ok(self.timeouts().to_json()),
            ("POST", ["timeouts"]) => {
                let mut timeouts = self.timeouts_mut();
                *timeouts = timeouts.updated(body)?;
                Ok(Value::Null)
            }
            ("POST", ["url"]) => {
                let url = text(body, "url")?;
                self.call("go", json!({ "url": url }))?;
                self.wait_until_loaded(self.timeouts().page_load())
            }
            ("GET", [what @ ("url" | "title" | "source")]) => {
                self.call("info", json!({ "what": what }))
            }
            ("POST", [action @ ("back" | "forward" | "refresh")]) => {
                self.call("history", json!({ "action": action }))?;
                self.wait_until_loaded(self.timeouts().page_load())
            }
            ("GET", ["window"]) => Ok(json!("main")),
            ("GET", ["window", "handles"]) => Ok(json!(["main"])),
            ("POST", ["window"]) => match body.get("handle").and_then(Value::as_str) {
                Some("main") => Ok(Value::Null),
                _ => Err(WebDriverError::new(
                    "no such window",
                    "the app has one window, `main`",
                )),
            },
            ("GET", ["window", "rect"]) => self.window(WindowCommand::Rect),
            ("POST", ["window", "rect"]) => self.window(set_rect(body)?),
            ("POST", ["window", "maximize"]) => self.window(WindowCommand::Maximize),
            ("POST", ["window", "minimize"]) => self.window(WindowCommand::Minimize),
            ("POST", ["window", "fullscreen"]) => self.window(WindowCommand::Fullscreen),
            ("POST", ["element"]) => self.find(None, body, false),
            ("POST", ["elements"]) => self.find(None, body, true),
            ("GET", ["element", "active"]) => self.call("active", json!({})),
            ("POST", ["element", id, "element"]) => self.find(Some(id), body, false),
            ("POST", ["element", id, "elements"]) => self.find(Some(id), body, true),
            ("POST", ["element", id, op @ ("click" | "clear")]) => {
                self.call(op, json!({ "id": id }))
            }
            ("POST", ["element", id, "value"]) => {
                let typed = match body.get("text").and_then(Value::as_str) {
                    Some(text) => text.to_owned(),
                    None => body
                        .get("value")
                        .and_then(Value::as_array)
                        .map(|keys| keys.iter().filter_map(Value::as_str).collect())
                        .ok_or_else(|| {
                            WebDriverError::new("invalid argument", "`text` is missing")
                        })?,
                };
                self.call("keys", json!({ "id": id, "text": typed }))
            }
            (
                "GET",
                [
                    "element",
                    id,
                    what @ ("text" | "name" | "rect" | "enabled" | "selected" | "displayed"),
                ],
            ) => self.call("prop", json!({ "id": id, "what": what })),
            (
                "GET",
                [
                    "element",
                    id,
                    what @ ("attribute" | "property" | "css"),
                    name,
                ],
            ) => self.call("prop", json!({ "id": id, "what": what, "name": name })),
            ("POST", ["execute", kind @ ("sync" | "async")]) => self.script(json!({
                "script": text(body, "script")?,
                "args": body.get("args").cloned().unwrap_or_else(|| json!([])),
                "isAsync": *kind == "async",
            })),
            ("GET", ["alert", "text"]) => self.call("dialog", json!({ "action": "text" })),
            ("POST", ["alert", action @ ("accept" | "dismiss")]) => {
                self.call("dialog", json!({ "action": action }))
            }
            ("POST", ["alert", "text"]) => self.call(
                "dialog",
                json!({ "action": "type", "text": text(body, "text")? }),
            ),
            ("POST", ["frame"]) => self.call("frame", frame_target(body)?),
            ("POST", ["frame", "parent"]) => self.call("frame", json!({ "parent": true })),
            ("POST", ["actions"]) => self.perform(body),
            ("DELETE", ["actions"]) => self.call("release", json!({})),
            ("GET", ["screenshot"]) => self.bridge.screenshot(None).map(Value::from),
            ("GET", ["element", id, "screenshot"]) => {
                let area = self.call("prop", json!({ "id": id, "what": "viewrect" }))?;
                self.bridge
                    .screenshot(Some(viewport_rect(&area)?))
                    .map(Value::from)
            }
            _ => Err(unknown(method, &command.join("/"))),
        }
    }

    fn window(&self, command: WindowCommand) -> Result<Value, WebDriverError> {
        let rect = self.bridge.window(command)?;
        // W3C reports whole pixels, and clients read integers (thirtyfour's
        // `Rect` holds `i64`), while a scale factor such as 1.5 gives fractions.
        let whole = |value: f64| value.round() as i64;
        Ok(json!({
            "x": whole(rect.x),
            "y": whole(rect.y),
            "width": whole(rect.width),
            "height": whole(rect.height),
        }))
    }

    /// Performs W3C actions tick by tick: the page dispatches a tick's input
    /// at once, and the tick then lasts as long as its longest pause or move.
    /// The whole sequence is checked before anything runs.
    fn perform(&self, body: &Value) -> Result<Value, WebDriverError> {
        for tick in actions::ticks(body)? {
            if !tick.inputs.is_empty() {
                self.call("input", json!({ "actions": tick.inputs }))?;
            }
            if !tick.duration.is_zero() {
                std::thread::sleep(tick.duration);
            }
        }
        Ok(Value::Null)
    }

    /// Runs a script for as long as the session's script timeout allows.
    fn script(&self, args: Value) -> Result<Value, WebDriverError> {
        let limit = self
            .timeouts()
            .script
            .map_or(Duration::MAX, Duration::from_millis);
        self.bridge.call("script", args, limit).map_err(|error| {
            if error.error == "timeout" {
                WebDriverError::new("script timeout", error.message)
            } else {
                error
            }
        })
    }

    /// Finds elements, looking again until something matches or the
    /// session's implicit wait is over.
    fn find(&self, root: Option<&str>, body: &Value, all: bool) -> Result<Value, WebDriverError> {
        let args = json!({ "using": text(body, "using")?, "value": text(body, "value")?, "root": root, "all": all });
        let started = Instant::now();
        let wait = Duration::from_millis(self.timeouts().implicit);
        loop {
            let found = self.call("find", args.clone());
            let nothing = match &found {
                Err(error) => error.error == "no such element",
                Ok(Value::Array(list)) => list.is_empty(),
                Ok(_) => false,
            };
            if !nothing || started.elapsed() >= wait {
                return found;
            }
            std::thread::sleep(RETRY_PAUSE);
        }
    }

    /// Waits up to `limit` until the page that a navigation started has
    /// loaded. The script answers `leaving` until the old page is gone, so an
    /// old page's `complete` is never taken for the new one.
    fn wait_until_loaded(&self, limit: Duration) -> Result<Value, WebDriverError> {
        let started = Instant::now();
        while started.elapsed() < limit {
            // While a page unloads, a call can be lost: a short timeout, then again.
            if let Ok(state) = self
                .bridge
                .call("info", json!({ "what": "ready" }), POLL_TIMEOUT)
                && state == "complete"
            {
                return Ok(Value::Null);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(WebDriverError::new(
            "timeout",
            "the page did not finish loading",
        ))
    }
}

/// The Set Window Rect body: each of `x`, `y`, `width` and `height` is
/// missing, `null` or an integer in the range W3C WebDriver allows.
fn set_rect(body: &Value) -> Result<WindowCommand, WebDriverError> {
    const LIMIT: f64 = 2_147_483_647.0;
    let field = |key: &str, min: f64| -> Result<Option<f64>, WebDriverError> {
        match body.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_f64()
                .filter(|number| (min..=LIMIT).contains(number))
                .map(Some)
                .ok_or_else(|| {
                    WebDriverError::new(
                        "invalid argument",
                        format!("`{key}` must be null or a number from {min} to {LIMIT}"),
                    )
                }),
        }
    };
    Ok(WindowCommand::SetRect {
        x: field("x", -LIMIT - 1.0)?,
        y: field("y", -LIMIT - 1.0)?,
        width: field("width", 0.0)?,
        height: field("height", 0.0)?,
    })
}

/// The frame Switch To Frame names, for the page: `null` for the top page,
/// an index into the current frame's frames, or a frame element.
fn frame_target(body: &Value) -> Result<Value, WebDriverError> {
    match body.get("id") {
        Some(Value::Null) => Ok(json!({ "top": true })),
        Some(Value::Number(number)) => number
            .as_u64()
            .filter(|index| *index <= u64::from(u16::MAX))
            .map(|index| json!({ "index": index }))
            .ok_or_else(|| invalid("a frame index is a whole number from 0 to 65535")),
        Some(Value::Object(reference)) => reference
            .get(ELEMENT_KEY)
            .and_then(Value::as_str)
            .map(|id| json!({ "element": id }))
            .ok_or_else(|| invalid("`id` is an object but not an element reference")),
        _ => Err(invalid(
            "`id` must be null, a frame index or a frame element",
        )),
    }
}

/// The rectangle the page gave for an element screenshot.
fn viewport_rect(value: &Value) -> Result<ViewportRect, WebDriverError> {
    let field = |key: &str| {
        value.get(key).and_then(Value::as_f64).ok_or_else(|| {
            WebDriverError::new(
                "unknown error",
                "the page gave no rectangle for the element",
            )
        })
    };
    Ok(ViewportRect {
        x: field("x")?,
        y: field("y")?,
        width: field("width")?,
        height: field("height")?,
    })
}

/// A session's timeouts in milliseconds (the W3C `timeouts` object).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timeouts {
    /// How long a script may run; `None` for no limit.
    script: Option<u64>,
    /// How long a navigation may take to load its page.
    page_load: u64,
    /// How long finding elements keeps looking.
    implicit: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            script: Some(30_000),
            page_load: 60_000,
            implicit: 0,
        }
    }
}

impl Timeouts {
    fn page_load(self) -> Duration {
        Duration::from_millis(self.page_load)
    }

    fn to_json(self) -> Value {
        json!({ "script": self.script, "pageLoad": self.page_load, "implicit": self.implicit })
    }

    /// These timeouts with the ones `body` sets (W3C Set Timeouts): a key
    /// left out keeps its value, and only `script` may be `null`.
    fn updated(mut self, body: &Value) -> Result<Self, WebDriverError> {
        if let Some(value) = body.get("script") {
            self.script = if value.is_null() {
                None
            } else {
                Some(milliseconds("script", value)?)
            };
        }
        if let Some(value) = body.get("pageLoad") {
            self.page_load = milliseconds("pageLoad", value)?;
        }
        if let Some(value) = body.get("implicit") {
            self.implicit = milliseconds("implicit", value)?;
        }
        Ok(self)
    }
}

/// A timeout: a whole number of milliseconds up to 2^53 - 1.
fn milliseconds(key: &str, value: &Value) -> Result<u64, WebDriverError> {
    const MAX: u64 = (1 << 53) - 1;
    value.as_u64().filter(|ms| *ms <= MAX).ok_or_else(|| {
        WebDriverError::new(
            "invalid argument",
            format!("`{key}` must be a whole number of milliseconds from 0 to {MAX}"),
        )
    })
}

fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str, WebDriverError> {
    body.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| WebDriverError::new("invalid argument", format!("`{key}` is missing")))
}

fn invalid(message: impl Into<String>) -> WebDriverError {
    WebDriverError::new("invalid argument", message)
}

fn unsupported(message: impl Into<String>) -> WebDriverError {
    WebDriverError::new("unsupported operation", message)
}

/// The rest of `path` after `/` and `token`, when it starts with them. The
/// token is compared in full, byte for byte, with [`constant_time_eq`]: a
/// request must not learn how many leading bytes of a guess were right from
/// how long the answer took. Only the length of the token can show.
fn strip_token<'a>(path: &'a str, token: &str) -> Option<&'a str> {
    let after_slash = path.strip_prefix('/')?;
    let candidate = after_slash.as_bytes().get(..token.len())?;
    if !constant_time_eq(candidate, token.as_bytes()) {
        return None;
    }
    // The bytes equal the token's, valid UTF-8, so this is a character boundary.
    after_slash.get(token.len()..)
}

/// Whether `a` and `b` hold the same bytes, looking at every byte whenever
/// the lengths agree: the differences are or-ed together instead of stopping
/// at the first one. `black_box` keeps the compiler from turning the loop
/// back into an early exit.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let difference = a.iter().zip(b).fold(0u8, |sum, (x, y)| sum | (x ^ y));
    std::hint::black_box(difference) == 0
}

fn unknown(method: &str, path: &str) -> WebDriverError {
    WebDriverError::new(
        "unknown command",
        format!("{method} {path} is not supported"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn the_token_comparison_looks_at_every_byte() {
        assert!(constant_time_eq(b"t0k3n", b"t0k3n"));
        assert!(constant_time_eq(b"", b""));
        // A difference in the first, the middle or the last byte, or in the
        // length, is a mismatch like any other.
        for other in [&b"X0k3n"[..], b"t0X3n", b"t0k3X", b"t0k3", b"t0k3nn", b""] {
            assert!(!constant_time_eq(b"t0k3n", other), "{other:?}");
        }
    }

    #[test]
    fn only_the_whole_token_is_stripped_from_the_start_of_the_path() {
        assert_eq!(strip_token("/t0k/status", "t0k"), Some("/status"));
        assert_eq!(strip_token("/t0k", "t0k"), Some(""));
        assert_eq!(strip_token("/t0kX/status", "t0k"), Some("X/status"));
        // A token with a multi-byte character is compared byte for byte.
        assert_eq!(strip_token("/t\u{f6}k/a", "t\u{f6}k"), Some("/a"));
        for path in [
            "",
            "/",
            "t0k/status",
            "//t0k/status",
            "/t0",
            "/T0K/status",
            "/t0\u{e9}x",
            "/\u{e9}\u{e9}\u{e9}",
        ] {
            assert_eq!(strip_token(path, "t0k"), None, "{path:?}");
        }
    }

    #[test]
    fn a_path_that_cuts_a_character_in_two_is_refused_not_a_panic() {
        // Byte 3 of "/t0\u{e9}" lies inside the two bytes of the last character.
        let driver = Driver::new(Fake::default(), "t0k");
        for path in ["/t0\u{e9}", "/t\u{e9}k/status", "/\u{1f600}/status"] {
            assert_eq!(driver.handle("GET", path, None, b"").status, 404, "{path}");
        }
    }

    /// Records each operation. The page's load state (`info` `ready`) comes
    /// from `states` (`complete` when empty), `reset` answers `null`, and
    /// every other answer comes from `answers` (`null` when empty).
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<(String, Value)>>,
        answers: Mutex<VecDeque<Result<Value, WebDriverError>>>,
        states: Mutex<VecDeque<Result<Value, WebDriverError>>>,
        windows: Mutex<Vec<WindowCommand>>,
        /// How long each call was allowed to take.
        waits: Mutex<Vec<Duration>>,
        /// The area of each screenshot taken (`None` for the whole view).
        shots: Mutex<Vec<Option<ViewportRect>>>,
    }

    /// What the fake bridge answers for a screenshot.
    const PNG: &str = "iVBORw0KGgo=";

    /// A bridge that only runs operations: it has no window and takes no
    /// screenshots.
    struct Bare;

    impl Bridge for Bare {
        fn call(
            &self,
            op: &str,
            _args: Value,
            _timeout: Duration,
        ) -> Result<Value, WebDriverError> {
            Ok(if op == "info" {
                json!("complete")
            } else {
                Value::Null
            })
        }
    }

    impl Fake {
        fn answering(answers: Vec<Result<Value, WebDriverError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into()),
                ..Self::default()
            }
        }

        fn loading(states: Vec<Result<Value, WebDriverError>>) -> Self {
            Self {
                states: Mutex::new(states.into()),
                ..Self::default()
            }
        }
    }

    impl Bridge for Fake {
        fn call(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, WebDriverError> {
            let state = op == "info" && args["what"] == "ready";
            self.calls.lock().unwrap().push((op.to_owned(), args));
            self.waits.lock().unwrap().push(timeout);
            if state {
                self.states
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Ok(json!("complete")))
            } else if op == "reset" {
                Ok(Value::Null)
            } else {
                self.answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Ok(Value::Null))
            }
        }

        fn screenshot(&self, area: Option<ViewportRect>) -> Result<String, WebDriverError> {
            self.shots.lock().unwrap().push(area);
            Ok(PNG.to_owned())
        }

        fn window(&self, command: WindowCommand) -> Result<WindowRect, WebDriverError> {
            self.windows.lock().unwrap().push(command);
            // A scale factor such as 1.5 gives fractions.
            Ok(WindowRect {
                x: 10.4,
                y: 20.6,
                width: 800.0,
                height: 600.5,
            })
        }
    }

    /// Starts a session and forgets the calls that took.
    fn session(driver: &Driver<Fake>) -> String {
        let response = driver.handle("POST", "/t0k/session", None, b"{}");
        driver.bridge.calls.lock().unwrap().clear();
        response.body["value"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn elements_are_named_with_the_w3c_web_element_identifier() {
        // Clients read an element only under this key (W3C WebDriver, "web
        // element identifier"); any other is not an element to them.
        assert_eq!(ELEMENT_KEY, "element-6066-11e4-a52e-4f735466cecf");
        let script = include_str!("bridge.ts");
        assert!(
            script.contains(&format!("const KEY = '{ELEMENT_KEY}';")),
            "bridge.ts names elements with another key"
        );
    }

    #[test]
    fn the_page_script_serves_every_operation_the_driver_asks_for() {
        let script = include_str!("bridge.ts");
        for op in [
            "info", "go", "history", "find", "active", "prop", "click", "clear", "keys", "script",
            "dialog", "frame", "input", "release", "reset",
        ] {
            assert!(
                script.contains(&format!("\n    {op}: ")),
                "bridge.ts has no `{op}` operation"
            );
        }
    }

    #[test]
    fn status_needs_the_token_and_web_pages_are_refused() {
        let driver = Driver::new(Fake::default(), "t0k");
        assert_eq!(driver.handle("GET", "/t0k/status", None, b"").status, 200);
        assert_eq!(driver.handle("GET", "/status", None, b"").status, 404);
        assert_eq!(driver.handle("GET", "/t0kX/status", None, b"").status, 404);
        let from_page = driver.handle("GET", "/t0k/status", Some("https://evil.example"), b"");
        assert_eq!(from_page.status, 403);
    }

    #[test]
    fn a_session_is_needed_and_its_id_checked() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        assert_eq!(
            driver
                .handle("GET", &format!("/t0k/session/{id}/window"), None, b"")
                .body["value"],
            "main"
        );
        let wrong = driver.handle("GET", "/t0k/session/nope/window", None, b"");
        assert_eq!(
            (wrong.status, wrong.body["value"]["error"].as_str()),
            (404, Some("invalid session id"))
        );
        assert_eq!(
            driver
                .handle("DELETE", &format!("/t0k/session/{id}"), None, b"")
                .status,
            200
        );
        assert_eq!(
            driver
                .handle("GET", &format!("/t0k/session/{id}/window"), None, b"")
                .status,
            404
        );
    }

    #[test]
    fn finding_an_element_asks_the_page_and_passes_errors_on() {
        let reference = json!({ ELEMENT_KEY: "e1" });
        let driver = Driver::new(
            Fake::answering(vec![
                Ok(reference.clone()),
                Err(WebDriverError::new("no such element", "no #missing")),
            ]),
            "t0k",
        );
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/element");
        let found = driver.handle(
            "POST",
            &url,
            None,
            br##"{"using":"css selector","value":"#save"}"##,
        );
        assert_eq!(found.body["value"], reference);
        let missing = driver.handle(
            "POST",
            &url,
            None,
            br##"{"using":"css selector","value":"#missing"}"##,
        );
        assert_eq!(
            (missing.status, missing.body["value"]["error"].as_str()),
            (404, Some("no such element"))
        );
        let calls = driver.bridge.calls.lock().unwrap();
        assert_eq!(
            calls[0],
            (
                "find".to_owned(),
                json!({ "using": "css selector", "value": "#save", "root": null, "all": false })
            )
        );
    }

    #[test]
    fn typing_accepts_text_or_the_old_value_array() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/element/e1/value");
        driver.handle("POST", &url, None, br#"{"text":"hi"}"#);
        driver.handle("POST", &url, None, br#"{"value":["h","o"]}"#);
        let calls = driver.bridge.calls.lock().unwrap();
        assert_eq!(calls[0].1, json!({ "id": "e1", "text": "hi" }));
        assert_eq!(calls[1].1, json!({ "id": "e1", "text": "ho" }));
    }

    #[test]
    fn a_new_session_waits_for_the_window_and_its_page() {
        // The plugin serves before the app has opened its window, so a client
        // that starts a session at once must not get one that has no page yet.
        let driver = Driver::new(
            Fake::loading(vec![
                Err(WebDriverError::new(
                    "no such window",
                    "the app has no window",
                )),
                Ok(json!("loading")),
                Ok(json!("complete")),
            ]),
            "t0k",
        );
        let response = driver.handle("POST", "/t0k/session", None, b"{}");
        assert_eq!(response.status, 200);
        assert!(response.body["value"]["sessionId"].is_string());
        let calls = driver.bridge.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(
            calls[..3]
                .iter()
                .all(|(op, args)| op == "info" && args == &json!({ "what": "ready" }))
        );
        // Then the page forgets the last session's frame, dialog answers and
        // pressed keys: a session starts in the top page.
        assert_eq!(calls[3], ("reset".to_owned(), json!({})));
    }

    #[test]
    fn navigation_waits_for_the_new_page() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        driver.bridge.states.lock().unwrap().extend([
            Ok(json!("leaving")),
            Err(WebDriverError::new("timeout", "lost while unloading")),
            Ok(json!("loading")),
            Ok(json!("complete")),
        ]);
        let response = driver.handle(
            "POST",
            &format!("/t0k/session/{id}/url"),
            None,
            br#"{"url":"https://example.com"}"#,
        );
        assert_eq!(response.status, 200);
        assert_eq!(driver.bridge.calls.lock().unwrap().len(), 5);
    }

    #[test]
    fn the_window_rect_comes_from_the_native_window() {
        // The page cannot tell: WKWebView reports outerWidth and outerHeight 0.
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = |command: &str| format!("/t0k/session/{id}/window/{command}");
        let rect = driver.handle("GET", &url("rect"), None, b"");
        // Whole pixels: W3C says integers, and clients read them as such.
        assert_eq!(
            rect.body["value"],
            json!({ "x": 10, "y": 21, "width": 800, "height": 601 })
        );
        let set = driver.handle(
            "POST",
            &url("rect"),
            None,
            br#"{"x":5,"y":6,"width":640,"height":null}"#,
        );
        assert_eq!(set.status, 200);
        for command in ["maximize", "minimize", "fullscreen"] {
            assert_eq!(
                driver.handle("POST", &url(command), None, b"{}").status,
                200
            );
        }
        assert_eq!(
            *driver.bridge.windows.lock().unwrap(),
            vec![
                WindowCommand::Rect,
                WindowCommand::SetRect {
                    x: Some(5.0),
                    y: Some(6.0),
                    width: Some(640.0),
                    height: None,
                },
                WindowCommand::Maximize,
                WindowCommand::Minimize,
                WindowCommand::Fullscreen,
            ]
        );
        assert!(driver.bridge.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_window_rect_that_is_not_one_is_refused() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/window/rect");
        for body in [
            &br#"{"width":-1}"#[..],
            br#"{"x":"left"}"#,
            br#"{"height":1e300}"#,
        ] {
            let refused = driver.handle("POST", &url, None, body);
            assert_eq!(
                (refused.status, refused.body["value"]["error"].as_str()),
                (400, Some("invalid argument")),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert!(driver.bridge.windows.lock().unwrap().is_empty());
    }

    #[test]
    fn timeouts_are_kept_reported_and_reset_by_a_new_session() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/timeouts");
        let set = driver.handle(
            "POST",
            &url,
            None,
            br#"{"script":null,"pageLoad":7000,"implicit":250}"#,
        );
        assert_eq!(set.status, 200);
        assert_eq!(
            driver.handle("GET", &url, None, b"").body["value"],
            json!({ "script": null, "pageLoad": 7000, "implicit": 250 })
        );
        // A key left out keeps its value.
        driver.handle("POST", &url, None, br#"{"script":5000}"#);
        assert_eq!(
            driver.handle("GET", &url, None, b"").body["value"],
            json!({ "script": 5000, "pageLoad": 7000, "implicit": 250 })
        );
        let id = session(&driver);
        assert_eq!(
            driver
                .handle("GET", &format!("/t0k/session/{id}/timeouts"), None, b"")
                .body["value"],
            json!({ "script": 30_000, "pageLoad": 60_000, "implicit": 0 })
        );
    }

    #[test]
    fn timeouts_that_are_not_ones_are_refused() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/timeouts");
        for body in [
            &br#"{"implicit":-1}"#[..],
            br#"{"implicit":1.5}"#,
            br#"{"pageLoad":null}"#,
            br#"{"script":"soon"}"#,
        ] {
            let refused = driver.handle("POST", &url, None, body);
            assert_eq!(
                (refused.status, refused.body["value"]["error"].as_str()),
                (400, Some("invalid argument")),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn a_script_may_run_as_long_as_the_script_timeout() {
        let driver = Driver::new(
            Fake::answering(vec![
                Ok(json!(1)),
                Err(WebDriverError::new("timeout", "no answer")),
            ]),
            "t0k",
        );
        let id = session(&driver);
        driver.handle(
            "POST",
            &format!("/t0k/session/{id}/timeouts"),
            None,
            br#"{"script":45000}"#,
        );
        driver.bridge.waits.lock().unwrap().clear();
        let url = format!("/t0k/session/{id}/execute/async");
        let body = br#"{"script":"arguments[0](1)","args":[]}"#;
        assert_eq!(driver.handle("POST", &url, None, body).status, 200);
        assert_eq!(
            driver.bridge.waits.lock().unwrap()[0],
            Duration::from_secs(45)
        );
        // A script that runs out of time says so with the W3C code.
        let late = driver.handle("POST", &url, None, body);
        assert_eq!(
            (late.status, late.body["value"]["error"].as_str()),
            (500, Some("script timeout"))
        );
    }

    #[test]
    fn the_implicit_wait_looks_again_until_something_matches() {
        let reference = json!({ ELEMENT_KEY: "e1" });
        let driver = Driver::new(
            Fake::answering(vec![
                Err(WebDriverError::new("no such element", "not yet")),
                Ok(reference.clone()),
                Ok(json!([])),
                Ok(json!([reference.clone()])),
            ]),
            "t0k",
        );
        let id = session(&driver);
        driver.handle(
            "POST",
            &format!("/t0k/session/{id}/timeouts"),
            None,
            br#"{"implicit":5000}"#,
        );
        let find = |what: &str| {
            driver.handle(
                "POST",
                &format!("/t0k/session/{id}/{what}"),
                None,
                br##"{"using":"css selector","value":"#late"}"##,
            )
        };
        assert_eq!(find("element").body["value"], reference);
        assert_eq!(find("elements").body["value"], json!([reference]));
        let calls = driver.bridge.calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|(op, _)| op == "find").count(), 4);
    }

    #[test]
    fn unsupported_and_unknown_commands_say_so() {
        // A bridge that cannot take screenshots (the default) says so.
        let driver = Driver::new(Bare, "t0k");
        let started = driver.handle("POST", "/t0k/session", None, b"{}");
        let id = started.body["value"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        let shot = driver.handle("GET", &format!("/t0k/session/{id}/screenshot"), None, b"");
        assert_eq!(
            (shot.status, shot.body["value"]["error"].as_str()),
            (500, Some("unsupported operation"))
        );
        let cookies = driver.handle("GET", &format!("/t0k/session/{id}/cookie"), None, b"");
        assert_eq!(
            (cookies.status, cookies.body["value"]["error"].as_str()),
            (404, Some("unknown command"))
        );
    }

    #[test]
    fn user_prompts_are_answered_by_the_page() {
        let driver = Driver::new(
            Fake::answering(vec![
                Ok(json!("Delete it?")),
                Err(WebDriverError::new("no such alert", "no dialog is open")),
            ]),
            "t0k",
        );
        let id = session(&driver);
        let url = |path: &str| format!("/t0k/session/{id}/alert/{path}");
        let open = driver.handle("GET", &url("text"), None, b"");
        assert_eq!(open.body["value"], "Delete it?");
        let none = driver.handle("GET", &url("text"), None, b"");
        assert_eq!(
            (none.status, none.body["value"]["error"].as_str()),
            (404, Some("no such alert"))
        );
        for (path, body) in [
            ("accept", &b"{}"[..]),
            ("dismiss", b"{}"),
            ("text", br#"{"text":"Bob"}"#),
        ] {
            assert_eq!(driver.handle("POST", &url(path), None, body).status, 200);
        }
        let no_text = driver.handle("POST", &url("text"), None, b"{}");
        assert_eq!(
            (no_text.status, no_text.body["value"]["error"].as_str()),
            (400, Some("invalid argument"))
        );
        let dialog = |args: Value| ("dialog".to_owned(), args);
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![
                dialog(json!({ "action": "text" })),
                dialog(json!({ "action": "text" })),
                dialog(json!({ "action": "accept" })),
                dialog(json!({ "action": "dismiss" })),
                dialog(json!({ "action": "type", "text": "Bob" })),
            ]
        );
    }

    #[test]
    fn frames_are_entered_by_index_or_element_and_left_for_the_parent_or_top() {
        let driver = Driver::new(
            Fake::answering(vec![
                Ok(Value::Null),
                Ok(Value::Null),
                Ok(Value::Null),
                Ok(Value::Null),
                Err(WebDriverError::new("no such frame", "cross-origin")),
            ]),
            "t0k",
        );
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/frame");
        let element = serde_json::to_vec(&json!({ "id": { ELEMENT_KEY: "e7" } })).unwrap();
        for body in [&br#"{"id":2}"#[..], &element, br#"{"id":null}"#] {
            assert_eq!(driver.handle("POST", &url, None, body).status, 200);
        }
        let parent = driver.handle("POST", &format!("{url}/parent"), None, b"{}");
        assert_eq!(parent.status, 200);
        let foreign = driver.handle("POST", &url, None, br#"{"id":0}"#);
        assert_eq!(
            (foreign.status, foreign.body["value"]["error"].as_str()),
            (404, Some("no such frame"))
        );
        let frame = |args: Value| ("frame".to_owned(), args);
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![
                frame(json!({ "index": 2 })),
                frame(json!({ "element": "e7" })),
                frame(json!({ "top": true })),
                frame(json!({ "parent": true })),
                frame(json!({ "index": 0 })),
            ]
        );
    }

    #[test]
    fn a_frame_that_is_not_one_is_refused() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let url = format!("/t0k/session/{id}/frame");
        for body in [
            &b"{}"[..],
            br#"{"id":-1}"#,
            br#"{"id":65536}"#,
            br#"{"id":1.5}"#,
            br#"{"id":"main"}"#,
            br#"{"id":{"not-an-element":"e1"}}"#,
        ] {
            let refused = driver.handle("POST", &url, None, body);
            assert_eq!(
                (refused.status, refused.body["value"]["error"].as_str()),
                (400, Some("invalid argument")),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert!(driver.bridge.calls.lock().unwrap().is_empty());
    }

    /// The body thirtyfour 0.37 sends for `double_click_element`: the key
    /// source only pauses, the mouse moves onto the element and clicks twice.
    fn double_click(move_ms: u64) -> Vec<u8> {
        let pause = json!({ "type": "pause", "duration": 0 });
        let down = json!({ "type": "pointerDown", "button": 0, "duration": 0 });
        let up = json!({ "type": "pointerUp", "button": 0, "duration": 0 });
        let onto = json!({ "type": "pointerMove", "duration": move_ms, "origin": { ELEMENT_KEY: "e1" }, "x": 0, "y": 0 });
        serde_json::to_vec(&json!({ "actions": [
            { "id": "key", "type": "key", "actions": [pause, pause, pause, pause, pause] },
            {
                "id": "pointer",
                "type": "pointer",
                "parameters": { "pointerType": "mouse" },
                "actions": [onto, down, up, down, up],
            },
        ] }))
        .unwrap()
    }

    #[test]
    fn actions_run_tick_by_tick_and_each_tick_lasts_its_longest_move_or_pause() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let started = Instant::now();
        let done = driver.handle(
            "POST",
            &format!("/t0k/session/{id}/actions"),
            None,
            &double_click(120),
        );
        assert_eq!(done.status, 200, "{}", done.body);
        assert!(started.elapsed() >= Duration::from_millis(120));
        let input = |action: Value| ("input".to_owned(), json!({ "actions": [action] }));
        let down = json!({ "source": "pointer", "type": "pointerDown", "button": 0 });
        let up = json!({ "source": "pointer", "type": "pointerUp", "button": 0 });
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![
                input(json!({
                    "source": "pointer",
                    "type": "pointerMove",
                    "origin": { "element": "e1" },
                    "x": 0,
                    "y": 0,
                })),
                input(down.clone()),
                input(up.clone()),
                input(down),
                input(up),
            ]
        );
    }

    #[test]
    fn key_actions_go_to_the_page_and_a_pause_alone_does_not() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let body = json!({ "actions": [
            { "id": "keyboard", "type": "key", "actions": [
                { "type": "keyDown", "value": "a" },
                { "type": "keyUp", "value": "a" },
                { "type": "pause", "duration": 60 },
                { "type": "keyDown", "value": "\u{E007}" },
                { "type": "keyUp", "value": "\u{E007}" },
            ] },
            { "id": "idle", "type": "none", "actions": [{ "type": "pause" }] },
        ] });
        let started = Instant::now();
        let done = driver.handle(
            "POST",
            &format!("/t0k/session/{id}/actions"),
            None,
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(done.status, 200, "{}", done.body);
        assert!(started.elapsed() >= Duration::from_millis(60));
        let key = |kind: &str, value: &str| {
            (
                "input".to_owned(),
                json!({ "actions": [{ "source": "keyboard", "type": kind, "value": value }] }),
            )
        };
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![
                key("keyDown", "a"),
                key("keyUp", "a"),
                key("keyDown", "\u{E007}"),
                key("keyUp", "\u{E007}"),
            ]
        );
    }

    #[test]
    fn actions_that_are_not_simulated_are_unsupported_and_nothing_runs() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        for source in [
            json!({ "id": "wheel", "type": "wheel", "actions": [
                { "type": "scroll", "x": 0, "y": 0, "deltaX": 0, "deltaY": 50 },
            ] }),
            json!({ "id": "finger", "type": "pointer", "parameters": { "pointerType": "touch" },
                "actions": [{ "type": "pointerDown", "button": 0 }] }),
            json!({ "id": "mouse", "type": "pointer", "actions": [{ "type": "pointerCancel" }] }),
        ] {
            // A good source comes first: nothing runs when any is refused.
            let body = json!({ "actions": [
                { "id": "keyboard", "type": "key", "actions": [{ "type": "keyDown", "value": "a" }] },
                source,
            ] });
            let refused = driver.handle(
                "POST",
                &format!("/t0k/session/{id}/actions"),
                None,
                &serde_json::to_vec(&body).unwrap(),
            );
            assert_eq!(
                (refused.status, refused.body["value"]["error"].as_str()),
                (500, Some("unsupported operation")),
                "{body}"
            );
        }
        assert!(driver.bridge.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn actions_that_are_not_ones_are_refused() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let sequence = |source: Value| json!({ "actions": [source] });
        for body in [
            json!({}),
            sequence(json!({ "type": "key", "actions": [] })),
            sequence(json!({ "id": "x", "type": "joystick", "actions": [] })),
            sequence(json!({ "id": "k", "type": "key" })),
            sequence(
                json!({ "id": "k", "type": "key", "actions": [{ "type": "keyDown", "value": "" }] }),
            ),
            sequence(
                json!({ "id": "k", "type": "key", "actions": [{ "type": "pointerDown", "button": 0 }] }),
            ),
            sequence(
                json!({ "id": "p", "type": "pointer", "actions": [{ "type": "pointerDown", "button": "left" }] }),
            ),
            sequence(
                json!({ "id": "p", "type": "pointer", "actions": [{ "type": "pointerMove", "origin": "elsewhere" }] }),
            ),
            sequence(
                json!({ "id": "p", "type": "pointer", "actions": [{ "type": "pointerMove", "x": "left" }] }),
            ),
            sequence(
                json!({ "id": "n", "type": "none", "actions": [{ "type": "pause", "duration": -1 }] }),
            ),
        ] {
            let refused = driver.handle(
                "POST",
                &format!("/t0k/session/{id}/actions"),
                None,
                &serde_json::to_vec(&body).unwrap(),
            );
            assert_eq!(
                (refused.status, refused.body["value"]["error"].as_str()),
                (400, Some("invalid argument")),
                "{body}"
            );
        }
        assert!(driver.bridge.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn releasing_the_actions_lets_go_of_keys_and_buttons() {
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let released = driver.handle("DELETE", &format!("/t0k/session/{id}/actions"), None, b"");
        assert_eq!(released.status, 200);
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![("release".to_owned(), json!({}))]
        );
    }

    #[test]
    fn screenshots_come_from_the_bridge_and_an_element_is_cut_out_of_one() {
        let driver = Driver::new(
            Fake::answering(vec![Ok(
                json!({ "x": 10.5, "y": 20.0, "width": 30.0, "height": 40.0 }),
            )]),
            "t0k",
        );
        let id = session(&driver);
        let page = driver.handle("GET", &format!("/t0k/session/{id}/screenshot"), None, b"");
        assert_eq!((page.status, &page.body["value"]), (200, &json!(PNG)));
        let element = driver.handle(
            "GET",
            &format!("/t0k/session/{id}/element/e1/screenshot"),
            None,
            b"",
        );
        assert_eq!((element.status, &element.body["value"]), (200, &json!(PNG)));
        assert_eq!(
            *driver.bridge.shots.lock().unwrap(),
            vec![
                None,
                Some(ViewportRect {
                    x: 10.5,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                }),
            ]
        );
        // The page scrolls the element into view and says where it is.
        assert_eq!(
            *driver.bridge.calls.lock().unwrap(),
            vec![("prop".to_owned(), json!({ "id": "e1", "what": "viewrect" }))]
        );
    }
}
