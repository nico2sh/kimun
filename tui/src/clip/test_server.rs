//! A local HTTP server for tests: it answers each connection with the next
//! canned reply and counts the connections.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// A local test server. It answers each connection with the next canned
/// reply, and repeats the last one.
pub(crate) struct Server {
    pub(crate) base: String,
    pub(crate) port: u16,
    accepts: Arc<AtomicUsize>,
}

impl Server {
    pub(crate) fn accepted(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }
}

/// Builds one reply: the status line, extra headers, and the body. When
/// `stall` is set, the server sends the reply and then waits instead of
/// closing the connection.
pub(crate) fn reply(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for h in headers {
        out.push_str(h);
        out.push_str("\r\n");
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut out = out.into_bytes();
    out.extend_from_slice(body);
    out
}

pub(crate) fn serve(stall: bool, make: impl FnOnce(&str) -> Vec<Vec<u8>>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let replies = make(&base);
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = accepts.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let r = &replies[n.min(replies.len() - 1)];
            let _ = stream.write_all(r);
            let _ = stream.flush();
            if stall {
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    });
    Server {
        base,
        port,
        accepts,
    }
}

pub(crate) fn ok_page(body: &str) -> Vec<u8> {
    reply(
        "200 OK",
        &["Content-Type: text/html; charset=utf-8"],
        body.as_bytes(),
    )
}
