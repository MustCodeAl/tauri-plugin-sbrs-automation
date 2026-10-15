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
            session: Mutex::new(None),
            sessions: AtomicU64::new(0),
            timeouts: Mutex::new(Timeouts::default()),
        }
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
        let rest = path
            .strip_prefix('/')
            .and_then(|path| path.strip_prefix(self.token.as_str()))
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
            ("GET", ["screenshot"]) | ("GET", ["element", _, "screenshot"]) => {
                Err(WebDriverError::new(
                    "unsupported operation",
                    "screenshots of a Tauri webview are not supported yet",
                ))
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

    /// Records each operation. The page's load state (`info` `ready`) comes
    /// from `states` (`complete` when empty), every other answer from
    /// `answers` (`null` when empty).
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<(String, Value)>>,
        answers: Mutex<VecDeque<Result<Value, WebDriverError>>>,
        states: Mutex<VecDeque<Result<Value, WebDriverError>>>,
        windows: Mutex<Vec<WindowCommand>>,
        /// How long each call was allowed to take.
        waits: Mutex<Vec<Duration>>,
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
            } else {
                self.answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Ok(Value::Null))
            }
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
        let script = include_str!("bridge.js");
        assert!(
            script.contains(&format!("const KEY = '{ELEMENT_KEY}';")),
            "bridge.js names elements with another key"
        );
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
        assert_eq!(calls.len(), 3);
        assert!(
            calls
                .iter()
                .all(|(op, args)| op == "info" && args == &json!({ "what": "ready" }))
        );
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
        let driver = Driver::new(Fake::default(), "t0k");
        let id = session(&driver);
        let shot = driver.handle("GET", &format!("/t0k/session/{id}/screenshot"), None, b"");
        assert_eq!(
            (shot.status, shot.body["value"]["error"].as_str()),
            (500, Some("unsupported operation"))
        );
        let alert = driver.handle("GET", &format!("/t0k/session/{id}/alert/text"), None, b"");
        assert_eq!(
            (alert.status, alert.body["value"]["error"].as_str()),
            (404, Some("unknown command"))
        );
    }
}
