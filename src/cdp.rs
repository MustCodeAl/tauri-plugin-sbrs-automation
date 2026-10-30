//! Optional Chromium CDP setup (feature `cdp`) and authenticated discovery.
//!
//! This does not add Chromium to a system WebKit runtime. Pass the returned
//! arguments to your CEF initialization, or use WebView2 on Windows.

use std::sync::atomic::{AtomicU16, Ordering};

/// The debugging port the launch settings turned on in this process (0: none).
/// Only [`CdpStealthConfig::chromium_args`] and [`CdpStealthConfig::apply_env`]
/// write it, and only with automation on, so the environment of a parent
/// process can never put a port here.
static ENABLED_PORT: AtomicU16 = AtomicU16::new(0);

/// What `GET /<token>/cdp` answers for the debugging port the plugin was told
/// of (`None`: CDP is off).
pub(crate) fn discovery(port: Option<u16>) -> serde_json::Value {
    serde_json::json!({
        "cdpEnabled": port.is_some(),
        "cdpPort": port,
        "cdpUrl": port.map(|port| format!("http://127.0.0.1:{port}")),
    })
}

/// The debugging port that the launch settings (`CdpStealthConfig`, feature
/// `cdp`) turned on in this process, if any. The plugin reads it once, when it
/// starts, and reports it on `GET /<token>/cdp`.
#[must_use]
pub fn enabled_port() -> Option<u16> {
    match ENABLED_PORT.load(Ordering::Relaxed) {
        0 => None,
        port => Some(port),
    }
}

/// Chromium launch settings for CEF or WebView2 (feature `cdp`).
///
/// ```
/// # #[cfg(feature = "cdp")] {
/// use tauri_plugin_sbrs_automation::cdp::CdpStealthConfig;
/// let config = CdpStealthConfig::default().debugging_port(9333);
/// // Nothing at all when automation is off (a release build without
/// // `SBRS_AUTOMATION=1`).
/// let args = config.chromium_args();
/// assert!(args.is_empty() || args.contains(&"--remote-debugging-port=9333".into()));
/// # }
/// ```
#[cfg(feature = "cdp")]
#[derive(Debug, Clone)]
pub struct CdpStealthConfig {
    /// `SBRS_CDP_PORT`, or 9222. Port zero uses 9222 rather than an undiscoverable port.
    pub debugging_port: u16,
    /// Include the few engine stealth arguments (default false: opt in). They
    /// hide `navigator.webdriver` and stop background throttling; none of them
    /// turns off a protection of the app's browser. Site isolation, the popup
    /// blocker and the encryption of stored data stay on.
    pub stealth_args: bool,
    /// Additional trusted arguments. The switches of the debugging endpoint
    /// (`--remote-debugging-*`, `--remote-allow-origins`) and WebView2's
    /// `--edge-webview-switches` are reserved, in any case and with any prefix
    /// (`--`, `-`, `/`): an argument that mentions one is dropped.
    pub extra_args: Vec<String>,
}

#[cfg(feature = "cdp")]
impl Default for CdpStealthConfig {
    fn default() -> Self {
        Self {
            debugging_port: std::env::var("SBRS_CDP_PORT")
                .ok()
                .and_then(|port| port.parse().ok())
                .filter(|port| *port != 0)
                .unwrap_or(9222),
            stealth_args: false,
            extra_args: Vec::new(),
        }
    }
}

#[cfg(feature = "cdp")]
impl CdpStealthConfig {
    /// Selects the fixed local debugging port.
    #[must_use]
    pub fn debugging_port(mut self, port: u16) -> Self {
        self.debugging_port = port;
        self
    }

    /// Opts in to (or out of) the stealth arguments. Off by default.
    #[must_use]
    pub fn stealth_args(mut self, enabled: bool) -> Self {
        self.stealth_args = enabled;
        self
    }

