//! The plain HTTP/1.1 plumbing the loopback fakes share: accept connections one
//! at a time, read a request head, write a response that ends the connection.

use std::io::Read;
#[cfg(test)]
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
/// A request head larger than this is not one lvpm sent.
const MAX_REQUEST_HEAD: usize = 16 * 1024;

/// Answer connections one at a time, each by `handle`, until `stop` is set.
pub(crate) fn accept_loop(
    listener: TcpListener,
    stop: &AtomicBool,
    mut handle: impl FnMut(TcpStream),
) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Ok(stream) = stream {
            handle(stream);
        }
    }
}

/// Stop the thread running [`accept_loop`] on `addr` and wait for it.
pub(crate) fn shut_down(addr: SocketAddr, stop: &AtomicBool, thread: Option<JoinHandle<()>>) {
    stop.store(true, Ordering::SeqCst);
    // `accept` is blocking; a connection wakes it so it can see the flag.
    TcpStream::connect(addr).ok();
    if let Some(thread) = thread {
        thread.join().ok();
    }
}

/// One parsed request head.
pub(crate) struct Request {
    pub(crate) method: String,
    /// The target as sent, minus a proxy's `http://host`: a query stays.
    pub(crate) path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    /// The value of the first header called `name`, matched without case.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        let (_, value) = self
            .headers
            .iter()
            .find(|(have, _)| have.eq_ignore_ascii_case(name))?;
        Some(value)
    }
}

/// Read the request on `stream`, which gets its socket timeouts first.
pub(crate) fn accept(stream: &mut TcpStream) -> std::io::Result<Request> {
    stream.set_read_timeout(Some(SOCKET_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;
    read_request(stream)
}

pub(crate) fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_REQUEST_HEAD || stream.read(&mut byte)? == 0 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = path_of(request_line.next().unwrap_or_default());
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Ok(Request {
        method,
        path,
        headers,
    })
}

/// `/a/b` as sent to an origin, or `http://host/a/b` as sent to a proxy.
fn path_of(target: &str) -> String {
    match target.strip_prefix("http://") {
        Some(rest) => rest
            .find('/')
            .map_or("/", |slash| &rest[slash..])
            .to_string(),
        None => target.to_string(),
    }
}

/// An HTTP/1.1 response that ends the connection after `body`.
pub(crate) fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// What a test's raw `GET` got back.
#[cfg(test)]
pub(crate) struct Reply {
    pub(crate) status_line: String,
    pub(crate) head: String,
    pub(crate) body: Vec<u8>,
}

/// `GET target` from the server at `addr`, with `extra_headers` (each ending
/// in CRLF) after `Host: test`.
#[cfg(test)]
pub(crate) fn get_from(addr: SocketAddr, target: &str, extra_headers: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(SOCKET_TIMEOUT)).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: test\r\n{extra_headers}\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response head")
        + 4;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    Reply {
        status_line: head.lines().next().unwrap().to_string(),
        head,
        body: raw[split..].to_vec(),
    }
}
