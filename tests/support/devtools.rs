//! A stand-in for the DevTools HTTP endpoint of Chromium (`GET /json/version`),
//! for the tests of `GET /<token>/cdp`. Every server runs on a thread of its
//! own for the rest of the process and answers each connection once.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::thread;

/// A loopback port nothing listens on at this moment.
pub fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// What Chromium answers on `/json/version` when it listens on `port`.
pub fn version_body(port: u16) -> String {
    format!(
        r#"{{"Browser":"Chrome/140.0.0.0","Protocol-Version":"1.3","User-Agent":"Mozilla/5.0 (Test)","V8-Version":"14.0.0","WebKit-Version":"537.36","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/0a1b2c3d-4e5f"}}"#
    )
}

/// A complete HTTP/1.1 answer with `body` as JSON.
pub fn http(status_line: &str, body: &str) -> String {
    format!(
        "{status_line}\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Reads one request head from `stream`.
fn read_head(stream: &mut TcpStream) {
    let mut head = Vec::new();
    let mut chunk = [0u8; 512];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
}

/// Answers each connection on `listener` with `reply(port)`, then closes it
/// (or keeps it open when `keep_open`). Returns the port.
fn spawn(
    listener: TcpListener,
    reply: impl Fn(u16) -> String + Send + 'static,
    keep_open: bool,
) -> u16 {
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            read_head(&mut stream);
            let _ = stream.write_all(reply(port).as_bytes());
            if keep_open {
                held.push(stream);
            }
        }
    });
    port
}

/// Serves `reply(port)` as the whole answer on a free port.
pub fn serve_raw(reply: impl Fn(u16) -> String + Send + 'static) -> u16 {
    spawn(
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
        reply,
        false,
    )
}

/// Serves the `/json/version` of a Chromium on a free port.
pub fn serve_devtools() -> u16 {
    serve_raw(|port| http("HTTP/1.1 200 OK", &version_body(port)))
}

/// Serves the `/json/version` of a Chromium on `port`, which must be free.
pub fn serve_devtools_on(port: u16) -> u16 {
    spawn(
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap(),
        |port| http("HTTP/1.1 200 OK", &version_body(port)),
        false,
    )
}

/// Like `serve_devtools`, but the connection stays open after the answer.
pub fn serve_devtools_keeping_the_connection() -> u16 {
    spawn(
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
        |port| http("HTTP/1.1 200 OK", &version_body(port)),
        true,
    )
}

/// Accepts connections and never answers.
pub fn serve_silence() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    port
}
