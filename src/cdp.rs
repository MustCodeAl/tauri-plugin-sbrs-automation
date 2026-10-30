//! Chromium DevTools (CDP) for an app that ships its own Chromium: CEF, or
//! WebView2 on Windows. The launch settings, `CdpStealthConfig`, are the
//! feature `cdp`; the discovery route `GET /<token>/cdp` is always there.
//! This does not add Chromium to a system WebKit runtime (macOS, Linux).
//!
//! # The debugging socket has no authentication
//!
//! Chromium's DevTools socket accepts every process that can reach it, and on
//! a shared machine every user: whoever connects can run script in each page
//! of the app, read what the app stores and drive its windows. Only the
//! discovery route is protected (the secret token in the URL, loopback only,
//! no request from a web page); the socket it names is not. So the launch
//! settings produce the debugging switches only when the plugin's automation
//! gate is on (a debug build, or `SBRS_AUTOMATION` set to `1`, `true` or
//! `on`), bind them to `127.0.0.1`, and a release build of an app must not
//! turn the gate on.
//!
//! # Which port
//!
//! Without `SBRS_CDP_PORT` the launch settings pick a free loopback port, the
//! same one for the life of the process. A port that is asked for and in use
//! enables nothing. The plugin reads the port the launch settings enabled
//! once, when it starts, and never from the environment of a parent process.
//! `GET /<token>/cdp` advertises the port only while `GET /json/version`
//! answers there like Chromium (`cdpState` is `ready`; `unreachable` before
//! Chromium has started or when something else holds the port). That confirms
//! a Chromium debugger listens on the port, not that the browser is the app's
//! own: another browser that took the port in the instant between the choice
//! and the start of Chromium would pass.
//!
//! # Setup
//!
//! Pass `CdpStealthConfig::chromium_args` to CEF (its `command_line_arg`
//! takes the switch name without the leading `--`, and a value), or call
//! `CdpStealthConfig::apply_env` before Tauri starts, on WebView2.
//! The Tauri CEF guide documents `Cef::command_line_arg` and
//! `Cef::remote_debugging`, which is off by default and, as a port, reachable
//! by every process on the machine.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

/// The debugging port the launch settings turned on in this process (0: none).
/// Only [`CdpStealthConfig::chromium_args`] and [`CdpStealthConfig::apply_env`]
/// write it, and only with automation on, so the environment of a parent
/// process can never put a port here.
static ENABLED_PORT: AtomicU16 = AtomicU16::new(0);

/// What `GET /<token>/cdp` answers for the debugging port the plugin was told
/// of (`None`: CDP is off).
pub(crate) fn discovery(port: Option<u16>) -> serde_json::Value {
    discovery_with(port, PROBE_TIMEOUT)
}

/// How long the discovery route waits for the debugger to answer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// [`discovery`] with a given wait for the debugger.
fn discovery_with(port: Option<u16>, timeout: Duration) -> serde_json::Value {
    let (state, port) = match port {
        None => ("disabled", None),
        Some(port) if devtools_answers(port, timeout) => ("ready", Some(port)),
        Some(_) => ("unreachable", None),
    };
    serde_json::json!({
        "cdpEnabled": port.is_some(),
        "cdpPort": port,
        "cdpUrl": port.map(|port| format!("http://127.0.0.1:{port}")),
        "cdpState": state,
    })
}

/// How much of an answer the probe reads: `/json/version` is a few hundred
/// bytes, and what listens there may not be Chromium.
const PROBE_LIMIT: usize = 16 * 1024;

