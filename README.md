# tauri-plugin-sbrs-automation

Test a [Tauri](https://tauri.app) app's real window with
[seleniumbase-rs](https://github.com/MustCodeAl/seleniumbase-rs) (`BaseCase`
and the `sbrs` CLI) on **macOS, Windows and Linux**.

Tauri apps use the system webview. Windows (WebView2) and Linux (WebKitGTK)
have WebDriver drivers, but **macOS (WKWebView) has none**. This plugin runs a
small W3C WebDriver endpoint inside your app, so the same tests drive the app
everywhere.

> [!NOTE]
> Early (0.1, Tauri 3 alpha). It serves the core of WebDriver: sessions,
> timeouts, navigation, finding elements (CSS, XPath, link text, tag name),
> clicking, typing, reading text, attributes, properties and state, running
> scripts (sync and async) and the window's rectangle (read, move, resize,
> maximize, minimize, full screen). Clicks and typing are DOM events, not
> OS-level input, so the mouse and keyboard are never used.
>
> **Verified on a real app** (2026-10-15, macOS, Tauri `3.0.0-alpha.4` with
> the wry runtime): the `tauri-profile-manager` example of seleniumbase-rs is
> driven by `BaseCase` (thirtyfour 0.37) in an end-to-end test that starts a
> session, reads the title, finds elements by CSS selector, waits for them to
> be displayed and enabled, clicks, clears and types into inputs, reads and
> asserts text, reads and resizes the window, and ends the session; sync and
> async scripts were run on the same app with raw WebDriver requests. Windows
> and Linux, and the XPath, link-text and tag-name strategies, are covered by
> unit tests only.
>
> Not served: screenshots (`unsupported operation`); frames, alerts, cookies,
> actions (hover, drag), shadow roots, and opening or closing windows
> (`unknown command`). The endpoint drives one window, `main` (or the only
> one). Cookies are left out on purpose: on macOS the app's
> page is `tauri://localhost`, where WKWebView keeps no cookies at all
> (`document.cookie` reads back empty after a write).

## Set up your app

1. Add the plugin, best behind a feature so release builds never have it:

   ```toml
   [features]
   e2e = ["dep:tauri-plugin-sbrs-automation"]

   [dependencies]
   tauri-plugin-sbrs-automation = { git = "https://github.com/MustCodeAl/tauri-plugin-sbrs-automation", optional = true }
   ```

2. Register it:

   ```rust
   let mut builder = tauri::Builder::default();
   #[cfg(feature = "e2e")]
   {
       builder = builder.plugin(tauri_plugin_sbrs_automation::init());
   }
   ```

3. Allow its command. With the plugin behind a feature, put the permission in
   a capability of its own, `src-tauri/capabilities/e2e/automation.json`:

   ```json
   {
     "identifier": "e2e-automation",
     "windows": ["main"],
     "permissions": ["sbrs-automation:default"]
   }
   ```

   and let `build.rs` read that folder only with the feature. A capability
   that names `sbrs-automation:default` while the plugin is not compiled in
   fails the build (`Permission sbrs-automation:default not found`), so it
   cannot go in `default.json` next to an optional plugin:

   ```rust
   fn main() {
       let capabilities = if std::env::var_os("CARGO_FEATURE_E2E").is_some() {
           "./capabilities/**/*"
       } else {
           "./capabilities/*.json"
       };
       println!("cargo:rerun-if-changed=capabilities");
       let attributes = tauri_build::Attributes::new().capabilities_path_pattern(capabilities);
       if let Err(error) = tauri_build::try_build(attributes) {
           panic!("{error:#}");
       }
   }
   ```

   If the plugin is always compiled in, `"sbrs-automation:default"` can simply
   join the permissions of `capabilities/default.json`.

4. On Tauri 3, the app must name its runtime, or it stops at start with
   `RuntimeNotConfigured`: depend on `tauri-runtime-wry` (the same alpha) and
   call `.runtime(tauri_runtime_wry::Wry::default())` on the builder.

## Run a test

Start the app with a token of your choice (or read the random one it prints):

```bash
SBRS_AUTOMATION_TOKEN=secret cargo tauri dev --features e2e
```

Then point `BaseCase` at it. The URL ends with `/`: WebDriver clients join
each command path (`session`, ...) to the server URL as a relative URL, so
without the slash the token is replaced and every request is refused with
`the URL does not start with the token`.

```rust
use seleniumbase_rs::{BaseCase, BrowserConfig};

#[tokio::test]
async fn saving_a_profile() -> Result<(), seleniumbase_rs::SeleniumBaseError> {
    let config = BrowserConfig::default().with_webdriver_url("http://127.0.0.1:4445/secret/");
    let mut sb = BaseCase::new(config).await?;
    sb.type_text("#name", "Work").await?;
    sb.click("#save").await?;
    sb.assert_text("#status", "Saved").await?;
    sb.quit().await
}
```

Any WebDriver client works the same way with `http://127.0.0.1:4445/secret/`
as its server URL. A new session waits until the app's window has loaded its
page, so a test can start one as soon as the endpoint answers
`GET /secret/status`. For a test that starts the app itself, see
`examples/tauri-profile-manager/src-tauri/tests/e2e_automation.rs` in
seleniumbase-rs.

| Setting | Default |
|---|---|
| `SBRS_AUTOMATION` | on in debug builds, off in release; `1` turns it on, `0` off |
| `SBRS_AUTOMATION_PORT` | `4445`; if the port is taken (a second instance, a Selenium container), the app runs without the endpoint and says so on stderr |
| `SBRS_AUTOMATION_TOKEN` | random, printed when the app starts |

## How it works

```text
BaseCase / sbrs ──HTTP (WebDriver)──▶ plugin server (127.0.0.1, inside the app)
                                          │ eval
                                          ▼
                                   bridge.js in the webview ──IPC reply──▶ plugin
```

The server turns each WebDriver command into an operation of an injected script
(`src/bridge.js`), which runs it in the page and sends the result back through
Tauri's IPC.

## Security

The endpoint can run any JavaScript in your app's window; that is what
WebDriver's "execute script" is for. So it is guarded:

- on in debug builds only, unless you set `SBRS_AUTOMATION=1`;
- listens on `127.0.0.1` only;
- every URL must start with the secret token;
- requests from web pages (any request with an `Origin` header) are refused.

Keep it behind a Cargo feature that release builds do not enable.

## License

MIT.
