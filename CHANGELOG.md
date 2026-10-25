# Changelog

## Unreleased

- Add optional `cdp` configuration for app-owned CEF/WebView2, reusing the
  canonical seleniumbase-rs stealth argument generator through a pinned Git
  dependency (Rust 1.99). Respect the existing automation gate and bind CDP to
  loopback. Environment setup is explicitly unsafe and must precede threads.
- Add authenticated `GET /<token>/cdp` discovery with the existing Origin and
  token guards. Keep the WebDriver routes, status response and page bridge intact.