/// Whether a Chromium DevTools endpoint answers `GET /json/version` on
/// `127.0.0.1:port` within `timeout`: a `200` whose JSON names a `Browser` and
/// a `webSocketDebuggerUrl` on that same port. It does not prove that the
/// browser is the app's own, only that what answers on the port the launch
/// settings chose is a Chromium debugger and not some other listener. A second
/// browser that took the port in the instant between the check for a free port
/// and the start of Chromium would pass; a port chosen free makes that unlikely.
fn devtools_answers(port: u16, timeout: Duration) -> bool {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Instant;

    let deadline = Instant::now() + timeout;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    if stream.set_write_timeout(Some(timeout)).is_err()
        || stream.write_all(request.as_bytes()).is_err()
    {
        return false;
    }
    let mut reply = Vec::new();
    let mut chunk = [0u8; 2048];
    while reply.len() < PROBE_LIMIT && !answer_is_complete(&reply) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
            Err(_) => return false,
        }
    }
    is_devtools_version(&reply, port)
}

/// Where the head of an HTTP answer ends and its body begins.
fn body_start(reply: &[u8]) -> Option<usize> {
    reply
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|end| end + 4)
}

/// Whether the head says how long the body is and the body is all there:
/// then there is no need to wait for the connection to close.
fn answer_is_complete(reply: &[u8]) -> bool {
    let Some(start) = body_start(reply) else {
        return false;
    };
    let head = String::from_utf8_lossy(&reply[..start]);
    head.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, length)| length.trim().parse::<usize>().ok())
        .is_some_and(|length| reply.len() >= start + length)
}

/// Whether `reply` is the `200` answer of a Chromium DevTools endpoint that
/// listens on `port`.
fn is_devtools_version(reply: &[u8], port: u16) -> bool {
    let Some(start) = body_start(reply) else {
        return false;
    };
    let succeeded = reply.starts_with(b"HTTP/1.1 200") || reply.starts_with(b"HTTP/1.0 200");
    let Ok(version) = serde_json::from_slice::<serde_json::Value>(&reply[start..]) else {
        return false;
    };
    succeeded
        && version["Browser"].is_string()
        && version["webSocketDebuggerUrl"]
            .as_str()
            .is_some_and(|url| url.starts_with(&format!("ws://127.0.0.1:{port}/devtools/")))
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
    /// The port to listen on: `SBRS_CDP_PORT` when it holds a valid non-zero
    /// port, else `None`, which picks a free loopback port (the same one for
    /// the life of the process). A port that is already in use enables
    /// nothing, so a client cannot end up at another browser; the discovery
    /// route reports the port that was chosen.
    pub debugging_port: Option<u16>,
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
                .filter(|port| *port != 0),
            stealth_args: false,
            extra_args: Vec::new(),
        }
    }
}

#[cfg(feature = "cdp")]
impl CdpStealthConfig {
    /// Selects a fixed local debugging port; zero asks for a free one, as
    /// `None` does. The port must be free when the arguments are built.
    #[must_use]
    pub fn debugging_port(mut self, port: u16) -> Self {
        self.debugging_port = Some(port).filter(|port| *port != 0);
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
        self.args_for(self.port())
    }

    /// The debugging port the launch settings use, for an app that turns on
    /// remote debugging itself (CEF's `remote_debugging`, say) instead of
    /// passing [`chromium_args`](Self::chromium_args): the port asked for when
    /// it is free, else a free one. `None` when the plugin's automation gate
    /// is off or the port asked for is in use. Like `chromium_args`, it makes
    /// the plugin report the port on `GET /<token>/cdp`, once a Chromium
    /// debugger answers there.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        let port = self.resolve(crate::Settings::automation_enabled());
        if let Some(port) = port {
            // The plugin reads this once, when it starts, and reports it on
            // `GET /<token>/cdp`; nothing else tells it of a debugging port.
            ENABLED_PORT.store(port, Ordering::Relaxed);
        }
        port
    }

    /// The port the debugger listens on, when the automation gate is on.
    fn resolve(&self, automation: bool) -> Option<u16> {
        if !automation {
            return None;
        }
        match self.debugging_port {
            None => {
                let port = automatic_port();
                if port.is_none() {
                    eprintln!(
                        "tauri-plugin-sbrs-automation: no CDP endpoint, no free loopback port was found"
                    );
                }
                port
            }
            // A port that an earlier call recorded is this process's own: it
            // is held by the Chromium these arguments started, not by another.
            Some(port) if enabled_port() == Some(port) || port_is_free(port) => Some(port),
            Some(port) => {
                eprintln!(
                    "tauri-plugin-sbrs-automation: no CDP endpoint, port {port} is already in use (leave SBRS_CDP_PORT unset to pick a free port)"
                );
                None
            }
        }
    }

    /// [`chromium_args`](Self::chromium_args) for a given state of the
    /// automation gate: nothing at all when it is off. It records no port.
    #[cfg(test)]
    fn args_with(&self, automation: bool) -> Vec<String> {
        self.args_for(self.resolve(automation))
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
        // No arguments (the port asked for is in use): leave the environment as it is.
        if args.is_empty() {
            return Vec::new();
        }
        // No arguments (the port asked for is in use): leave the environment as it is.

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

/// A loopback port that nothing listens on at this moment.
#[cfg(feature = "cdp")]
fn free_loopback_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).ok()?;
    listener.local_addr().ok().map(|address| address.port())
}

