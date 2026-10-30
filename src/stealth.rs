//! seleniumbase-rs's stealth in the app's own webviews (feature `stealth`).
//!
//! Unlike the rest of the plugin, this part is meant for a shipped app. It
//! needs no automation gate and opens nothing: no debugging port, no listener,
//! no WebDriver endpoint. The DevTools channels it uses live inside the app's
//! process.
//!
//! A [`StealthPlan`] has two parts, and each runtime takes what it can:
//!
//! 1. **The init script**, on every runtime: [`StealthScriptExt::stealth_script`]
//!    adds it to a webview while it is built, for the main frame and every
//!    frame, to run before the page's own scripts.
//! 2. **The overrides** (user agent and Client Hints, locale, time zone,
//!    geolocation, screen metrics and the rest), on Chromium runtimes: a
//!    [`DevToolsChannel`] sends them through the webview's in-process DevTools
//!    channel. [`cef`] connects one to CEF (`tauri-runtime-cef`) and
//!    [`webview2`] to WebView2 on Windows. WKWebView (macOS) and WebKitGTK
//!    (Linux) speak no DevTools Protocol, so they get the script only.
//!
//! The same channel drives the webview with seleniumbase-rs's
//! [`Page`] API ([`DevToolsChannel::page`]): `goto`, locators, human input.
//!
//! # Examples
//!
//! ```no_run
//! use tauri_plugin_sbrs_automation::stealth::{Fingerprint, OsType, StealthPlan, StealthScriptExt};
//!
//! let plan = StealthPlan::new(&Fingerprint::randomized(OsType::Windows, 7));
//! tauri::Builder::default().setup(move |app| {
//!     tauri::WebviewWindowBuilder::new(app, "browser", tauri::WebviewUrl::App("index.html".into()))
//!         .stealth_script(&plan)
//!         .build()?;
//!     Ok(())
//! });
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde_json::{Value, json};
use tauri::{Manager, Runtime};

pub use seleniumbase_rs::sb_cdp::{
    Browser, CdpInbox, CdpRequest, CdpTransport, Page, StealthPlan, TransportError,
};
pub use seleniumbase_rs::{Fingerprint, OsType, SeleniumBaseError};

/// Adds a [`StealthPlan`]'s init script to a webview window while it is built.
///
/// A `WebviewBuilder` (Tauri's `unstable` multi-webview API) takes the same
/// script: `.initialization_script_for_all_frames(plan.init_script())`.
pub trait StealthScriptExt: Sized {
    /// Runs `plan`'s init script in every document of the webview, frames
    /// included, before any script of the page.
    #[must_use]
    fn stealth_script(self, plan: &StealthPlan) -> Self;
}

impl<R: Runtime, M: Manager<R>> StealthScriptExt for tauri::WebviewWindowBuilder<'_, R, M> {
    fn stealth_script(self, plan: &StealthPlan) -> Self {
        self.initialization_script_for_all_frames(plan.init_script())
    }
}

/// The DevTools channel of one webview, inside the app's process, with
/// seleniumbase-rs on it.
///
/// Make one with [`cef::channel`], [`webview2::channel`], or [`new`](Self::new)
/// for a channel of your own. It reaches the webview's page only: it opens no
/// tabs, and it never listens on a port.
#[derive(Debug, Clone)]
pub struct DevToolsChannel {
    browser: Browser,
    inbox: CdpInbox,
}

impl DevToolsChannel {
    /// A channel whose commands leave through `transport`. What the webview
    /// answers goes to [`inbox`](Self::inbox).
    #[must_use]
    pub fn new(transport: impl CdpTransport) -> Self {
        let (browser, inbox) = Browser::over_channel(transport);
        Self { browser, inbox }
    }

    /// Where the webview's answers and events go in.
    #[must_use]
    pub fn inbox(&self) -> &CdpInbox {
        &self.inbox
    }

