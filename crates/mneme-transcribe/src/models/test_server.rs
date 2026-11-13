//! A small HTTP/1.1 server on localhost for the download tests, so they need
//! no network. It answers `GET <path>` with the [`Reply`] set for that path
//! and closes the connection after every response.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What the server sends for one path.
#[derive(Clone)]
pub(crate) enum Reply {
    /// `200` with a correct `Content-Length` and the whole body.
    Ok(Vec<u8>),
    /// This status with a short text body.
    Status(u16),
    /// `302` to this `Location`.
    Redirect(String),
    /// `302` without a `Location` header.
    RedirectWithoutLocation,
    /// `200` declaring `declared` bytes, then `body`, then the connection
    /// closes.
    DeclaredLength { body: Vec<u8>, declared: u64 },
    /// `200` with no `Content-Length`: the body ends when the connection
    /// closes.
    NoLength(Vec<u8>),
    /// `200` with a correct `Content-Length`. Sends `pause_after` bytes, then
    /// nothing until [`TestServer::release`] is called, then the rest.
    Gated { body: Vec<u8>, pause_after: usize },
    /// `200` with a correct `Content-Length`, sent `chunk` bytes at a time
    /// with `delay` between chunks.
    Trickle {
        body: Vec<u8>,
        chunk: usize,
        delay: Duration,
    },
}

pub(crate) struct TestServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept_thread: Option<JoinHandle<()>>,
}

struct Shared {
    routes: Mutex<HashMap<String, Reply>>,
    /// The path of every request received, in order.
    requests: Mutex<Vec<String>>,
    released: AtomicBool,
    stopping: AtomicBool,
}

impl TestServer {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a free localhost port");
        let addr = listener.local_addr().expect("the listener's address");
        let shared = Arc::new(Shared {
            routes: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            released: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        let accept_shared = Arc::clone(&shared);
        let accept_thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if accept_shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&accept_shared);
                thread::spawn(move || serve(stream, &shared));
            }
        });
        TestServer {
            addr,
            shared,
            accept_thread: Some(accept_thread),
        }
    }

    /// Sets what `GET path` answers. `path` starts with `/`.
    pub(crate) fn route(&self, path: &str, reply: Reply) {
        self.shared
            .routes
            .lock()
            .unwrap()
            .insert(path.to_owned(), reply);
    }

    /// `http://127.0.0.1:<port><path>`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The path of every request received so far, in order.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.shared.requests.lock().unwrap().clone()
    }

    /// Lets every [`Reply::Gated`] response send the rest of its body.
    pub(crate) fn release(&self) {
        self.shared.released.store(true, Ordering::SeqCst);
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some(path) = read_request_path(&mut stream) else {
        return;
    };
    shared.requests.lock().unwrap().push(path.clone());
    let reply = shared.routes.lock().unwrap().get(&path).cloned();
    // A failed write means the client went away, which the cancel tests do
    // on purpose.
    let _ = respond(&mut stream, reply, shared);
}

fn read_request_path(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 || head.len() > 16 * 1024 {
            return None;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).ok()?;
    let mut request_line = head.lines().next()?.split(' ');
    let _method = request_line.next()?;
    request_line.next().map(str::to_owned)
}

fn respond(stream: &mut TcpStream, reply: Option<Reply>, shared: &Shared) -> std::io::Result<()> {
    match reply {
        None => send_status(stream, 404),
        Some(Reply::Status(status)) => send_status(stream, status),
        Some(Reply::Ok(body)) => {
            send_head(stream, 200, &[("Content-Length", body.len().to_string())])?;
            stream.write_all(&body)
        }
        Some(Reply::Redirect(location)) => send_head(
            stream,
            302,
            &[("Location", location), ("Content-Length", "0".to_owned())],
        ),
        Some(Reply::RedirectWithoutLocation) => {
            send_head(stream, 302, &[("Content-Length", "0".to_owned())])
        }
        Some(Reply::DeclaredLength { body, declared }) => {
            send_head(stream, 200, &[("Content-Length", declared.to_string())])?;
            stream.write_all(&body)
        }
        Some(Reply::NoLength(body)) => {
            send_head(stream, 200, &[])?;
            stream.write_all(&body)
        }
        Some(Reply::Gated { body, pause_after }) => {
            send_head(stream, 200, &[("Content-Length", body.len().to_string())])?;
            stream.write_all(&body[..pause_after])?;
            stream.flush()?;
            while !shared.released.load(Ordering::SeqCst) {
                if shared.stopping.load(Ordering::SeqCst) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(5));
            }
            stream.write_all(&body[pause_after..])
        }
        Some(Reply::Trickle { body, chunk, delay }) => {
            send_head(stream, 200, &[("Content-Length", body.len().to_string())])?;
            for piece in body.chunks(chunk) {
                if shared.stopping.load(Ordering::SeqCst) {
                    return Ok(());
                }
                stream.write_all(piece)?;
                stream.flush()?;
                thread::sleep(delay);
            }
            Ok(())
        }
    }
}

fn send_status(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    let body = format!("status {status}\n");
    send_head(
        stream,
        status,
        &[("Content-Length", body.len().to_string())],
    )?;
    stream.write_all(body.as_bytes())
}

fn send_head(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, String)],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())
}
