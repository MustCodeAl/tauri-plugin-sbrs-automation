//! Optional Chromium CDP setup (feature `cdp`) and authenticated discovery.
//!
//! This does not add Chromium to a system WebKit runtime. Pass the returned
//! arguments to your CEF initialization, or use WebView2 on Windows.

/// The configured port, only when automation and CDP were explicitly enabled.
pub(crate) fn active_port() -> Option<u16> {
    if !crate::Settings::automation_enabled() {
        return None;
    }
    std::env::var("SBRS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .filter(|port| *port != 0)
}

/// Chromium launch settings for CEF or WebView2 (feature `cdp`).
///
/// ```
/// # #[cfg(feature = "cdp")] {
/// use tauri_plugin_sbrs_automation::cdp::CdpStealthConfig;
/// let config = CdpStealthConfig::default().debugging_port(9333);
/// assert!(config.chromium_args().contains(&"--remote-debugging-port=9333".into()));
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

    /// Returns loopback debugging switches, the opt-in stealth flags, and extra arguments.
    /// Reserved debugging switches in `extra_args` cannot override the local endpoint.
    #[must_use]
    pub fn chromium_args(&self) -> Vec<String> {
        let port = if self.debugging_port == 0 {
            9222
        } else {
            self.debugging_port
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

    /// Appends WebView2 flags and advertises `SBRS_CDP_PORT`; returns CEF arguments.
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
        let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
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
        let port = if self.debugging_port == 0 {
            9222
        } else {
            self.debugging_port
        };
        // SAFETY: the caller guarantees exclusive access to the process
        // environment before any threads or runtime are started. Values have
        // no NUL bytes and the keys are fixed, valid environment names.
        unsafe {
            std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", combined);
            std::env::set_var("SBRS_CDP_PORT", port.to_string());
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
    fn stealth_arguments_are_off_by_default() {
        let config = CdpStealthConfig::default();
        assert!(!config.stealth_args);
        let args = config.chromium_args();
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
            .chromium_args()
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
        let args = config.chromium_args();
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
            .chromium_args();
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
            let args = CdpStealthConfig::default().arg(arg).chromium_args();
            assert!(!args.contains(&arg.to_owned()), "{arg} reached {args:?}");
        }
    }

    #[test]
    fn ordinary_arguments_are_kept() {
        let args = CdpStealthConfig::default()
            .arg("--lang=en-US")
            .arg("--user-agent=Tests/1.0")
            .chromium_args();
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