    /// The library's browser over this channel, for raw commands
    /// ([`Browser::execute`] reaches the webview's page).
    #[must_use]
    pub fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Sends `plan`'s overrides to the webview's page, in order, each after
    /// the answer to the one before.
    ///
    /// The init script is not among them: give it to the webview while it is
    /// built ([`StealthScriptExt::stealth_script`]). The overrides last as long
    /// as the page's DevTools session, which is the webview's life, and apply
    /// from the next request and document on, so apply them before the webview
    /// goes to the site that should see them.
    ///
    /// # Errors
    ///
    /// [`SeleniumBaseError::CdpDriver`] naming the command the webview refused
    /// (the ones before it stay applied), or
    /// [`SeleniumBaseError::BrowserDisconnected`] when the channel is gone.
    pub async fn apply(&self, plan: &StealthPlan) -> Result<(), SeleniumBaseError> {
        for (method, params) in plan.overrides() {
            self.browser.execute(method, params.clone()).await?;
        }
        Ok(())
    }

    /// The webview's page as a seleniumbase-rs [`Page`], to drive it.
    ///
    /// Preparing it installs the library's page helper script in the
    /// webview's documents, as for any tab the library drives; the stealth
    /// alone ([`apply`](Self::apply)) does not.
    ///
    /// # Errors
    ///
    /// What the webview answers to the preparing commands, or
    /// [`SeleniumBaseError::BrowserDisconnected`] when the channel is gone.
    pub async fn page(&self) -> Result<Page, SeleniumBaseError> {
        self.browser.default_page().await
    }

    /// Ends the channel, as when the webview is destroyed: what waits for an
    /// answer fails at once. What was set in the page stays.
    pub fn close(&self) {
        self.inbox.close();
    }
}

/// The answer to a command: the `result` object when it succeeded, the
/// browser's error message when not. An empty answer is an empty result.
fn outcome(succeeded: bool, answer: &[u8]) -> Result<Value, String> {
    let value = if answer.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice::<Value>(answer)
            .map_err(|error| format!("the webview answered with text that is not JSON: {error}"))?
    };
    if succeeded {
        Ok(value)
    } else {
        Err(value
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| value.to_string(), str::to_owned))
    }
}

