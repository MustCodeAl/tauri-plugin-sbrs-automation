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

/// Chromium launch settings for CEF or WebView2 (feature `cdp`, Rust 1.99).
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
    /// Include seleniumbase-rs's canonical engine stealth arguments (default true).
    pub stealth_args: bool,
    /// Additional trusted arguments. Remote-debugging switches are reserved.
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
            stealth_args: true,
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

    /// Enables or disables canonical stealth arguments.
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

    /// Returns loopback debugging switches, canonical stealth flags, and extra arguments.
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
            args.extend(seleniumbase_rs::stealth::engine_spoofing_args());
        }
        args.extend(
            self.extra_args
                .iter()
                .filter(|arg| {
                    !arg.contains('\0')
                        && !arg.contains("remote-debugging-")
                        && !arg.contains("edge-webview-switches")
                })
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
        if existing.contains("remote-debugging-pipe") || existing.contains("edge-webview-switches")
        {
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
    fn the_flags_use_the_canonical_generator() {
        let config = CdpStealthConfig::default()
            .debugging_port(9333)
            .arg("--lang=en-US");
        let args = config.chromium_args();
        assert_eq!(
            &args[..2],
            [
                "--remote-debugging-port=9333",
                "--remote-debugging-address=127.0.0.1"
            ]
        );
        let canonical = seleniumbase_rs::stealth::engine_spoofing_args();
        assert_eq!(&args[2..args.len() - 1], canonical);
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
        ] {
            let args = CdpStealthConfig::default().arg(arg).chromium_args();
            assert!(!args.contains(&arg.to_owned()));
        }
    }
}
