//! The loopback HTTP server that carries WebDriver requests to the [`Driver`].

use std::io::Read;
use std::net::SocketAddr;
use std::thread;

use crate::webdriver::{Bridge, Driver};

/// Bodies larger than this are cut (WebDriver bodies are small).
const MAX_BODY: u64 = 16 * 1024 * 1024;

/// The server URL to give a WebDriver client. It ends with `/`: clients join
/// each command path (`session`, ...) to it as a relative URL, which would
/// replace a last segment, the token, that has no slash after it.
pub fn endpoint_url(local: SocketAddr, token: &str) -> String {
    format!("http://{local}/{token}/")
}

/// Serves `driver` on `addr` (see [`spawn`]) and says on stderr where, or why
/// not. A port that cannot be bound leaves the app running without the
/// endpoint: the plugin is on in every debug build, so a second instance of
/// the app, or a Selenium container on the same port, must not stop the app.
pub fn start<B: Bridge + 'static>(
    addr: SocketAddr,
    driver: Driver<B>,
    token: &str,
) -> Option<SocketAddr> {
    match spawn(addr, driver) {
        Ok(local) => {
            eprintln!(
                "tauri-plugin-sbrs-automation: WebDriver at {}",
                endpoint_url(local, token)
            );
            Some(local)
        }
        Err(error) => {
            eprintln!(
                "tauri-plugin-sbrs-automation: no WebDriver endpoint, cannot listen on {addr}: {error} (set SBRS_AUTOMATION_PORT to another port)"
            );
            None
        }
    }
}

/// Serves `driver` on `addr` from a thread of its own, one request at a time,
/// and returns the address it listens on.
///
/// # Errors
///
/// When the address cannot be bound or the thread cannot start.
pub fn spawn<B: Bridge + 'static>(
    addr: SocketAddr,
    driver: Driver<B>,
) -> std::io::Result<SocketAddr> {
    let server =
        tiny_http::Server::http(addr).map_err(|error| std::io::Error::other(error.to_string()))?;
    let local = server.server_addr().to_ip().unwrap_or(addr);
    thread::Builder::new()
        .name("sbrs-automation".to_owned())
        .spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = Vec::new();
                // A body that cannot be read is answered as an empty one.
                let _ = request.as_reader().take(MAX_BODY).read_to_end(&mut body);
                let origin = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("Origin"))
                    .map(|header| header.value.as_str().to_owned());
                let response = driver.handle(
                    request.method().as_str(),
                    request.url(),
                    origin.as_deref(),
                    &body,
                );
                let data = serde_json::to_vec(&response.body).unwrap_or_default();
                let mut reply =
                    tiny_http::Response::from_data(data).with_status_code(response.status);
                if let Ok(header) =
                    tiny_http::Header::from_bytes("Content-Type", "application/json; charset=utf-8")
                {
                    reply = reply.with_header(header);
                }
                // The client may have gone away; nothing to do then.
                let _ = request.respond(reply);
            }
        })?;
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdriver::WebDriverError;
    use serde_json::Value;
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::Duration;

    struct Null;

    impl Bridge for Null {
        fn call(
            &self,
            _op: &str,
            _args: Value,
            _timeout: Duration,
        ) -> Result<Value, WebDriverError> {
            Ok(Value::Null)
        }
    }

    fn get(addr: SocketAddr, path: &str, extra: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{extra}\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    #[test]
    fn a_port_in_use_leaves_the_app_without_an_endpoint_but_running() {
        // The plugin is on in every debug build, so a second instance of the
        // app, or a Selenium container on the same port, must not stop it.
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap();
        assert_eq!(start(addr, Driver::new(Null, "abc"), "abc"), None);
    }

    #[test]
    fn a_free_port_serves_and_says_where() {
        let local = start(
            "127.0.0.1:0".parse().unwrap(),
            Driver::new(Null, "abc"),
            "abc",
        )
        .unwrap();
        let ok = get(local, "/abc/status", "");
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    }

    #[test]
    fn the_endpoint_url_ends_with_a_slash() {
        // Clients join command paths (`session`, ...) to it as relative URLs,
        // which would replace a last segment, the token, that has no slash.
        let url = endpoint_url("127.0.0.1:4445".parse().unwrap(), "abc");
        assert_eq!(url, "http://127.0.0.1:4445/abc/");
    }

    #[test]
    fn the_server_answers_on_loopback_and_refuses_web_pages() {
        let addr = spawn("127.0.0.1:0".parse().unwrap(), Driver::new(Null, "abc")).unwrap();
        let ok = get(addr, "/abc/status", "");
        assert!(
            ok.starts_with("HTTP/1.1 200") && ok.contains("\"ready\":true"),
            "{ok}"
        );
        let page = get(addr, "/abc/status", "Origin: https://evil.example\r\n");
        assert!(page.starts_with("HTTP/1.1 403"), "{page}");
        let cdp = get(addr, "/abc/cdp", "");
        assert!(
            cdp.starts_with("HTTP/1.1 200") && cdp.contains("\"cdpEnabled\""),
            "{cdp}"
        );
        let page = get(addr, "/abc/cdp", "Origin: null\r\n");
        assert!(page.starts_with("HTTP/1.1 403"), "{page}");
        assert!(get(addr, "/abcX/cdp", "").starts_with("HTTP/1.1 404"));
    }
}
