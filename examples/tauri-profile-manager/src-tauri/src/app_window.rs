//! A profile opened in a window of the app itself, with the profile's identity.
//!
//! The app runs on CEF on every desktop. The window gets the profile's whole
//! stealth plan through `tauri-plugin-sbrs-automation`'s `stealth` feature: the
//! init script ([`StealthScriptExt::stealth_script`]) and, over the CEF
//! DevTools channel (`stealth::cef::channel`), the overrides (user agent and
//! Client Hints, locale, time zone, screen) and the profile's cookies, before
//! it goes to the page. It has the profile's proxy and a CEF profile of its
//! own; a window the plan cannot reach is closed rather than shown to the
//! site.
//!
//!
//! Each window is new, so it starts with the profile's current
//! fingerprint, keeps nothing on disk and shares no storage with the app's own
//! window. No capability names its label, so the site it shows cannot call the
//! app.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{Manager, Runtime, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_sbrs_automation::stealth::{StealthPlan, StealthScriptExt};

use crate::models::Profile;
use crate::store::build_config;
use serde_json::{Value, json};
use tauri::WebviewWindow;
use tauri_plugin_sbrs_automation::stealth::{DevToolsChannel, SeleniumBaseError, cef};
use tauri_runtime_cef::{
    AsCefWebviewDispatcher, DevToolsProtocol, WebviewCefExt, allocate_devtools_message_id,
};
use tracing::{error, warn};

use crate::models::BrowserCookie;
use crate::session::cookie_params;

/// A proxy address a webview takes: `http`, `https` or `socks5`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyUrl(pub Url);

/// The profile's proxy as a webview takes it, or `None` when it has none or
/// names something else.
fn proxy_url(proxy: Option<&str>) -> Option<ProxyUrl> {
    let url = Url::parse(proxy?.trim()).ok()?;
    matches!(url.scheme(), "http" | "https" | "socks5").then_some(ProxyUrl(url))
}

/// The window's own CEF profile: 16 bytes from its label (FNV-1a, twice), so
/// each window has storage of its own.
fn data_store_id(label: &str) -> [u8; 16] {
    let fnv = |seed: u64| {
        label.bytes().fold(seed, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&fnv(0xcbf2_9ce4_8422_2325).to_le_bytes());
    id[8..].copy_from_slice(&fnv(0x8422_2325_cbf2_9ce4).to_le_bytes());
    id
}

/// Joins the window's CEF DevTools channel to the plugin's bridge. The
/// observer only hands answers on, so it never blocks CEF's UI thread.
fn connect<R>(window: &WebviewWindow<R>) -> Result<DevToolsChannel, String>
where
    R: Runtime,
    R::WebviewDispatcher: AsCefWebviewDispatcher,
{
    let sender = window.clone();
    let (channel, observer) = cef::channel(
        move |message| Ok(sender.send_dev_tools_message(message)?),
        || Ok(allocate_devtools_message_id()?),
    );
    window
        .on_dev_tools_protocol(move |protocol| match protocol {
            DevToolsProtocol::MethodResult {
                message_id,
                success,
                result,
            } => observer.method_result(message_id, success, &result),
            DevToolsProtocol::Event { method, params } => observer.event(&method, &params),
            DevToolsProtocol::Message(_) => {}
        })
        .map_err(|e| format!("Could not reach the window's DevTools: {e}"))?;
    Ok(channel)
}

/// Sends the plan's overrides, then the profile's cookies.
async fn prepare(
    channel: &DevToolsChannel,
    plan: &StealthPlan,
    cookies: &[BrowserCookie],
) -> Result<(), SeleniumBaseError> {
    channel.apply(plan).await?;
    if !cookies.is_empty() {
        let cookies: Vec<Value> = cookies.iter().map(cookie_params).collect();
        channel
            .browser()
            .execute("Network.setCookies", json!({ "cookies": cookies }))
            .await?;
    }
    Ok(())
}

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

    #[test]
    fn only_http_https_and_socks5_proxies_reach_the_window() {
        for proxy in [
            "http://10.0.0.1:8080",
            "https://p.example:443",
            "socks5://127.0.0.1:9050",
        ] {
            assert!(proxy_url(Some(proxy)).is_some(), "{proxy}");
        }
        for proxy in ["", "10.0.0.1:8080", "ftp://p.example", "not a url"] {
            assert!(proxy_url(Some(proxy)).is_none(), "{proxy}");
        }
        assert!(proxy_url(None).is_none());
    }

    #[test]
    fn each_label_gets_a_data_store_of_its_own_and_the_same_one_every_time() {
        assert_eq!(data_store_id("profile-a-0"), data_store_id("profile-a-0"));
        assert_ne!(data_store_id("profile-a-0"), data_store_id("profile-a-1"));
    }

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