    /// Adds a trusted Chromium argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Returns the loopback debugging switches, the opt-in stealth flags and
    /// the extra arguments, or nothing at all when the plugin's automation
    /// gate is off (a release build without `SBRS_AUTOMATION=1`): an app that
    /// passes the result to CEF cannot open a debugging port in a release
    /// build, whatever it calls. Add the app's own permanent arguments
    /// yourself rather than through [`extra_args`](Self::extra_args), which
    /// are dropped with the rest.
    /// Reserved debugging switches in `extra_args` are dropped, so they cannot
    /// override the local endpoint.
    #[must_use]
    pub fn chromium_args(&self) -> Vec<String> {
        let port = self.port_with(crate::Settings::automation_enabled());
        if let Some(port) = port {
            // The plugin reads this once, when it starts, and reports it on
            // `GET /<token>/cdp`; nothing else tells it of a debugging port.
            ENABLED_PORT.store(port, Ordering::Relaxed);
        }
        self.args_for(port)
    }

    /// The port the debugger listens on, when the automation gate is on.
    fn port_with(&self, automation: bool) -> Option<u16> {
        let port = if self.debugging_port == 0 {
            9222
        } else {
            self.debugging_port
        };
        automation.then_some(port)
    }

    /// [`chromium_args`](Self::chromium_args) for a given state of the
    /// automation gate: nothing at all when it is off. It records no port.
    #[cfg(test)]
    fn args_with(&self, automation: bool) -> Vec<String> {
        self.args_for(self.port_with(automation))
    }

    /// The switches for `port`; nothing when there is none.
    fn args_for(&self, port: Option<u16>) -> Vec<String> {
        let Some(port) = port else {
            return Vec::new();
        };
        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            "--remote-debugging-address=127.0.0.1".to_owned(),
        ];
        if self.stealth_args {
            args.extend(STEALTH_ARGS.iter().map(|arg| (*arg).to_owned()));
        }
        args.extend(
            self.extra_args
                .iter()
                .filter(|arg| !is_reserved(arg))
                .cloned(),
        );
        args
    }

    /// Appends the arguments to `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` (the
    /// WebView2 way) and returns them (the CEF way), and records the debugging
    /// port for the plugin to report. The port is not exported: nothing but
    /// this variable is set, and WebView2 reads it as it creates its
    /// environment, so child processes that inherit it inherit the debugging
    /// switches too (remove it for a child that must not).
    /// Does nothing when the plugin's automation gate is off.
    ///
    /// # Safety
    ///
    /// Call before creating threads or initializing any runtime that might read
    /// the process environment, normally at the beginning of `main`, before Tauri.
    pub unsafe fn apply_env(&self) -> Vec<String> {
        if !crate::Settings::automation_enabled() {
            return Vec::new();
        }
        let existing = match std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => String::new(),
            // Text that is not Unicode can be neither checked nor extended
            // without changing it.
            Err(std::env::VarError::NotUnicode(_)) => return Vec::new(),
        };
        // These switches can bypass or supersede the TCP endpoint. Refuse
        // activation rather than advertising a different endpoint to clients.
        if is_reserved(&existing) {
            return Vec::new();
        }
        let args = self.chromium_args();
        let additional = args
            .iter()
            .map(|arg| webview2_argument(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let combined = if existing.is_empty() {
            additional
        } else {
            format!("{existing} {additional}")
        };

        // SAFETY: the caller guarantees exclusive access to the process
        // environment before any threads or runtime are started. The value has
        // no NUL bytes and the key is a fixed, valid environment name.
        unsafe {
            std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", combined);
        }
        args
    }
}

