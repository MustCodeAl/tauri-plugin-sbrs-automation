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
> navigation, finding elements (CSS, XPath, link text, tag name), clicking,
> typing, reading text, attributes, properties and state, and running
> scripts. Screenshots, frames, alerts and cookies answer "unsupported" for
> now. Clicks and typing are DOM events, not OS-level input.

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

3. Allow its command in `src-tauri/capabilities/default.json`:

   ```json
   "permissions": ["core:default", "sbrs-automation:default"]
   ```

## Run a test

Start the app with a token of your choice (or read the random one it prints):

```bash
SBRS_AUTOMATION_TOKEN=secret cargo tauri dev --features e2e
```

Then point `BaseCase` at it:

```rust
use seleniumbase_rs::{BaseCase, BrowserConfig};

#[tokio::test]
async fn saving_a_profile() -> Result<(), seleniumbase_rs::SeleniumBaseError> {
    let config = BrowserConfig::default().with_webdriver_url("http://127.0.0.1:4445/secret");
    let mut sb = BaseCase::new(config).await?;
    sb.type_text("#name", "Work").await?;
    sb.click("#save").await?;
    sb.assert_text("#status", "Saved").await?;
    sb.quit().await
}
```

Any WebDriver client works the same way with `http://127.0.0.1:4445/secret`
as its server URL.

| Setting | Default |
|---|---|
| `SBRS_AUTOMATION` | on in debug builds, off in release; `1` turns it on, `0` off |
| `SBRS_AUTOMATION_PORT` | `4445` |
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