/// An event's parameters; text that is not JSON is passed on as `null`.
fn event_params(params: &[u8]) -> Value {
    if params.iter().all(u8::is_ascii_whitespace) {
        return json!({});
    }
    serde_json::from_slice(params).unwrap_or(Value::Null)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub mod cef {
    //! The in-process DevTools channel of a CEF webview (Tauri's
    //! `tauri-runtime-cef`): `send_dev_tools_message` out,
    //! `on_dev_tools_protocol` back.
    //!
    //! CEF hands every DevTools message of a browser to every observer, the
    //! runtime's own included, so a request's id must come from the runtime's
    //! `allocate_devtools_message_id`. The bridge numbers each command with
    //! one, and takes only the results of the ids it allocated.
    //!
    //! This plugin pins Tauri `3.0.0-alpha.4` and builds without a runtime,
    //! so it cannot name `tauri-runtime-cef`'s types; the app joins the two
    //! with a few lines. With `tauri-runtime-cef` 3.0.0-alpha.6 (Tauri
    //! 3.0.0-alpha.5 or later), whose DevTools API is documented, and not
    //! run here:
    //!
    //! ```ignore
    //! use tauri_runtime_cef::{allocate_devtools_message_id, DevToolsProtocol, WebviewCefExt};
    //! use tauri_plugin_sbrs_automation::stealth::{cef, StealthPlan};
    //!
    //! let sender = webview.clone();
    //! let (channel, observer) = cef::channel(
    //!     move |message| Ok(sender.send_dev_tools_message(message)?),
    //!     || Ok(allocate_devtools_message_id()?),
    //! );
    //! webview.on_dev_tools_protocol(move |protocol| match protocol {
    //!     DevToolsProtocol::MethodResult { message_id, success, result } => {
    //!         observer.method_result(message_id, success, &result)
    //!     }
    //!     DevToolsProtocol::Event { method, params } => observer.event(&method, &params),
    //!     DevToolsProtocol::Message(_) => {}
    //! })?;
    //! tauri::async_runtime::spawn(async move { channel.apply(&plan).await });
    //! ```

    use super::{
        Arc, CdpInbox, CdpRequest, DevToolsChannel, HashMap, Mutex, TransportError, Value,
        event_params, json, lock, outcome,
    };

    /// CEF's request ids to the library's, for the commands in flight.
    type Ids = Arc<Mutex<HashMap<i32, u64>>>;

    /// A [`DevToolsChannel`] to a CEF browser, and the [`Observer`] its
    /// DevTools callback feeds.
    ///
    /// `send` hands one UTF-8 JSON message to the browser
    /// (`send_dev_tools_message`); `allocate_id` returns a request id from the
    /// runtime (`allocate_devtools_message_id`).
    pub fn channel<S, A>(send: S, allocate_id: A) -> (DevToolsChannel, Observer)
    where
        S: Fn(&[u8]) -> Result<(), TransportError> + Send + Sync + 'static,
        A: Fn() -> Result<i32, TransportError> + Send + Sync + 'static,
    {
        let ids: Ids = Arc::default();
        let in_flight = Arc::clone(&ids);
        let channel =
            DevToolsChannel::new(move |request: CdpRequest| -> Result<(), TransportError> {
                let native = allocate_id()?;
                let mut message = json!({
                    "id": native,
                    "method": request.method,
                    "params": request.params,
                });
                if let Some(session) = request.session_id {
                    message["sessionId"] = Value::String(session);
                }
                // Before sending: CEF may answer before `send` returns. An id
                // whose result never comes (CEF drops a malformed message
                // without one) stays until the channel is dropped.
                lock(&in_flight).insert(native, request.id);
                send(message.to_string().as_bytes()).inspect_err(|_refused| {
                    lock(&in_flight).remove(&native);
                })
            });
        let observer = Observer {
            inbox: channel.inbox().clone(),
            ids,
        };
        (channel, observer)
    }

    /// What the app's `on_dev_tools_protocol` callback passes on. Cheap to
    /// clone; every method returns at once, as a callback on CEF's UI thread
    /// must.
    #[derive(Debug, Clone)]
    pub struct Observer {
        inbox: CdpInbox,
        ids: Ids,
    }

    impl Observer {
        /// A `DevToolsProtocol::MethodResult`. `result` is the method's
        /// `result` object when `success`, its `error` object when not. A
        /// result for an id this channel did not allocate answers someone
        /// else's request and is ignored.
        pub fn method_result(&self, message_id: i32, success: bool, result: &[u8]) {
            let ours = lock(&self.ids).remove(&message_id);
            if let Some(id) = ours {
                self.inbox.respond(id, outcome(success, result));
            }
        }

        /// A `DevToolsProtocol::Event` of the webview's page.
        pub fn event(&self, method: &str, params: &[u8]) {
            self.inbox.event(method, event_params(params), None);
        }

        /// The webview is gone: commands still waiting fail at once.
        pub fn close(&self) {
            self.inbox.close();
        }
    }
}

pub mod webview2 {
    //! The in-process DevTools channel of WebView2 (Windows):
    //! `CallDevToolsProtocolMethod` out, its completion handler and the
    //! DevTools event receivers back.
    //!
    //! WebView2 answers each call through the handler given with it, and
    //! sends only the events subscribed to by name
    //! (`GetDevToolsProtocolEventReceiver`); [`EVENTS`] are the ones the
    //! library's `Page` listens for. It may also run calls out of order, so the
    //! library waits for each answer where order matters.
    //!
    //! This plugin builds without a runtime and cannot name WebView2's COM
    //! types; the app makes the calls (with `webview2-com`, through
    //! `Webview::with_webview` on the UI thread) and passes the results on.
    //! Not built or run here:
    //!
    //! ```ignore
    //! let channel = webview2::channel(move |call: webview2::Call| {
    //!     webview.with_webview(move |platform| unsafe {
    //!         // `platform.controller().CoreWebView2()?.CallDevToolsProtocolMethod(
    //!         //     &call.method, &call.params_json, &handler)`, where the
    //!         // completion handler runs
    //!         // `call.reply.complete(error_code.is_ok(), &return_object_as_json)`.
    //!     })?;
    //!     Ok(())
    //! });
    //! // For each name in `webview2::EVENTS`, a DevTools event receiver whose
    //! // handler runs `webview2::event(channel.inbox(), name, &json, session_id)`.
    //! ```

    use super::{
        Arc, CdpInbox, CdpRequest, DevToolsChannel, OnceLock, TransportError, event_params, outcome,
    };

    /// The events the library's `Page` listens for: subscribe to each with
    /// `GetDevToolsProtocolEventReceiver`. Add the ones your own code waits
    /// for.
    pub const EVENTS: [&str; 14] = [
        "Page.loadEventFired",
        "Page.frameNavigated",
        "Page.javascriptDialogOpening",
        "Page.javascriptDialogClosed",
        "Page.downloadWillBegin",
        "Page.downloadProgress",
        "Runtime.exceptionThrown",
        "Runtime.consoleAPICalled",
        "Network.requestWillBeSent",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
        "Fetch.requestPaused",
        "Fetch.authRequired",
    ];

    /// One `CallDevToolsProtocolMethod` for the app to make.
    #[derive(Debug)]
    pub struct Call {
        /// `methodName`, such as `Emulation.setTimezoneOverride`.
        pub method: String,
        /// `parametersAsJson`.
        pub params_json: String,
        /// The session of an attached target, for
        /// `CallDevToolsProtocolMethodForSession`; `None` for the page.
        pub session_id: Option<String>,
        /// Completes the call from its completion handler.
        pub reply: Reply,
    }

    /// Completes one [`Call`].
    #[derive(Debug)]
    pub struct Reply {
        inbox: CdpInbox,
        id: u64,
    }

    impl Reply {
        /// The completion handler ran: `succeeded` is whether its error code
        /// was a success, `return_object_json` the JSON it gave, the method's
        /// result or, on failure, the error.
        pub fn complete(self, succeeded: bool, return_object_json: &str) {
            self.inbox
                .respond(self.id, outcome(succeeded, return_object_json.as_bytes()));
        }
    }

    /// A [`DevToolsChannel`] whose commands go to `call`, which must make the
    /// WebView2 call (on the UI thread) and return without waiting for it.
    pub fn channel<C>(call: C) -> DevToolsChannel
    where
        C: Fn(Call) -> Result<(), TransportError> + Send + Sync + 'static,
    {
        let inbox: Arc<OnceLock<CdpInbox>> = Arc::default();
        let replies = Arc::clone(&inbox);
        let channel =
            DevToolsChannel::new(move |request: CdpRequest| -> Result<(), TransportError> {
                let inbox = replies.get().ok_or("the channel is not ready")?.clone();
                call(Call {
                    method: request.method,
                    params_json: request.params.to_string(),
                    session_id: request.session_id,
                    reply: Reply {
                        inbox,
                        id: request.id,
                    },
                })
            });
        // Set before the channel is handed out, so before any command.
        let _ = inbox.set(channel.inbox().clone());
        channel
    }

    /// An event from a DevTools event receiver: its name, its
    /// `ParameterObjectAsJson` and its `SessionId` (`None` or empty for the
    /// page).
    pub fn event(
        inbox: &CdpInbox,
        name: &str,
        parameter_object_json: &str,
        session_id: Option<&str>,
    ) {
        inbox.event(
            name,
            event_params(parameter_object_json.as_bytes()),
            session_id.filter(|session| !session.is_empty()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    fn plan() -> StealthPlan {
        StealthPlan::new(&Fingerprint::randomized(OsType::Windows, 11))
    }

    /// A fake CEF browser: what `send` received, and ids from 1000 up.
    struct FakeCef {
        sent: Arc<Mutex<Vec<Value>>>,
        observer: cef::Observer,
        channel: DevToolsChannel,
    }

    fn fake_cef() -> FakeCef {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let next = Arc::new(AtomicUsize::new(1000));
        let log = Arc::clone(&sent);
        let (channel, observer) = cef::channel(
            move |message: &[u8]| {
                lock(&log).push(serde_json::from_slice(message)?);
                Ok(())
            },
            move || Ok(i32::try_from(next.fetch_add(1, Ordering::SeqCst))?),
        );
        FakeCef {
            sent,
            observer,
            channel,
        }
    }

    impl FakeCef {
        /// Waits until `count` messages were sent, and returns them.
        async fn sent(&self, count: usize) -> Vec<Value> {
            for _ in 0..200 {
                if lock(&self.sent).len() >= count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            lock(&self.sent).clone()
        }
    }

    #[tokio::test]
    async fn a_cef_message_is_the_protocol_text_with_the_runtimes_id() {
        let cef = fake_cef();
        let browser = cef.channel.browser().clone();
        let call = tokio::spawn(async move {
            browser
                .execute(
                    "Emulation.setTimezoneOverride",
                    json!({ "timezoneId": "Asia/Tokyo" }),
                )
                .await
        });

        let sent = cef.sent(1).await;
        assert_eq!(
            sent[0],
            json!({
                "id": 1000,
                "method": "Emulation.setTimezoneOverride",
                "params": { "timezoneId": "Asia/Tokyo" },
            }),
            "no sessionId: the channel reaches the webview's page"
        );
        cef.observer.method_result(1000, true, b"{}");
        assert_eq!(call.await.unwrap().unwrap(), json!({}));
    }

    #[tokio::test]
    async fn a_cef_result_for_an_id_it_did_not_allocate_is_someone_elses() {
        let cef = fake_cef();
        let browser = cef.channel.browser().clone();
        let call = tokio::spawn(async move { browser.execute("Page.enable", json!({})).await });
        cef.sent(1).await;

        cef.observer.method_result(7, true, br#"{"wrong":true}"#);
        cef.observer.method_result(1000, true, b"");

        assert_eq!(
            call.await.unwrap().unwrap(),
            json!({}),
            "an empty result is {{}}"
        );
    }

    #[tokio::test]
    async fn a_failed_cef_result_carries_the_error_message() {
        let cef = fake_cef();
        let browser = cef.channel.browser().clone();
        let call = tokio::spawn(async move {
            browser
                .execute(
                    "Emulation.setTimezoneOverride",
                    json!({ "timezoneId": "Mars/Base" }),
                )
                .await
        });
        cef.sent(1).await;

        cef.observer.method_result(
            1000,
            false,
            br#"{"code":-32000,"message":"Invalid timezone ID"}"#,
        );

        let error = call.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("Emulation.setTimezoneOverride"), "{error}");
        assert!(error.contains("Invalid timezone ID"), "{error}");
    }

    #[tokio::test]
    async fn apply_sends_the_overrides_in_order_one_answer_at_a_time() {
        let cef = fake_cef();
        let plan = plan();
        let expected: Vec<(String, Value)> = plan
            .overrides()
            .map(|(method, params)| (method.to_owned(), params.clone()))
            .collect();
        let applying = {
            let channel = cef.channel.clone();
            let plan = plan.clone();
            tokio::spawn(async move { channel.apply(&plan).await })
        };

        for (index, _) in expected.iter().enumerate() {
            let sent = cef.sent(index + 1).await;
            assert_eq!(sent.len(), index + 1, "the next waits for this answer");
            let id = i32::try_from(1000 + index).unwrap();
            cef.observer.method_result(id, true, b"{}");
        }
        applying.await.unwrap().unwrap();

        let sent: Vec<(String, Value)> = lock(&cef.sent)
            .iter()
            .map(|message| {
                (
                    message["method"].as_str().unwrap().to_owned(),
                    message["params"].clone(),
                )
            })
            .collect();
        assert_eq!(sent, expected);
        assert!(
            sent.iter()
                .all(|(method, _)| method != "Page.addScriptToEvaluateOnNewDocument"),
            "the script goes in as the webview's initialization script"
        );
        assert!(
            sent.iter()
                .any(|(method, params)| method == "Network.setUserAgentOverride"
                    && params.get("userAgentMetadata").is_some()),
            "the user agent comes with its Client Hints"
        );
    }

    #[tokio::test]
    async fn a_refused_override_stops_apply_and_names_it() {
        let cef = fake_cef();
        let plan = plan();
        let applying = {
            let channel = cef.channel.clone();
            tokio::spawn(async move { channel.apply(&plan).await })
        };
        let first = cef.sent(1).await;

        cef.observer.method_result(
            1000,
            false,
            br#"{"message":"'Network.enable' wasn't found"}"#,
        );

        let error = applying.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("Network.enable"), "{error}");
        assert_eq!(first[0]["method"], "Network.enable");
        assert_eq!(lock(&cef.sent).len(), 1, "nothing after the refused one");
    }

    #[tokio::test]
    async fn a_cef_event_reaches_the_page_and_ends_a_navigation() {
        let cef = fake_cef();
        let observer = cef.observer.clone();
        let sent = Arc::clone(&cef.sent);
        // The webview's side: answers everything, and loads at once.
        let agent = tokio::spawn(async move {
            let mut answered = 0;
            loop {
                let pending: Vec<Value> = lock(&sent)[answered..].to_vec();
                for message in pending {
                    let id = i32::try_from(message["id"].as_i64().unwrap()).unwrap();
                    if message["method"] == "Page.navigate" {
                        observer.event("Page.loadEventFired", br#"{"timestamp":1}"#);
                        observer.method_result(id, true, br#"{"frameId":"F","loaderId":"L"}"#);
                    } else {
                        observer.method_result(id, true, b"{}");
                    }
                    answered += 1;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });

        let page = cef.channel.page().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), page.goto("https://example.com/"))
            .await
            .expect("the load event ends the wait")
            .unwrap();
        agent.abort();

        assert!(
            lock(&cef.sent)
                .iter()
                .all(|message| message.get("sessionId").is_none())
        );
    }

    #[tokio::test]
    async fn a_closed_cef_channel_fails_what_waits() {
        let cef = fake_cef();
        let browser = cef.channel.browser().clone();
        let call = tokio::spawn(async move { browser.execute("Page.enable", json!({})).await });
        cef.sent(1).await;

        cef.observer.close();

        let outcome = tokio::time::timeout(Duration::from_secs(2), call)
            .await
            .expect("closing does not wait for the timeout")
            .unwrap();
        assert!(matches!(
            outcome,
            Err(SeleniumBaseError::BrowserDisconnected { .. })
        ));
        assert!(!cef.channel.browser().is_connected());
    }

    #[tokio::test]
    async fn a_cef_send_that_fails_fails_the_command_and_forgets_its_id() {
        let (channel, observer) =
            cef::channel(|_: &[u8]| Err("the browser is gone".into()), || Ok(5));

        let error = channel
            .browser()
            .execute("Page.enable", json!({}))
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("the browser is gone"), "{error}");
        // A late result for that id is not mistaken for anything.
        observer.method_result(5, true, b"{}");
    }

    #[tokio::test]
    async fn webview2_calls_carry_the_method_and_parameters_and_complete_by_reply() {
        let calls = Arc::new(Mutex::new(Vec::<webview2::Call>::new()));
        let queue = Arc::clone(&calls);
        let channel = webview2::channel(move |call| {
            lock(&queue).push(call);
            Ok(())
        });
        let browser = channel.browser().clone();
        let pending = tokio::spawn(async move {
            browser
                .execute("Emulation.setLocaleOverride", json!({ "locale": "de-DE" }))
                .await
        });
        let call = loop {
            if let Some(call) = lock(&calls).pop() {
                break call;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        };

        assert_eq!(call.method, "Emulation.setLocaleOverride");
        assert_eq!(
            serde_json::from_str::<Value>(&call.params_json).unwrap(),
            json!({ "locale": "de-DE" })
        );
        assert_eq!(call.session_id, None);
        call.reply.complete(true, "{}");
        assert_eq!(pending.await.unwrap().unwrap(), json!({}));
    }

    #[tokio::test]
    async fn a_failed_webview2_call_carries_the_error_it_returned() {
        let channel = webview2::channel(|call| {
            call.reply.complete(
                false,
                r#"{"code":-32601,"message":"'Foo.bar' wasn't found"}"#,
            );
            Ok(())
        });

        let error = channel
            .browser()
            .execute("Foo.bar", json!({}))
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("'Foo.bar' wasn't found"), "{error}");
    }

    #[tokio::test]
    async fn webview2_events_reach_listeners_as_the_pages() {
        let channel = webview2::channel(|_| Ok(()));
        let mut events = channel.browser().events();

        webview2::event(
            channel.inbox(),
            "Page.loadEventFired",
            r#"{"timestamp":2}"#,
            Some(""),
        );
        webview2::event(
            channel.inbox(),
            "Network.loadingFinished",
            "{}",
            Some("FRAME"),
        );

        let first = events.next().await.unwrap();
        assert_eq!(first.method, "Page.loadEventFired");
        assert_eq!(first.params["timestamp"], 2);
        let second = events.next().await.unwrap();
        assert_eq!(second.session_id.as_deref(), Some("FRAME"));
    }

    #[test]
    fn the_webview2_events_include_the_load_event_a_navigation_waits_for() {
        assert!(webview2::EVENTS.contains(&"Page.loadEventFired"));
        let mut unique = webview2::EVENTS.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), webview2::EVENTS.len());
    }

    #[test]
    fn answers_that_are_not_json_are_errors_and_events_pass_as_null() {
        assert!(outcome(true, b"not json").is_err());
        assert_eq!(outcome(false, b"{}"), Err("{}".to_owned()));
        assert_eq!(event_params(b"not json"), Value::Null);
        assert_eq!(event_params(b""), json!({}));
    }
}
