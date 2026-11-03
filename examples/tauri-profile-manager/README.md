# Tauri Profile Manager

A desktop multi-profile browser manager built with [Tauri](https://tauri.app), `seleniumbase-rs` and this repository's plugin, `tauri-plugin-sbrs-automation`. Each profile is its own browser identity, driven through a dedicated Docker WebDriver container or a Chrome the app launches itself (Pure CDP), or opened in a window of the app itself with the profile's fingerprint (the plugin's `stealth` feature).

It shows two of the plugin's uses:

- **Stealth inside the app** ([Open in an app window](#open-in-an-app-window)): `stealth::StealthScriptExt` gives a webview window of the app a fingerprint's init script. Fit for a shipped build: no port and no listener.
- **A test driving the app** ([End-to-end test](#end-to-end-test)): `init()` serves the app's window over WebDriver, so `BaseCase` drives it. Only the `e2e` feature registers it.

## Architecture

- **Tauri desktop app** — Rust backend + vanilla HTML/JS frontend, on Tauri 3
  (`3.0.0-alpha.4`, pinned exactly, since alphas may break each other). See
  [Tauri versions](#tauri-versions).
- **Profile store** — an encrypted vault (`profiles.vault`) in the OS app-data directory. See [Profile storage](#profile-storage).
- **Two engines** — each profile picks one. See [Engines](#engines).
- **App windows** — any profile with a fingerprint also opens in a window of the app. See [Open in an app window](#open-in-an-app-window).
  - **WebDriver** — `seleniumbase-rs::BaseCase` connects to a `selenium/standalone-chrome` container with per-profile `user_agent`, `proxy`, `locale`, and `headless` settings.
  - **Pure CDP** — Chrome is launched directly and driven over the DevTools Protocol. No Docker, no WebDriver.

## Engines

A profile's `engine` is `WebDriver` (the default, and what every profile saved by an earlier version uses) or `PureCdp`.

**Pure CDP** needs Google Chrome or Chromium installed and nothing else: no container and no WebDriver URL. Each launch starts its own Chrome and, inside it, an isolated browser context for the profile, so its cookies and storage are shared with nothing else. Closing the session disposes the context and closes the browser.

What a Pure CDP launch applies, in this order:

1. The profile's **proxy**, on the profile's browser context. A proxy password is answered for that context's tabs only; it is never put on a command line or sent as a protocol parameter.
2. The profile's **fingerprint** (and an imported profile's fingerprint): its script is installed to run before any page script, and the protocol overrides it recommends (user agent and client hints, locale, timezone, screen, permissions) are set. A `Proxy-Authorization` header the fingerprint might ask for is deliberately not sent, because it would reach every site.
3. The profile's **locale**, as both `Intl` locale and `Accept-Language`, so `navigator.language` and `Intl` agree. A headless Chrome's user agent no longer names itself.
4. The profile's **geolocation** (and the permission for pages to read it) and **screen size**.
5. The profile's saved **cookies**.
6. The **start URL**, last.

If any step fails, the launch fails with the step named (`LAUNCH_FAILED`, `OVERRIDE_FAILED`, `COOKIE_FAILED` or `OPEN_FAILED` over REST) and the browser is closed again.

Limits worth knowing:

- Only the first tab is set up. A popup or a tab the page opens does not get the fingerprint script.
- The browser context is discarded on close, so cookies a site sets are not kept unless you export them.
- A script run on a Pure CDP session is evaluated as a JavaScript *expression*; a WebDriver script is a function body and needs `return`.
- Starting a Pure CDP profile over REST returns the browser's DevTools port and WebSocket address (`port`, `ws_endpoint`) for a Playwright- or Puppeteer-style client to attach to. Anyone who can connect to that address controls the browser, so treat it as sensitive.
- Chrome is headed unless the profile's **Headless** box is ticked, on every operating system, so a machine with no display needs it ticked.

### Random identities

`Randomize` on a profile card (or `POST /api/v1/profiles/:id/randomize`) gives the profile a new fingerprint from `Fingerprint::randomized`: a user agent, client hints, platform, screen, GPU and the rest that agree with one another. It replaces whatever fingerprint the profile had, so the window asks first when there is one.

The identity claims *this machine's* operating system unless the request names another, because a page can still see the real graphics stack and fonts behind a different claim. The same operating system and seed always give the same identity. Without a seed a fresh 53-bit one is drawn (53 bits so JavaScript can show and return it exactly), and the response reports it.

The claimed Chrome version comes from a short window of recent releases. Pair it with a browser of that era, or edit the user agent, client hints and core version together.

## Open in an app window

`Open in app` on a profile card opens the profile in a new window of the app. The app runs on CEF (`tauri-runtime-cef`) on macOS, Windows and Linux, so the window is Chromium with a DevTools channel inside the app's process: no container, no WebDriver, no debugging port. It goes to the card's start URL, or `about:blank` without one; only `http` and `https` pages open.

The window carries the profile's fingerprint (its own, else its imported profile's), through the plugin's stealth:

```rust
use tauri_plugin_sbrs_automation::stealth::{StealthPlan, StealthScriptExt};

WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
    .incognito(true)
    .stealth_script(&StealthPlan::new(&fingerprint))
    .build()?;
```

`src-tauri/src/app_window.rs` is the whole of it; the `open_profile_window` command calls it.

What the window gets, and what it does not:

- **The fingerprint's init script**, in every document and frame, before the page's own scripts, so what they read (`navigator`, `screen` and the rest) is the fingerprint's. Every runtime takes it.
- **The protocol overrides** (user agent and Client Hints, locale, time zone, geolocation, screen), sent over the window's CEF DevTools channel with the plugin's `stealth::cef::channel`, then the profile's **cookies** (`Network.setCookies`). The window opens at `about:blank` and goes to the page only after both are in; a window they cannot reach is closed instead.
- **The profile's proxy** (`http`, `https` or `socks5`) and a **CEF profile of its own** (`data_store_identifier`), so windows share no storage with each other or the app's window. A proxy password is not answered.
- **No access to the app.** No capability names a profile window, so the site in it cannot call the app's commands.

A profile without a fingerprint is refused: `Randomize` gives it one.

## Profile storage

Profiles, tags and folders live in `profiles.vault`, an embedded database in which every document is sealed with AES-256-GCM under a key derived from a passphrase. The file alone shows only the ids and sizes of what it holds, not cookies, proxy credentials or names.

**Where the passphrase comes from**, in this order:

1. `SB_PROFILE_PASSPHRASE`, if set. Use this on a machine with no keychain, such as a headless server or CI.
2. The operating system's keychain (Keychain on macOS, Credential Manager on Windows, the Secret Service elsewhere). The first run generates a random 256-bit passphrase and stores it there; later runs read it back, and nobody types anything.

**Import from the old JSON files.** If `profiles.json`, `tags.json` or `folders.json` from an earlier version are in the app-data directory, they are imported on startup. Every document is read back from the vault and compared with its source *before* any file is deleted. If anything differs, or a file cannot be read, the files are left exactly as they were and the window says why. If a file imports but cannot be deleted, the window warns you to delete it yourself, and the next start removes it rather than importing it over your later changes.

**When the vault cannot be opened** — a passphrase that does not match, a missing keychain item for an existing vault, a keychain that fails — the app does not guess. It shows the reason in a banner, lists no profiles, refuses changes (`503 STORAGE_UNAVAILABLE` over REST) and writes nothing, so a problem with the passphrase can never look like an empty list or overwrite the vault. It never generates a new passphrase for a vault that already exists, since that would only lock you out.

What this protects against, and what it does not:

- It protects the *file*: a backup, a synced folder, a stolen disk.
- It does not protect against code running as you while the app is open, because the key has to be in memory to be used. `SB_PROFILE_PASSPHRASE` is also visible to other processes of the same user, as environment variables are.
- **Losing the passphrase loses the profiles.** If you use the keychain, back up your keychain; to move the vault to another machine, set `SB_PROFILE_PASSPHRASE` to the passphrase it was created with.
- Every change is written to the vault before the app's in-memory list, so a failed write leaves the two in agreement.

## Run the browser grid (WebDriver profiles)

Skip this if you only use Pure CDP profiles.

```bash
docker compose up -d
```

This exposes:

| Container | WebDriver | VNC (noVNC) |
|-----------|-----------|-------------|
| browser-a | http://localhost:4444 | http://localhost:7900 |
| browser-b | http://localhost:4445 | http://localhost:7901 |
| browser-c | http://localhost:4446 | http://localhost:7902 |

VNC password is `secret` by default.

## Start the Tauri app

Needs Rust 1.99 or newer, the version `seleniumbase-rs` requires (Tauri 3
needs 1.95), and Node LTS (`brew install node`, `winget install
OpenJS.NodeJS.LTS`). On Windows it uses WebView2, which comes with Windows 10
and 11. On Debian and Ubuntu:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev libssl-dev libx11-dev libxdo-dev
```

The Tauri CLI comes from npm, pinned to the same alpha:

```bash
npm ci
npx tauri dev
```

## Build a release bundle

```bash
npx tauri build
```

## End-to-end test

`src-tauri/tests/e2e_automation.rs` drives the app's real window with
`seleniumbase-rs`'s `BaseCase`: it reads the title, finds the profile list,
creates a WebDriver profile through the dialog, filters the list by typing in
the search box, adds a folder, and reads and resizes the window. The window is
served over WebDriver by
[`tauri-plugin-sbrs-automation`](https://github.com/MustCodeAl/tauri-plugin-sbrs-automation),
which only the `e2e` feature adds to the app.

```bash
cd src-tauri
cargo test --features e2e --test e2e_automation -- --ignored
```

The test builds the app with the feature, starts it with `SBRS_AUTOMATION=1`,
`SBRS_AUTOMATION_TOKEN=e2etoken` and `SBRS_AUTOMATION_PORT=4545`, waits for
`http://127.0.0.1:4545/e2etoken/status`, drives the window and stops the app,
also when a step fails. It gives the app a temporary `HOME` and a passphrase of
its own (`SB_PROFILE_PASSPHRASE`), so your vault and keychain are not touched;
that is also why it runs on macOS and Linux only (on Windows the app-data folder
does not follow `HOME`). The window opens on screen while it runs, but clicks
and typing are DOM events, so the mouse and keyboard stay yours. Ports 4545 and
45001 (the REST API) must be free.

- `Cargo.toml` takes the plugin from this repository (`path = "../../.."`);
  an app of your own takes it by `git`. The plugin is always built in, for
  its `stealth` feature; the `e2e` feature only registers `init()`.
- `seleniumbase-rs` comes by git at the `v0.7.0` tag, the one the plugin's
  `stealth` feature pins, so the app and the plugin share one copy of the
  library and its types (`Fingerprint`, `StealthPlan`).
- The permission the plugin's script needs, `sbrs-automation:default`, is in
  `capabilities/e2e/automation.json`. `build.rs` reads that folder only with the
  `e2e` feature, so other builds do not grant it.
- `BaseCase` is pointed at `http://127.0.0.1:4545/e2etoken/`, with a trailing
  slash. The WebDriver client joins each command path to the server URL as a
  relative URL, so without the slash the token would be replaced and every
  request refused.

It passed on macOS on 2026-10-15, in about 6 seconds once built.

## Tauri versions

The app builds on Tauri `3.0.0-alpha.4` (`tauri`, `tauri-runtime-wry`
`3.0.0-alpha.4`, `tauri-build` `3.0.0-alpha.3`, `tauri-plugin-opener`
`3.0.0-alpha.2`, `@tauri-apps/cli` `3.0.0-alpha.4`). On macOS its 121 tests
pass (one more is ignored; 2026-11-02, in this repository). Before the move,
in seleniumbase-rs, `npx tauri build --debug --bundles app` made the `.app`, and
the window opened and was driven by the [end-to-end test](#end-to-end-test)
(2026-10-15); neither has been run here yet.

Moving from Tauri 2 needed one code change: Tauri 3 starts no runtime unless
the app names one, so `run` passes `tauri_runtime_wry::Wry::default()` to
`Builder::runtime` (without it the app stops at once with
`RuntimeNotConfigured`). The frontend's `window.__TAURI__.core.invoke`, the
`core:default` and `opener:default` permissions and the configuration all
carried over, with the schema URL moved to `https://schema.tauri.app/config/3`.
To build on Tauri 2 instead, set those versions back to `2`, drop
`tauri-runtime-wry` and the `.runtime(...)` line, and set the schema to
`config/2`.

The whole library fits in a Tauri 3 app, not only the `turso` feature this
example uses: with every `seleniumbase-rs` feature switched on
(`cargo build --features seleniumbase-rs/mcp-server,seleniumbase-rs/turso,...`),
the app builds and links, and its 115 tests pass (checked before 0.3.0, when the
features also included `impersonate`, which has since moved to
seleniumbase-rs-stealth). No feature needs Python. (The `playwright` feature that
existed then is the separate crate `seleniumbase-rs-playwright` now.)

## Features

- Create / delete isolated browser profiles, stored encrypted at rest.
- Launch a profile against any WebDriver container, or as a Chrome of its own over Pure CDP, with no Docker.
- Open a profile in a window of the app, with its fingerprint's script (the plugin's stealth).
- Give a profile a random, internally consistent identity in one click.
- Navigate, screenshot, and close sessions from the UI.
- Per-profile fingerprint hints: user agent, locale, proxy, headless.
- Per-profile geolocation override via CDP `Emulation.setGeolocationOverride`.
- Tags and folders for organizing profiles.
- A local profile-compatible REST API (`http://127.0.0.1:45001/api/v1`) with CORS enabled.
- Ready for stealth/CDP/UC mode via `DriverMode` in `BrowserConfig`.
- UI tools for cloning, exporting/importing, proxy validation, and cookie management.

## Profile-compatible REST API

The Tauri backend starts an Actix-web server on `http://127.0.0.1:45001`. The UI uses it for tags/folders and profile tools, and external tools can call it directly.

### Authentication

Every endpoint requires this run's API token as a bearer token:

```
Authorization: Bearer <token>
```

The token is 256 bits of OS randomness, minted fresh on each start and never
written to disk. The app window fetches it over Tauri IPC (`get_api_token`),
which is reachable only from the app's own frontend.

This matters more than it first looks. Binding to `127.0.0.1` keeps the API off
the network, but it does **not** keep it away from the browser: any page the
user has open can also reach loopback. Without a token, a visited web page could
call `GET /api/v1/profiles` and read every saved profile — cookies and proxy
credentials included — or call `GET /api/v1/profiles/{id}/start?url=...` to
drive a logged-in browser session to a URL of its choosing. Because that one is
a plain GET, an `<img>` tag would be enough.

Two further limits back the token up:

- **CORS is restricted to the app window's own origins.** A request carrying
  any other `Origin` header is refused by the server before it reaches a
  handler, and a foreign page's preflight is refused too, so it cannot even
  attach the `Authorization` header. Tools with no `Origin` header, such as
  `curl`, are unaffected.
- **The `Host` header must name a loopback address.** This blocks DNS
  rebinding, where an attacker points a hostname they control at `127.0.0.1` to
  reach the API from a page they serve.

Requests failing either check are refused with `403 FORBIDDEN_HOST`, and a
missing or wrong token with `401 UNAUTHORIZED`.

Real endpoints:

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/version` | Launcher version |
| GET | `/api/v1/status` | Active sessions |
| GET | `/api/v1/profiles` | List profiles |
| POST | `/api/v1/profiles` | Create profile |
| GET | `/api/v1/profiles/:id` | Get profile |
| POST | `/api/v1/profiles/:id` | Update profile |
| DELETE | `/api/v1/profiles/:id` | Delete profile |
| GET | `/api/v1/profiles/:id/start?url=...` | Launch profile. For a Pure CDP profile, `port` and `ws_endpoint` are the browser's own DevTools address |
| GET | `/api/v1/profiles/:id/stop` | Stop profile session |
| POST | `/api/v1/profiles/:id/clone` | Clone profile |
| POST | `/api/v1/profiles/:id/randomize` | Give the profile a random identity. Optional body `{"os": "Macos", "seed": 7}`; returns `{os, seed, profile}` |
| GET | `/api/v1/profiles/:id/export` | Export profile JSON |
| POST | `/api/v1/profiles/import` | Import profile JSON |
| POST | `/api/v1/cookie_import` | Import cookies into profile/session |
| POST | `/api/v1/cookie_export` | Export stored cookies |
| POST | `/api/v1/proxy/validate` | Validate proxy via ipinfo.io |
| GET/POST | `/api/v1/tags` | List / create tags |
| POST/DELETE | `/api/v1/tags/:id` | Update / delete tag |
| GET/POST | `/api/v1/folders` | List / create folders |
| POST/DELETE | `/api/v1/folders/:id` | Update / delete folder |
| GET | `/api/v1/stop_all` | Stop every active session |

A profile's `engine` is `"WebDriver"` or `"PureCdp"` (`"pure_cdp"` is accepted too). `container_url` is required for WebDriver and ignored for Pure CDP. Every endpoint that reads or writes profiles, tags or folders answers `503` with `STORAGE_UNAVAILABLE` while the profile vault could not be opened, and `500` with `STORAGE_FAILED` if a change could not be saved, in which case nothing changed.

`os` takes `Windows`, `Macos`, `Linux`, `Android` or `ios`.

Stub endpoints (return placeholder data):

- `/api/v1/browser_cores`, `/api/v1/load_browser_core`, `/api/v1/delete_browser_core`
- `/api/v1/workspaces`
- `/api/v1/user/signin`, `/api/v1/user/refresh_token`
- `/api/v1/bookmarks/export`, `/api/v1/bookmarks/import`
- `/api/v1/2fa/setup`, `/api/v1/2fa/enable`

Example:

```bash
# Copy the token from the app window, or read it from the running process.
TOKEN='<token from get_api_token>'

curl http://127.0.0.1:45001/api/v1/profiles \
  -H "authorization: Bearer $TOKEN"

curl -X POST http://127.0.0.1:45001/api/v1/profiles \
  -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name":"EU Proxy","container_url":"http://localhost:4444","proxy":"http://proxy:8080"}'

# A Pure CDP profile needs no container, and can take a random identity.
curl -X POST http://127.0.0.1:45001/api/v1/profiles \
  -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name":"Direct","engine":"PureCdp","headless":true}'

curl -X POST http://127.0.0.1:45001/api/v1/profiles/<id>/randomize \
  -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"os":"Macos","seed":7}'
```

Without the header the same request returns `401`:

```bash
curl -i http://127.0.0.1:45001/api/v1/profiles
# HTTP/1.1 401 Unauthorized
```

## Known gaps

This is an example, not a finished product. Before relying on it:

- **The vault protects the file, not a running app.** Anything that runs as you
  while the app is open can reach the profiles. See [Profile storage](#profile-storage).
- **Pure CDP has no persistent browser profile.** Cookies and storage vanish
  when the session closes, and only the first tab gets the fingerprint.
- **The `2fa` and `user` endpoints are stubs.** They return fixed placeholder
  values and authenticate nobody.
- **The window runs with `"csp": null`.** Setting a real Content-Security-Policy
  in `tauri.conf.json` limits what injected content could do.

## Adding anti-detect hardening

To move closer to commercial-grade anti-detection:

1. Replace `selenium/standalone-chrome` with a custom Dockerfile that patches
   `cdc_` markers and injects anti-fingerprint extensions.
2. Use `DriverMode::Uc` in profiles to enable undetected-chrome args.
3. Add proxy assignment per profile and route containers through it.
4. Store cookies/cache per profile in `data/<profile>` and mount them into the
   container.

