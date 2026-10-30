# Changelog

## Unreleased

- Add the optional `stealth` feature: seleniumbase-rs's fingerprint stealth in
  the app's own webviews, for a shipped app. `stealth::StealthScriptExt`
  adds a `StealthPlan`'s init script to a `WebviewWindowBuilder` for every
  frame (all runtimes); a `stealth::DevToolsChannel` sends the plan's
  overrides in order through the webview's in-process DevTools channel
  (`apply`) and gives a seleniumbase-rs `Page` for the webview (`page`).
  `stealth::cef::channel` bridges CEF's `send_dev_tools_message` /
  `on_dev_tools_protocol` (request ids from the runtime's allocator) and
  `stealth::webview2::channel` bridges WebView2's
  `CallDevToolsProtocolMethod` and event receivers; the app joins them to its
  runtime (README). No port, no listener, no automation gate. The feature
  depends on seleniumbase-rs from git (pinned commit) and needs Rust 1.99;
  a default build and the `cdp` feature are unchanged. Tested against fake
  channels only; not run in a real CEF or WebView2 app.

- Add the optional `cdp` feature: launch settings (`cdp::CdpStealthConfig`)
  for an app-owned CEF or WebView2 that open Chromium's debugging port on
  `127.0.0.1`. It has no dependencies and does not raise the minimum Rust
  version. `chromium_args()` and `apply_env()` produce nothing unless the
  automation gate is on, so a release build opens no port; `apply_env` is
  `unsafe` and must run before any thread, and refuses an environment that
  already holds a `--remote-debugging-*`, `--remote-allow-origins` or
  `--edge-webview-switches` switch in any spelling.
- **Security:** the debugging socket has **no authentication**. Any process on
  the machine can connect and run script in the app's webviews, so it must be
  off in release builds. Only the discovery route is token protected.
- Add `GET /<token>/cdp` discovery, guarded like the other routes (secret
  token, loopback, no request with an `Origin`). It answers `cdpEnabled`,
  `cdpPort`, `cdpUrl` and `cdpState` (`disabled`, `unreachable`, `ready`) and
  names the port only while `GET /json/version` there answers like Chromium.
  That does not prove the browser is the app's own. The WebDriver routes,
  status response and page bridge are unchanged.
- Without `SBRS_CDP_PORT` the launch settings pick a free loopback port
  instead of a fixed 9222; a port that is asked for and in use enables
  nothing. `CdpStealthConfig::debugging_port` is an `Option<u16>`, and
  `CdpStealthConfig::port()` returns the port for an app that configures CEF's
  remote debugging itself.
- The plugin reads the debugging port once, at start, from the launch settings
  (`cdp::enabled_port`, `Driver::with_cdp_port`), not from the environment, and
  `apply_env` no longer exports `SBRS_CDP_PORT`. It still sets
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which WebView2 needs, so child
  processes inherit the switches (documented).
- Stealth arguments are opt-in (`stealth_args(true)`) and keep five flags that
  change scheduling, a dialog and `navigator.webdriver` only. seleniumbase-rs's
  longer list turned off site isolation, the popup blocker, phishing
  detection, component updates and the encryption of stored data in the app's
  own browser, and is not used.
- `SBRS_AUTOMATION` fails closed: only `1`, `true` and `on` (any case) turn the
  plugin on; any other value, a typo included, turns it off. Unset still
  follows the build profile.
- The URL token is compared in constant time.
