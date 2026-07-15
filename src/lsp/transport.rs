//! Hand-rolled JSON-RPC 2.0 transport with LSP `Content-Length` framing.
//!
//! Deliberately synchronous: the indexing path is sync (rayon + rusqlite), so
//! the transport uses a background reader thread plus `mpsc` channels instead
//! of an async runtime. Generic over `Read`/`Write` so it is unit-testable
//! with in-memory pipes.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

/// Transport-level failures surfaced to the client layer.
#[derive(Debug)]
pub enum TransportError {
    /// No response arrived within the deadline.
    Timeout,
    /// The peer closed the connection (EOF/broken pipe) or the transport was
    /// already shut down.
    Closed,
    /// The server answered with a JSON-RPC error object.
    Rpc(String),
    /// A message could not be written.
    Io(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Timeout => write!(f, "request timed out"),
            TransportError::Closed => write!(f, "connection closed"),
            TransportError::Rpc(e) => write!(f, "server error: {e}"),
            TransportError::Io(e) => write!(f, "write failed: {e}"),
        }
    }
}

type Pending = Arc<Mutex<HashMap<i64, mpsc::Sender<Result<Value, TransportError>>>>>;
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Framed JSON-RPC connection over arbitrary byte streams.
pub struct Transport {
    writer: SharedWriter,
    pending: Pending,
    next_id: AtomicI64,
    alive: Arc<AtomicBool>,
    reader_handle: Option<JoinHandle<()>>,
}

impl Transport {
    /// Start a transport over the given streams. Spawns the background reader
    /// thread immediately.
    pub fn new<R, W>(reader: R, writer: W, verbose: bool) -> Self
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(writer)));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));

        let reader_handle = {
            let writer = Arc::clone(&writer);
            let pending = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
            std::thread::spawn(move || {
                reader_loop(reader, writer, pending, &alive, verbose);
            })
        };

        Self {
            writer,
            pending,
            next_id: AtomicI64::new(1),
            alive,
            reader_handle: Some(reader_handle),
        }
    }

    /// Whether the reader thread still considers the connection open.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send a request and wait up to `timeout` for its response.
    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, TransportError> {
        if !self.is_alive() {
            return Err(TransportError::Closed);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        if let Err(e) = write_message(&self.writer, &message) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }

        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(TransportError::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Closed),
        }
    }

    /// Send a notification (no response expected).
    pub fn notify(&self, method: &str, params: Value) -> Result<(), TransportError> {
        if !self.is_alive() {
            return Err(TransportError::Closed);
        }
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        write_message(&self.writer, &message)?;
        Ok(())
    }

    /// Drop the connection: marks the transport closed and detaches the
    /// reader thread (it exits on EOF once the peer goes away).
    pub fn close(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
        // The reader thread blocks on `read` until the peer closes its end
        // (the caller kills the child process); don't join, just detach.
        if let Some(handle) = self.reader_handle.take() {
            drop(handle);
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Write one `Content-Length`-framed JSON-RPC message.
fn write_message(writer: &SharedWriter, message: &Value) -> Result<(), TransportError> {
    let body = serde_json::to_vec(message).map_err(|e| TransportError::Io(e.to_string()))?;
    let mut w = writer.lock().unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(header.as_bytes())
        .and_then(|_| w.write_all(&body))
        .and_then(|_| w.flush())
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                TransportError::Closed
            } else {
                TransportError::Io(e.to_string())
            }
        })
}

/// Read one framed message; `None` on EOF or malformed framing.
pub(crate) fn read_message<R: BufRead>(reader: &mut R) -> Option<Value> {
    let mut content_length: Option<usize> = None;

    // Headers: `Name: value\r\n` pairs terminated by an empty line.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None; // EOF
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok();
            }
        }
    }

    let len = content_length?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Background loop: parse incoming messages and dispatch them.
