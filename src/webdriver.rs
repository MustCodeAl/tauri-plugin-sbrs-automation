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

/// The key WebDriver uses for an element reference.
pub const ELEMENT_KEY: &str = "element-6066-11e4-a52f-4a20b6f6b6a0";

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_TIMEOUT: Duration = Duration::from_secs(2);
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs one operation of the injected script in the webview.
pub trait Bridge: Send + Sync {
    /// Runs `op` with `args` and returns its value, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// The WebDriver error the script reported, or `timeout`.
    fn call(&self, op: &str, args: Value, timeout: Duration) -> Result<Value, WebDriverError>;
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
}

impl<B: Bridge> Driver<B> {
    /// A driver whose URLs start with `/<token>`.
    pub fn new(bridge: B, token: impl Into<String>) -> Self {
        Self {
            bridge,
            token: token.into(),
            session: Mutex::new(None),
            sessions: AtomicU64::new(0),
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
            ("POST", ["session"]) => Ok(self.new_session()),
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

    fn new_session(&self) -> Value {
        let number = self.sessions.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("sbrs-automation-{number}");
        *self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id.clone());
        json!({
            "sessionId": id,
            "capabilities": {
                "browserName": "tauri",
                "platformName": std::env::consts::OS,
                "acceptInsecureCerts": false,
                "setWindowRect": false,
                "timeouts": timeouts(),
            },
        })
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

    fn command(
        &self,
        method: &str,
        command: &[&str],
        body: &Value,
    ) -> Result<Value, WebDriverError> {
        match (method, command) {
            ("GET", ["timeouts"]) => Ok(timeouts()),
            ("POST", ["timeouts"]) => Ok(Value::Null),
            ("POST", ["url"]) => {
                let url = text(body, "url")?;
                self.call("go", json!({ "url": url }))?;
                self.wait_until_loaded()
            }
            ("GET", [what @ ("url" | "title" | "source")]) => {
                self.call("info", json!({ "what": what }))
            }
            ("POST", [action @ ("back" | "forward" | "refresh")]) => {
                self.call("history", json!({ "action": action }))?;
                self.wait_until_loaded()
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
            ("GET", ["window", "rect"])
            | ("POST", ["window", "rect" | "maximize" | "minimize" | "fullscreen"]) => {
                self.call("rect", json!({}))
            }
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
            ("POST", ["execute", kind @ ("sync" | "async")]) => self.call(
                "script",
                json!({
                    "script": text(body, "script")?,
                    "args": body.get("args").cloned().unwrap_or_else(|| json!([])),
                    "isAsync": *kind == "async",
                }),
            ),
            ("GET", ["screenshot"]) | ("GET", ["element", _, "screenshot"]) => {
                Err(WebDriverError::new(
                    "unsupported operation",
                    "screenshots of a Tauri webview are not supported yet",
                ))
            }
            _ => Err(unknown(method, &command.join("/"))),
        }
    }

    fn find(&self, root: Option<&str>, body: &Value, all: bool) -> Result<Value, WebDriverError> {
        self.call(
            "find",
            json!({ "using": text(body, "using")?, "value": text(body, "value")?, "root": root, "all": all }),
        )
    }

    /// Waits until the page that a navigation started has loaded. The script
    /// answers `leaving` until the old page is gone, so an old page's
    /// `complete` is never taken for the new one.
    fn wait_until_loaded(&self) -> Result<Value, WebDriverError> {
        let started = Instant::now();
        while started.elapsed() < PAGE_LOAD_TIMEOUT {
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

fn timeouts() -> Value {
    json!({ "script": 30_000, "pageLoad": 60_000, "implicit": 0 })
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

    /// Records each operation and answers from a queue (`null` when empty).
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<(String, Value)>>,
        answers: Mutex<VecDeque<Result<Value, WebDriverError>>>,
    }

    impl Fake {
        fn answering(answers: Vec<Result<Value, WebDriverError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into()),
                ..Self::default()
            }
        }
    }

    impl Bridge for Fake {
        fn call(&self, op: &str, args: Value, _timeout: Duration) -> Result<Value, WebDriverError> {
            self.calls.lock().unwrap().push((op.to_owned(), args));
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(Value::Null))
        }
    }

    fn session(driver: &Driver<Fake>) -> String {
        let response = driver.handle("POST", "/t0k/session", None, b"{}");
        response.body["value"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned()
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
    fn navigation_waits_for_the_new_page() {
        let driver = Driver::new(
            Fake::answering(vec![
                Ok(Value::Null),
                Ok(json!("leaving")),
                Err(WebDriverError::new("timeout", "lost while unloading")),
                Ok(json!("loading")),
                Ok(json!("complete")),
            ]),
            "t0k",
        );
        let id = session(&driver);
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
