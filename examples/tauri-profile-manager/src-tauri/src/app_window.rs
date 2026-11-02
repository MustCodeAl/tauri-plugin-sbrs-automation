//! A profile opened in a window of the app itself, with the profile's identity.
//!
//! The window's webview is the system one (WKWebView on macOS, WebView2 on
//! Windows, WebKitGTK on Linux), not a Chrome the app launches. It gets the
//! profile's fingerprint from `tauri-plugin-sbrs-automation`'s `stealth`
//! feature: [`StealthScriptExt::stealth_script`] adds the fingerprint's init
//! script, which runs in every document and frame before the page's own
//! scripts. Every runtime takes that script. The protocol overrides of the
//! same plan (user agent and Client Hints, locale, time zone, screen) need a
//! DevTools channel, which WKWebView and WebKitGTK do not have and which this
//! example does not wire up for WebView2.
//!
//! Each window is new and incognito, so it starts with the profile's current
//! fingerprint, keeps nothing on disk and shares no storage with the app's own
//! window. No capability names its label, so the site it shows cannot call the
//! app.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_sbrs_automation::stealth::{StealthPlan, StealthScriptExt};

use crate::models::Profile;
use crate::store::build_config;

/// The page a window opens when it is given none.
const BLANK: &str = "about:blank";

/// Numbers the windows, so each one gets a label of its own.
static OPENED: AtomicU64 = AtomicU64::new(0);

/// A window to open for a profile.
#[derive(Debug)]
pub struct ProfileWindow {
    /// The window's label: `profile-`, the profile's id and a number.
    pub label: String,
    /// The window's title: the profile's name.
    pub title: String,
    /// The page the window goes to, once the stealth is in place.
    pub url: Url,
    /// The stealth of the profile's fingerprint.
    pub plan: StealthPlan,
    /// The profile's cookies, set before the window goes to `url`.
    pub cookies: Vec<BrowserCookie>,
    /// The profile's proxy, when it names one a webview can use.
    pub proxy: Option<ProxyUrl>,
}

impl ProfileWindow {
    /// Plans a window for `profile` at `url` (`about:blank` when `None` or
    /// empty), with the fingerprint a launch of the profile would use: its
    /// own, else its imported profile's.
    ///
    /// # Errors
    ///
    /// When the profile has no fingerprint, its id cannot be part of a window
    /// label, or `url` is not an `http` or `https` address.
    pub fn plan(profile: &Profile, url: Option<&str>) -> Result<Self, String> {
            let url = start_url(url)?;
            let label = window_label(&profile.id)?;
            let config = build_config(profile);
            let fingerprint = config.fingerprint.ok_or_else(|| {
                format!(
                    "\"{}\" has no fingerprint to open with: randomize it or set one first",
                    profile.name
                )
            })?;
            let proxy = proxy_url(config.proxy.as_deref());
            if config.proxy.is_some() && proxy.is_none() {
                warn!(profile_id = %profile.id, "the proxy is not an http or socks5 address: the window opens without it");
            }
            Ok(Self {
                label,
                title: profile.name.clone(),
                url,
                plan: StealthPlan::new(&fingerprint),
                cookies: profile.cookies.clone(),
                proxy,
            })
        }

    /// Opens the window.
    ///
    /// # Errors
    ///
    /// What Tauri reports when it cannot build the window.
    pub fn open<R, M>(self, manager: &M) -> Result<(), String>
        where
            R: Runtime,
            R::WebviewDispatcher: AsCefWebviewDispatcher,
            M: Manager<R>,
        {
            let Self {
                label,
                title,
                url,
                plan,
                cookies,
                proxy,
            } = self;
            // The window opens at a blank page and goes to `url` only after the
            // plan is in: the overrides apply from the next request on.
            let mut builder =
                WebviewWindowBuilder::new(manager, &label, WebviewUrl::External(start_url(None)?))
                    .title(title)
                    .incognito(true)
                    .data_store_identifier(data_store_id(&label))
                    .stealth_script(&plan);
            if let Some(proxy) = proxy {
                builder = builder.proxy_url(proxy.0);
            }
            let window = builder
                .build()
                .map_err(|e| format!("Could not open the window: {e}"))?;
            let channel = connect(&window)?;
            tauri::async_runtime::spawn(async move {
                // A window the plan could not reach stays off the site: it would
                // show the real browser.
                if let Err(e) = prepare(&channel, &plan, &cookies).await {
                    error!(window = %label, error = %e, "could not apply the profile's stealth: closing the window");
                    let _ = window.destroy();
                } else if url.as_str() != BLANK
                    && let Err(e) = window.navigate(url)
                {
                    error!(window = %label, error = %e, "could not go to the page");
                }
            });
            Ok(())
        }
}