/// Opt-in launch flags that reduce automation signals and background throttling,
/// taken from `engine_spoofing_args()` in seleniumbase-rs (`src/stealth/patcher.rs`),
/// minus every flag that weakens the browser an app ships. Left out on purpose:
/// `--disable-features=IsolateOrigins,site-per-process,...` (site isolation, and
/// the mDNS hiding of local addresses in WebRTC), `--disable-popup-blocking`,
/// `--disable-client-side-phishing-detection`, `--disable-background-networking`
/// (updates of component and revocation data), `--password-store=basic` and
/// `--use-mock-keychain` (the key that encrypts stored cookies and passwords),
/// and `--force-webrtc-ip-handling-policy=disable_non_proxied_udp` (breaks
/// WebRTC). The other flags of that list only matter to a stand-alone browser.
///
/// Why each remaining flag is safe inside an app:
#[cfg(feature = "cdp")]
const STEALTH_ARGS: [&str; 5] = [
    // Blink only stops reporting `navigator.webdriver`; no security setting.
    "--disable-blink-features=AutomationControlled",
    // These three keep timers and rendering running in a window that is in
    // the background or hidden behind another one, so a test is not slowed
    // down. They change scheduling only.
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    // No "resubmit the form?" dialog that a script cannot answer; UI only.
    "--disable-prompt-on-repost",
];

/// Whether `text` names a switch this module reserves (in any spelling), or
/// holds a NUL byte, which no command line can carry.
#[cfg(feature = "cdp")]
fn is_reserved(text: &str) -> bool {
    // Chromium reads switch names without regard to case on Windows, takes
    // `/` and `-` as well as `--` as the prefix there, and drops quotes when
    // it splits the command line. So the text is lowered and unquoted, and the
    // names are searched anywhere in it rather than at its start.
    const RESERVED: [&str; 3] = [
        // The debugging port, address, pipe and any other of its switches.
        "remote-debugging-",
        // The origins that may open the debugging socket.
        "remote-allow-origins",
        // WebView2's own channel for browser switches.
        "edge-webview-switches",
    ];
    if text.contains('\0') {
        return true;
    }
    let plain = text.replace('"', "").to_ascii_lowercase();
    RESERVED.iter().any(|name| plain.contains(name))
}

/// Quotes one argument using the Windows command-line backslash/quote rules.
#[cfg(feature = "cdp")]
fn webview2_argument(arg: &str) -> String {
    if !arg.contains(|c: char| c.is_whitespace() || c == '"') {
        return arg.to_owned();
    }
    let mut result = String::from("\"");
    let mut slashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            '\\',
            if ch == '"' { slashes * 2 + 1 } else { slashes },
        ));
        result.push(ch);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