/// Whether nothing listens on `127.0.0.1:port` at this moment.
#[cfg(feature = "cdp")]
fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// The port this process uses when none was asked for: a free one, found the
/// first time and the same afterwards, so that every call that builds the
/// arguments agrees. There is a moment between finding it and Chromium
/// binding it in which another process could take it; the discovery route
/// does not advertise a port where no Chromium debugger answers.
#[cfg(feature = "cdp")]
fn automatic_port() -> Option<u16> {
    static PORT: std::sync::OnceLock<Option<u16>> = std::sync::OnceLock::new();
    *PORT.get_or_init(free_loopback_port)
}

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

/// A fake DevTools endpoint for the tests below, shared with `tests/`.
#[cfg(test)]
#[path = "../tests/support/devtools.rs"]
mod devtools;

#[cfg(test)]
mod discovery_tests {
    use super::{devtools, discovery_with};
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(2);

    fn answer(state: &str, port: Option<u16>) -> Value {
        json!({
            "cdpEnabled": port.is_some(),
            "cdpPort": port,
            "cdpUrl": port.map(|port| format!("http://127.0.0.1:{port}")),
            "cdpState": state,
        })
    }

    #[test]
    fn without_a_port_the_answer_is_disabled() {
        assert_eq!(discovery_with(None, WAIT), answer("disabled", None));
    }

    #[test]
    fn a_port_where_devtools_answers_is_advertised() {
        let port = devtools::serve_devtools();
        assert_eq!(
            discovery_with(Some(port), WAIT),
            answer("ready", Some(port))
        );
    }

    #[test]
    fn a_port_where_nothing_listens_is_not_advertised() {
        let port = devtools::free_port();
        assert_eq!(
            discovery_with(Some(port), WAIT),
            answer("unreachable", None)
        );
    }