/// The page to open: an `http` or `https` address, or `about:blank`.
fn start_url(url: Option<&str>) -> Result<Url, String> {
    let text = url
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(BLANK);
    let parsed = Url::parse(text).map_err(|e| format!("Not a web address: {text} ({e})"))?;
    if parsed.as_str() == BLANK || matches!(parsed.scheme(), "http" | "https") {
        Ok(parsed)
    } else {
        Err(format!(
            "Only http and https pages open in an app window, not {text}"
        ))
    }
}

/// A label no other window has. An id of anything but ASCII letters, digits,
/// `-` and `_` is refused rather than changed, so two profiles can never
/// share a label.
fn window_label(id: &str) -> Result<String, String> {
    let fits = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_');
    if id.is_empty() || !id.chars().all(fits) {
        return Err(format!("The profile id {id:?} cannot name a window"));
    }
    let number = OPENED.fetch_add(1, Ordering::Relaxed);
    Ok(format!("profile-{id}-{number}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::default_profiles;
    use tauri_plugin_sbrs_automation::stealth::{Fingerprint, OsType};

    fn masked() -> Profile {
        let mut profile = default_profiles().remove(0);
        profile.fingerprint = Some(Fingerprint::randomized(OsType::Windows, 7));
        profile
    }

    #[test]
    fn the_window_runs_the_profiles_fingerprint_script() {
        let profile = masked();
        let window = ProfileWindow::plan(&profile, Some("https://example.com/")).unwrap();
        let expected = StealthPlan::new(profile.fingerprint.as_ref().unwrap());
        assert_eq!(window.plan.init_script(), expected.init_script());
        assert_eq!(window.url.as_str(), "https://example.com/");
        assert_eq!(window.title, profile.name);
    }

    #[test]
    fn a_profile_without_a_fingerprint_is_refused() {
        let mut profile = masked();
        profile.fingerprint = None;
        let error = ProfileWindow::plan(&profile, None).unwrap_err();
        assert!(error.contains("no fingerprint"), "{error}");
    }

    #[test]
    fn without_an_address_the_window_opens_a_blank_page() {
        for url in [None, Some(""), Some("  ")] {
            let window = ProfileWindow::plan(&masked(), url).unwrap();
            assert_eq!(window.url.as_str(), BLANK, "{url:?}");
        }
    }

    #[test]
    fn only_web_pages_open() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "tauri://localhost/",
            "about:config",
            "example.com",
        ] {
            assert!(
                ProfileWindow::plan(&masked(), Some(url)).is_err(),
                "{url} was accepted"
            );
        }
        assert!(ProfileWindow::plan(&masked(), Some("http://127.0.0.1:8080/")).is_ok());
    }

    #[test]
    fn each_window_gets_a_label_of_its_own() {
        let profile = masked();
        let first = ProfileWindow::plan(&profile, None).unwrap().label;
        let second = ProfileWindow::plan(&profile, None).unwrap().label;
        assert_ne!(first, second);
        for label in [&first, &second] {
            assert!(
                label.starts_with(&format!("profile-{}-", profile.id)),
                "{label}"
            );
        }
    }

    #[test]
    fn an_id_that_cannot_name_a_window_is_refused() {
        for id in ["", "a b", "a/b", "ä"] {
            let mut profile = masked();
            profile.id = id.to_owned();
            assert!(
                ProfileWindow::plan(&profile, None).is_err(),
                "{id:?} was accepted"
            );
        }
    }
}