fn reader_loop<R: Read>(
    reader: R,
    writer: SharedWriter,
    pending: Pending,
    alive: &AtomicBool,
    verbose: bool,
) {
    let mut reader = BufReader::new(reader);

    while alive.load(Ordering::SeqCst) {
        let Some(message) = read_message(&mut reader) else {
            break; // EOF or framing error: connection is gone
        };

        let has_method = message.get("method").is_some();
        let id = message.get("id").cloned();

        match (has_method, id) {
            // Server -> client request: auto-reply so the server never stalls.
            (true, Some(id)) => {
                let method = message["method"].as_str().unwrap_or("");
                let result = auto_reply(method, message.get("params"));
                let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                if write_message(&writer, &reply).is_err() {
                    break;
                }
            }
            // Notification: dropped (logged under verbose for log messages).
            (true, None) => {
                if verbose {
                    let method = message["method"].as_str().unwrap_or("");
                    if method == "window/logMessage" || method == "window/showMessage" {
                        let text = message
                            .pointer("/params/message")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        eprintln!("lsp: {text}");
                    }
                }
            }
            // Response to one of our requests.
            (false, Some(id)) => {
                if let Some(id) = id.as_i64() {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let outcome = if let Some(error) = message.get("error") {
                            let text = error
                                .get("message")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .unwrap_or_else(|| error.to_string());
                            Err(TransportError::Rpc(text))
                        } else {
                            Ok(message.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = tx.send(outcome);
                    }
                }
            }
            (false, None) => {} // malformed; ignore
        }
    }

    alive.store(false, Ordering::SeqCst);
    // Wake up any in-flight requests so they fail fast instead of timing out.
    pending.lock().unwrap().clear();
}

/// Canned replies for server -> client requests we don't implement.
fn auto_reply(method: &str, params: Option<&Value>) -> Value {
    match method {
        // Reply with one `null` per requested configuration item.
        "workspace/configuration" => {
            let count = params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map(|items| items.len())
                .unwrap_or(1);
            Value::Array(vec![Value::Null; count])
        }
        // Acknowledged with a null result.
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => Value::Null,
        "workspace/workspaceFolders" => Value::Null,
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Condvar;

    /// A blocking in-memory byte pipe (one direction).
    #[derive(Clone)]
    struct Pipe {
        inner: Arc<(Mutex<PipeState>, Condvar)>,
    }

    struct PipeState {
        buf: VecDeque<u8>,
        closed: bool,
    }

    impl Pipe {
        fn new() -> Self {
            Pipe {
                inner: Arc::new((
                    Mutex::new(PipeState {
                        buf: VecDeque::new(),
                        closed: false,
                    }),
                    Condvar::new(),
                )),
            }
        }

        fn close(&self) {
            let (lock, cvar) = &*self.inner;
            lock.lock().unwrap().closed = true;
            cvar.notify_all();
        }
    }

    impl Read for Pipe {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let (lock, cvar) = &*self.inner;
            let mut state = lock.lock().unwrap();
            while state.buf.is_empty() && !state.closed {
                state = cvar.wait(state).unwrap();
            }
            if state.buf.is_empty() {
                return Ok(0); // EOF
            }
            let n = out.len().min(state.buf.len());
            for slot in out.iter_mut().take(n) {
                *slot = state.buf.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let (lock, cvar) = &*self.inner;
            let mut state = lock.lock().unwrap();
            if state.closed {
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
            }
            state.buf.extend(data.iter().copied());
            cvar.notify_all();
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Spawn a scripted peer: reads framed messages from `incoming` and
    /// passes them to `handler`, writing any returned messages to `outgoing`.
    fn spawn_peer<F>(incoming: Pipe, outgoing: Pipe, mut handler: F) -> JoinHandle<Vec<Value>>
    where
        F: FnMut(&Value) -> Option<Value> + Send + 'static,
    {
        std::thread::spawn(move || {
            let mut received = Vec::new();
            let mut reader = BufReader::new(incoming);
            let mut out = outgoing;
            while let Some(msg) = read_message(&mut reader) {
                if let Some(reply) = handler(&msg) {
                    let body = serde_json::to_vec(&reply).unwrap();
                    let header = format!("Content-Length: {}\r\n\r\n", body.len());
                    out.write_all(header.as_bytes()).unwrap();
                    out.write_all(&body).unwrap();
                }
                received.push(msg);
            }
            received
        })
    }

    /// Transport connected to a scripted in-memory peer.
    fn transport_with_peer<F>(handler: F) -> (Transport, Pipe, Pipe, JoinHandle<Vec<Value>>)
    where
        F: FnMut(&Value) -> Option<Value> + Send + 'static,
    {
        let client_to_server = Pipe::new();
        let server_to_client = Pipe::new();
        let peer = spawn_peer(client_to_server.clone(), server_to_client.clone(), handler);
        let transport = Transport::new(server_to_client.clone(), client_to_server.clone(), false);
        (transport, client_to_server, server_to_client, peer)
    }

    #[test]
    fn request_response_roundtrip_and_correlation() {
        let (transport, c2s, s2c, peer) = transport_with_peer(|msg| {
            // Echo the request id back with a method-specific payload.
            let id = msg.get("id")?.clone();
            let method = msg["method"].as_str().unwrap_or("");
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "echo": method },
            }))
        });

        let a = transport
            .request("alpha", json!({}), Duration::from_secs(2))
            .unwrap();
        let b = transport
            .request("beta", json!({"x": 1}), Duration::from_secs(2))
            .unwrap();
        assert_eq!(a["echo"], "alpha");
        assert_eq!(b["echo"], "beta");

        c2s.close();
        s2c.close();
        let received = peer.join().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0]["method"], "alpha");
        assert_eq!(received[0]["jsonrpc"], "2.0");
        assert_eq!(received[1]["params"]["x"], 1);
        drop(transport);
    }

    #[test]
    fn timeout_when_server_never_responds() {
        let (transport, c2s, s2c, peer) = transport_with_peer(|_| None);

        let err = transport
            .request("hang", json!({}), Duration::from_millis(100))
            .unwrap_err();
        assert!(matches!(err, TransportError::Timeout), "got {err:?}");

        c2s.close();
        s2c.close();
        peer.join().unwrap();
        drop(transport);
    }

    #[test]
    fn rpc_error_is_surfaced() {
        let (transport, c2s, s2c, peer) = transport_with_peer(|msg| {
            let id = msg.get("id")?.clone();
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" },
            }))
        });

        let err = transport
            .request("nope", json!({}), Duration::from_secs(2))
            .unwrap_err();
        match err {
            TransportError::Rpc(text) => assert!(text.contains("method not found")),
            other => panic!("expected Rpc error, got {other:?}"),
        }

        c2s.close();
        s2c.close();
        peer.join().unwrap();
        drop(transport);
    }

    #[test]
    fn auto_replies_to_server_requests() {
        // The peer sends a workspace/configuration request as soon as it sees
        // our request, then answers our request afterwards.
        let (transport, c2s, s2c, peer) = transport_with_peer(|msg| {
            let id = msg.get("id")?.clone();
            if msg["method"] == "first" {
                // Two messages: a server->client request, then our response.
                // spawn_peer writes only one message per handler call, so
                // pack them by returning the server request first and the
                // response on a later call is not possible; instead reply to
                // "first" directly and let the dedicated test below cover the
                // config request path.
                Some(json!({"jsonrpc": "2.0", "id": id, "result": null}))
            } else {
                None
            }
        });

        // Inject a server->client request by writing it straight onto the
        // server->client pipe.
        {
            let server_request = json!({
                "jsonrpc": "2.0",
                "id": 999,
                "method": "workspace/configuration",
                "params": { "items": [ {"section": "a"}, {"section": "b"} ] },
            });
            let body = serde_json::to_vec(&server_request).unwrap();
            let mut s2c_writer = s2c.clone();
            s2c_writer
                .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
                .unwrap();
            s2c_writer.write_all(&body).unwrap();
        }

        // Our own request still completes (correlation by id skips the auto-
        // reply traffic).
        transport
            .request("first", json!({}), Duration::from_secs(2))
            .unwrap();

        c2s.close();
        s2c.close();
        let received = peer.join().unwrap();
        // The peer saw our request plus the auto-reply to id 999 with one
        // null per configuration item.
        let reply = received
            .iter()
            .find(|m| m.get("id").and_then(Value::as_i64) == Some(999))
            .expect("auto-reply for workspace/configuration");
        assert_eq!(reply["result"], json!([null, null]));
        drop(transport);
    }

    #[test]
    fn eof_marks_transport_closed() {
        let (transport, c2s, s2c, peer) = transport_with_peer(|_| None);
        s2c.close();
        c2s.close();
        peer.join().unwrap();

        // Give the reader thread a moment to observe EOF.
        for _ in 0..100 {
            if !transport.is_alive() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!transport.is_alive());
        let err = transport
            .request("late", json!({}), Duration::from_millis(100))
            .unwrap_err();
        assert!(matches!(err, TransportError::Closed));
        drop(transport);
    }

    #[test]
    fn notifications_are_dropped_without_stalling() {
        let (transport, c2s, s2c, peer) = transport_with_peer(|msg| {
            let id = msg.get("id")?.clone();
            Some(json!({"jsonrpc": "2.0", "id": id, "result": 42}))
        });

        // A notification from the server must not disturb correlation.
        {
            let notification = json!({
                "jsonrpc": "2.0",
                "method": "window/logMessage",
                "params": { "type": 3, "message": "hello" },
            });
            let body = serde_json::to_vec(&notification).unwrap();
            let mut s2c_writer = s2c.clone();
            s2c_writer
                .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
                .unwrap();
            s2c_writer.write_all(&body).unwrap();
        }

        let value = transport
            .request("ping", json!({}), Duration::from_secs(2))
            .unwrap();
        assert_eq!(value, json!(42));

        transport.notify("initialized", json!({})).unwrap();

        c2s.close();
        s2c.close();
        let received = peer.join().unwrap();
        assert!(received.iter().any(|m| m["method"] == "initialized"));
        drop(transport);
    }
}