    #[test]
    fn a_listener_that_is_not_devtools_is_not_advertised() {
        let others = [
            // Another HTTP server.
            devtools::serve_raw(|_| devtools::http("HTTP/1.1 200 OK", r#"{"hello":"world"}"#)),
            // Chromium's answer, but not a success.
            devtools::serve_raw(|port| {
                devtools::http("HTTP/1.1 404 Not Found", &devtools::version_body(port))
            }),
            // Text that is not JSON.
            devtools::serve_raw(|_| devtools::http("HTTP/1.1 200 OK", "<html></html>")),
            // Not HTTP at all.
            devtools::serve_raw(|_| "SSH-2.0-OpenSSH_9.0\r\n".to_owned()),
            // A DevTools answer that names another port: a proxy, or a second
            // browser behind this one.
            devtools::serve_raw(|port| {
                devtools::http(
                    "HTTP/1.1 200 OK",
                    &devtools::version_body(port).replace(
                        &format!("127.0.0.1:{port}"),
                        &format!("127.0.0.1:{}", port.wrapping_add(1)),
                    ),
                )
            }),
        ];
        for port in others {
            assert_eq!(
                discovery_with(Some(port), WAIT),
                answer("unreachable", None),
                "port {port}"
            );
        }
    }

    #[test]
    fn a_listener_that_never_answers_is_given_up_on() {
        let port = devtools::serve_silence();
        let started = Instant::now();
        assert_eq!(
            discovery_with(Some(port), Duration::from_millis(300)),
            answer("unreachable", None)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn an_answer_with_its_length_needs_no_closed_connection() {
        let port = devtools::serve_devtools_keeping_the_connection();
        let started = Instant::now();
        assert_eq!(
            discovery_with(Some(port), Duration::from_secs(10)),
            answer("ready", Some(port))
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[cfg(all(test, feature = "cdp"))]
mod tests {
    use super::devtools;
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    /// A config that asks for no port, whatever `SBRS_CDP_PORT` holds.
    fn unset() -> CdpStealthConfig {
        CdpStealthConfig {
            debugging_port: None,
            ..CdpStealthConfig::default()
        }
    }

    #[test]
    fn an_unset_port_is_a_free_loopback_port_that_stays_the_same() {
        let port = unset().resolve(true).unwrap();
        assert_ne!(port, 0);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
        // Another config, another call: the same port for the whole process.
        assert_eq!(unset().resolve(true), Some(port));
        assert_eq!(unset().resolve(false), None);
        assert!(
            unset()
                .args_with(true)
                .contains(&format!("--remote-debugging-port={port}"))
        );
    }

    #[test]
    fn a_free_port_that_was_asked_for_is_kept() {
        let port = devtools::free_port();
        assert_eq!(
            CdpStealthConfig::default()
                .debugging_port(port)
                .resolve(true),
            Some(port)
        );
    }

    #[test]
    fn a_port_that_is_taken_enables_nothing() {
        // Another browser (or another instance of the app) already listens
        // there: Chromium would fail to bind, and a client that followed the
        // port would attach to that other browser.
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let config = CdpStealthConfig::default()
            .debugging_port(port)
            .stealth_args(true);
        assert_eq!(config.resolve(true), None);
        assert!(config.args_with(true).is_empty());
    }

    #[test]
    fn port_zero_asks_for_a_free_port_like_none() {
        assert_eq!(
            CdpStealthConfig::default().debugging_port(0).debugging_port,
            None
        );
        assert_eq!(
            CdpStealthConfig::default()
                .debugging_port(9333)
                .debugging_port,
            Some(9333)
        );
    }

    #[test]
    fn nothing_is_produced_when_automation_is_off() {
        // A release build must not open a debugging port, whatever the app
        // asked for: not the port, not the stealth flags, not the extras.
        let port = devtools::free_port();
        let config = CdpStealthConfig::default()
            .debugging_port(port)
            .stealth_args(true)
            .arg("--lang=en-US");
        assert!(config.args_with(false).is_empty());
        assert_eq!(config.resolve(false), None);
        let on = config.args_with(true);
        assert!(on.contains(&format!("--remote-debugging-port={port}")));
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
        let port = devtools::free_port();
        let config = CdpStealthConfig::default()
            .debugging_port(port)
            .stealth_args(true)
            .arg("--lang=en-US");
        let args = config.args_with(true);
        assert_eq!(
            &args[..2],
            [
                format!("--remote-debugging-port={port}"),
                "--remote-debugging-address=127.0.0.1".to_owned()
            ]
        );
        assert_eq!(&args[2..args.len() - 1], STEALTH_ARGS);
        assert_eq!(args.last().unwrap(), "--lang=en-US");
    }

    #[test]
    fn stealth_can_be_disabled_without_disabling_loopback_debugging() {
        let port = devtools::free_port();
        let args = CdpStealthConfig::default()
            .debugging_port(port)
            .stealth_args(false)
            .args_with(true);
        assert_eq!(
            args,
            [
                format!("--remote-debugging-port={port}"),
                "--remote-debugging-address=127.0.0.1".to_owned()
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