#[cfg(all(test, feature = "cdp"))]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_produced_when_automation_is_off() {
        // A release build must not open a debugging port, whatever the app
        // asked for: not the port, not the stealth flags, not the extras.
        let config = CdpStealthConfig::default()
            .debugging_port(9333)
            .stealth_args(true)
            .arg("--lang=en-US");
        assert!(config.args_with(false).is_empty());
        let on = config.args_with(true);
        assert!(on.contains(&"--remote-debugging-port=9333".to_owned()));
        assert!(on.contains(&"--remote-debugging-address=127.0.0.1".to_owned()));
    }

    #[test]
    fn stealth_arguments_are_off_by_default() {
        let config = CdpStealthConfig::default();
        assert!(!config.stealth_args);
        let args = config.args_with(true);
        assert_eq!(args.len(), 2, "{args:?}");
        assert!(args[0].starts_with("--remote-debugging-port="), "{args:?}");
        assert_eq!(args[1], "--remote-debugging-address=127.0.0.1");
    }

    #[test]
    fn the_stealth_list_is_exactly_the_vetted_one() {
        assert_eq!(
            STEALTH_ARGS.to_vec(),
            [
                "--disable-blink-features=AutomationControlled",
                "--disable-background-timer-throttling",
                "--disable-backgrounding-occluded-windows",
                "--disable-renderer-backgrounding",
                "--disable-prompt-on-repost",
            ]
        );
    }

    #[test]
    fn stealth_never_weakens_the_apps_own_browser() {
        // Each of these turns off a protection of the browser the app ships:
        // site isolation, the popup blocker, phishing detection, the update
        // of component data, the encryption of stored cookies and passwords,
        // and WebRTC's hiding of the local address.
        let args = CdpStealthConfig::default()
            .stealth_args(true)
            .args_with(true)
            .join(" ");
        for weakening in [
            "--disable-features",
            "site-per-process",
            "IsolateOrigins",
            "WebRtcHideLocalIpsWithMdns",
            "--disable-popup-blocking",
            "--disable-client-side-phishing-detection",
            "--disable-background-networking",
            "--password-store",
            "--use-mock-keychain",
            "--force-webrtc-ip-handling-policy",
            "--no-sandbox",
            "--disable-web-security",
        ] {
            assert!(!args.contains(weakening), "{weakening} in {args}");
        }
    }

    #[test]
    fn opt_in_stealth_adds_exactly_the_vetted_arguments() {
        let config = CdpStealthConfig::default()
            .debugging_port(9333)
            .stealth_args(true)
            .arg("--lang=en-US");
        let args = config.args_with(true);
        assert_eq!(
            &args[..2],
            [
                "--remote-debugging-port=9333",
                "--remote-debugging-address=127.0.0.1"
            ]
        );
        assert_eq!(&args[2..args.len() - 1], STEALTH_ARGS);
        assert_eq!(args.last().unwrap(), "--lang=en-US");
    }

    #[test]
    fn stealth_can_be_disabled_without_disabling_loopback_debugging() {
        let args = CdpStealthConfig::default()
            .debugging_port(9222)
            .stealth_args(false)
            .args_with(true);
        assert_eq!(
            args,
            [
                "--remote-debugging-port=9222",
                "--remote-debugging-address=127.0.0.1"
            ]
        );
    }

    #[test]
    fn extra_arguments_cannot_redirect_or_replace_the_debugger() {
        for arg in [
            "--remote-debugging-address=0.0.0.0",
            "--remote-debugging-port=9999",
            "--remote-debugging-pipe",
            // Chromium reads switch names without regard to case on Windows,
            // and takes `/` and `-` as well as `--` there.
            "--REMOTE-DEBUGGING-PORT=9999",
            "--Remote-Debugging-Address=0.0.0.0",
            "/remote-debugging-port=9999",
            "-remote-debugging-pipe",
            // The origins a debugging socket accepts.
            "--remote-allow-origins=*",
            "--REMOTE-ALLOW-ORIGINS=http://evil.example",
            // Quotes are dropped when the command line is split.
            "--remote-debugging-\"port\"=9999",
            "--EDGE-WEBVIEW-SWITCHES=x",
        ] {
            let args = CdpStealthConfig::default().arg(arg).args_with(true);
            assert!(!args.contains(&arg.to_owned()), "{arg} reached {args:?}");
        }
    }

    #[test]
    fn ordinary_arguments_are_kept() {
        let args = CdpStealthConfig::default()
            .arg("--lang=en-US")
            .arg("--user-agent=Tests/1.0")
            .args_with(true);
        assert!(args.contains(&"--lang=en-US".to_owned()));
        assert!(args.contains(&"--user-agent=Tests/1.0".to_owned()));
    }

    #[test]
    fn reserved_switches_are_found_in_any_spelling() {
        for text in [
            "--remote-debugging-port=1",
            "--REMOTE-DEBUGGING-PORT=1",
            "/remote-debugging-address=0.0.0.0",
            "-remote-debugging-pipe",
            "--lang=fr --Remote-Allow-Origins=*",
            "--remote-debugging-\"port\"=1",
            "--edge-webview-switches=x",
            "--lang=en\0",
        ] {
            assert!(is_reserved(text), "{text:?}");
        }
        for text in [
            "",
            "--lang=en-US",
            "--disable-gpu",
            "--user-agent=Tests/1.0",
        ] {
            assert!(!is_reserved(text), "{text:?}");
        }
    }
}
