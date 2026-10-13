//! The loopback HTTP server that carries WebDriver requests to the [`Driver`].

use std::io::Read;
use std::net::SocketAddr;
use std::thread;

use crate::webdriver::{Bridge, Driver};

/// Bodies larger than this are cut (WebDriver bodies are small).
const MAX_BODY: u64 = 16 * 1024 * 1024;

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
    fn the_server_answers_on_loopback_and_refuses_web_pages() {
        let addr = spawn("127.0.0.1:0".parse().unwrap(), Driver::new(Null, "abc")).unwrap();
        let ok = get(addr, "/abc/status", "");
        assert!(
            ok.starts_with("HTTP/1.1 200") && ok.contains("\"ready\":true"),
            "{ok}"
        );
        let page = get(addr, "/abc/status", "Origin: https://evil.example\r\n");
        assert!(page.starts_with("HTTP/1.1 403"), "{page}");
    }
}
