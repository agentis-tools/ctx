//! Hand-rolled JSON-RPC 2.0 transport with LSP `Content-Length` framing.
//!
//! Deliberately synchronous: the indexing path is sync (rayon + rusqlite), so
//! the transport uses a background reader thread, a background writer thread,
//! and `mpsc` channels instead of an async runtime. Generic over
//! `Read`/`Write` so it is unit-testable with in-memory pipes.
//!
//! All writes go through a bounded queue owned by the writer thread. This
//! keeps every caller-facing operation time-bounded: if the server stops
//! draining its stdin (wedged process, SIGSTOP) a plain `write_all` larger
//! than the OS pipe buffer would block forever — and would do so while
//! holding a writer lock, so even the failure accounting that kills the child
//! could never run. With the queue, senders give up at their deadline with
//! [`TransportError::Timeout`] and the client's consecutive-timeout logic can
//! kill the child, which breaks the pipe and unblocks the writer thread.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Transport-level failures surfaced to the client layer.
#[derive(Debug)]
pub enum TransportError {
    /// No response arrived within the deadline, or the outgoing message could
    /// not even be queued for writing within it (server not draining stdin).
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

/// Maximum outgoing messages queued for the writer thread.
const WRITE_QUEUE_CAPACITY: usize = 64;
/// Enqueue deadline for notifications (which carry no caller timeout).
const NOTIFY_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval while waiting for space in the write queue.
const ENQUEUE_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Maximum accepted `Content-Length` (64 MB). Anything larger is treated as a
/// framing error and kills the connection, instead of letting a corrupt or
/// hostile header drive an unbounded allocation.
pub(crate) const MAX_CONTENT_LENGTH: usize = 64 * 1024 * 1024;

/// Framed JSON-RPC connection over arbitrary byte streams.
pub struct Transport {
    /// Bounded queue of framed bytes; `None` once closed. The writer thread
    /// owns the underlying stream and is the only place blocking writes run.
    write_tx: Option<mpsc::SyncSender<Vec<u8>>>,
    pending: Pending,
    next_id: AtomicI64,
    alive: Arc<AtomicBool>,
    reader_handle: Option<JoinHandle<()>>,
    writer_handle: Option<JoinHandle<()>>,
}

impl Transport {
    /// Start a transport over the given streams. Spawns the background reader
    /// and writer threads immediately.
    pub fn new<R, W>(reader: R, writer: W, verbose: bool) -> Self
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (write_tx, write_rx) = mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE_CAPACITY);

        let writer_handle = {
            let alive = Arc::clone(&alive);
            let pending = Arc::clone(&pending);
            std::thread::spawn(move || {
                writer_loop(write_rx, writer, &alive, &pending);
            })
        };

        let reader_handle = {
            let write_tx = write_tx.clone();
            let pending = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
            std::thread::spawn(move || {
                reader_loop(reader, &write_tx, &pending, &alive, verbose);
            })
        };

        Self {
            write_tx: Some(write_tx),
            pending,
            next_id: AtomicI64::new(1),
            alive,
            reader_handle: Some(reader_handle),
            writer_handle: Some(writer_handle),
        }
    }

    /// Whether the reader thread still considers the connection open.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send a request and wait up to `timeout` for its response. The timeout
    /// covers both queueing the outgoing bytes and waiting for the reply, so
    /// a server that stops reading its stdin surfaces as `Timeout` (feeding
    /// the client's consecutive-timeout kill logic) instead of blocking.
    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, TransportError> {
        if !self.is_alive() {
            return Err(TransportError::Closed);
        }
        let Some(write_tx) = &self.write_tx else {
            return Err(TransportError::Closed);
        };
        let deadline = Instant::now() + timeout;

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let frame = match frame_message(&message) {
            Ok(frame) => frame,
            Err(e) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(e);
            }
        };
        if let Err(e) = enqueue_until(write_tx, frame, deadline) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(TransportError::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Closed),
        }
    }

    /// Send a notification (no response expected). Bounded: gives up with
    /// `Timeout` if the write queue stays full past a fixed deadline.
    pub fn notify(&self, method: &str, params: Value) -> Result<(), TransportError> {
        if !self.is_alive() {
            return Err(TransportError::Closed);
        }
        let Some(write_tx) = &self.write_tx else {
            return Err(TransportError::Closed);
        };
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let frame = frame_message(&message)?;
        enqueue_until(write_tx, frame, Instant::now() + NOTIFY_ENQUEUE_TIMEOUT)
    }

    /// Drop the connection: marks the transport closed and detaches both
    /// background threads.
    pub fn close(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
        // Dropping our sender lets the writer thread exit once the reader
        // thread (which holds the other clone) is gone and the queue drains.
        // If the writer is blocked mid-write it exits when the caller kills
        // the child and the pipe breaks (EPIPE).
        self.write_tx.take();
        // Never join: either thread may be blocked on a wedged peer. They
        // exit on EOF / broken pipe once the child process goes away.
        if let Some(handle) = self.reader_handle.take() {
            drop(handle);
        }
        if let Some(handle) = self.writer_handle.take() {
            drop(handle);
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Serialize one message into `Content-Length`-framed bytes.
fn frame_message(message: &Value) -> Result<Vec<u8>, TransportError> {
    let body = serde_json::to_vec(message).map_err(|e| TransportError::Io(e.to_string()))?;
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Queue framed bytes for the writer thread, retrying until `deadline`.
///
/// Uses `try_send` + a short poll instead of a blocking `send` so a wedged
/// server (full queue that never drains) costs at most the caller's deadline.
fn enqueue_until(
    tx: &mpsc::SyncSender<Vec<u8>>,
    mut frame: Vec<u8>,
    deadline: Instant,
) -> Result<(), TransportError> {
    loop {
        match tx.try_send(frame) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(f)) => {
                if Instant::now() >= deadline {
                    return Err(TransportError::Timeout);
                }
                frame = f;
                std::thread::sleep(ENQUEUE_POLL_INTERVAL);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(TransportError::Closed),
        }
    }
}

/// Writer thread: the only place blocking writes happen. Owns the stream;
/// exits when every sender is gone (transport closed and reader exited) or a
/// write fails (child killed -> EPIPE). A write failure marks the transport
/// dead and wakes in-flight requests so they fail fast.
fn writer_loop<W: Write>(
    rx: mpsc::Receiver<Vec<u8>>,
    mut writer: W,
    alive: &AtomicBool,
    pending: &Pending,
) {
    while let Ok(frame) = rx.recv() {
        if writer
            .write_all(&frame)
            .and_then(|_| writer.flush())
            .is_err()
        {
            alive.store(false, Ordering::SeqCst);
            pending.lock().unwrap().clear();
            break;
        }
    }
    // Dropping the writer closes the child's stdin.
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
    if len > MAX_CONTENT_LENGTH {
        return None; // framing error: connection is treated as dead
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Runs the reader-loop epilogue even if the loop body panics: mark the
/// connection dead and wake in-flight requests so they fail fast.
struct ReaderEpilogue<'a> {
    alive: &'a AtomicBool,
    pending: &'a Pending,
}

impl Drop for ReaderEpilogue<'_> {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
    }
}

/// Background loop: parse incoming messages and dispatch them.
fn reader_loop<R: Read>(
    reader: R,
    write_tx: &mpsc::SyncSender<Vec<u8>>,
    pending: &Pending,
    alive: &AtomicBool,
    verbose: bool,
) {
    let mut reader = BufReader::new(reader);
    let _epilogue = ReaderEpilogue { alive, pending };

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
                // Never block the reader on a full write queue: drop the
                // reply instead. Blocking here would deadlock the transport
                // (the reader is what unblocks pending requests), and a full
                // queue means the server is not draining stdin anyway.
                if let Ok(frame) = frame_message(&reply) {
                    let _ = write_tx.try_send(frame);
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

    /// A writer whose peer never drains: accepts up to `capacity` bytes, then
    /// blocks until `release()` (after which it fails like a broken pipe).
    /// Models a wedged language server that stopped reading its stdin.
    #[derive(Clone)]
    struct StuckWriter {
        capacity: usize,
        written: Arc<Mutex<usize>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
        /// Signals each `write` call (used to sequence tests).
        write_signal: Option<mpsc::Sender<()>>,
    }

    impl StuckWriter {
        fn new(capacity: usize) -> Self {
            StuckWriter {
                capacity,
                written: Arc::new(Mutex::new(0)),
                gate: Arc::new((Mutex::new(false), Condvar::new())),
                write_signal: None,
            }
        }

        fn release(&self) {
            let (lock, cvar) = &*self.gate;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }

    impl Write for StuckWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if let Some(signal) = &self.write_signal {
                let _ = signal.send(());
            }
            let mut written = self.written.lock().unwrap();
            if *written + data.len() > self.capacity {
                drop(written);
                let (lock, cvar) = &*self.gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = cvar.wait(released).unwrap();
                }
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
            }
            *written += data.len();
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

    /// Write one framed message directly onto a pipe (as the server would).
    fn inject(pipe: &Pipe, message: &Value) {
        let body = serde_json::to_vec(message).unwrap();
        let mut writer = pipe.clone();
        writer
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .unwrap();
        writer.write_all(&body).unwrap();
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
        // The peer answers any request of ours; the injected server->client
        // request below is auto-replied by the reader thread.
        let (transport, c2s, s2c, peer) = transport_with_peer(|msg| {
            let id = msg.get("id")?.clone();
            Some(json!({"jsonrpc": "2.0", "id": id, "result": null}))
        });

        // Inject a server->client request by writing it straight onto the
        // server->client pipe.
        inject(
            &s2c,
            &json!({
                "jsonrpc": "2.0",
                "id": 999,
                "method": "workspace/configuration",
                "params": { "items": [ {"section": "a"}, {"section": "b"} ] },
            }),
        );

        // Our own request still completes (correlation by id skips the auto-
        // reply traffic).
        transport
            .request("first", json!({}), Duration::from_secs(2))
            .unwrap();
        // Writes are asynchronous now: a second request fences the queue.
        // The reader saw the 999 request before it dispatched the response
        // to "first", so its auto-reply is queued before "second" — once the
        // peer answered "second" the auto-reply has been written.
        transport
            .request("second", json!({}), Duration::from_secs(2))
            .unwrap();

        c2s.close();
        s2c.close();
        let received = peer.join().unwrap();
        // The peer saw our requests plus the auto-reply to id 999 with one
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
        inject(
            &s2c,
            &json!({
                "jsonrpc": "2.0",
                "method": "window/logMessage",
                "params": { "type": 3, "message": "hello" },
            }),
        );

        let value = transport
            .request("ping", json!({}), Duration::from_secs(2))
            .unwrap();
        assert_eq!(value, json!(42));

        transport.notify("initialized", json!({})).unwrap();
        // Writes are asynchronous: fence the queued notification with a
        // request the peer answers before closing the pipes.
        transport
            .request("after", json!({}), Duration::from_secs(2))
            .unwrap();

        c2s.close();
        s2c.close();
        let received = peer.join().unwrap();
        assert!(received.iter().any(|m| m["method"] == "initialized"));
        drop(transport);
    }

    /// A wedged server (never drains stdin) must surface as `Timeout` on the
    /// caller — never as an indefinitely blocked `request()`. Guarded by its
    /// own watchdog so a regression fails instead of hanging the test run.
    #[test]
    fn request_times_out_when_server_stops_draining_stdin() {
        let s2c = Pipe::new();
        let writer = StuckWriter::new(8); // smaller than any framed message
        let release_handle = writer.clone();

        let (done_tx, done_rx) = mpsc::channel();
        let reader = s2c.clone();
        std::thread::spawn(move || {
            let transport = Transport::new(reader, writer, false);
            let result = transport.request("hang", json!({}), Duration::from_millis(300));
            done_tx.send(result).unwrap();
        });

        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("request() must not block on a wedged server");
        assert!(
            matches!(result, Err(TransportError::Timeout)),
            "got {result:?}"
        );

        release_handle.release();
        s2c.close();
    }

    /// When the write queue itself is full (writer blocked, queue at
    /// capacity), enqueueing must give up at the caller's deadline.
    #[test]
    fn full_write_queue_times_out_request_instead_of_blocking() {
        let s2c = Pipe::new();
        let writer = StuckWriter::new(0); // first write blocks immediately
        let release_handle = writer.clone();

        let (done_tx, done_rx) = mpsc::channel();
        let reader = s2c.clone();
        std::thread::spawn(move || {
            let transport = Transport::new(reader, writer, false);
            // Fill the queue: the writer thread is stuck on the first frame,
            // the rest sit in the bounded channel.
            for _ in 0..(WRITE_QUEUE_CAPACITY + 1) {
                let _ = transport.notify("noise", json!({}));
            }
            let result = transport.request("hang", json!({}), Duration::from_millis(200));
            done_tx.send(result).unwrap();
        });

        let result = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("request() must not block on a full write queue");
        assert!(
            matches!(result, Err(TransportError::Timeout)),
            "got {result:?}"
        );

        release_handle.release();
        s2c.close();
    }

    /// The reader thread must keep dispatching responses even when the write
    /// queue is full and it cannot send auto-replies (they are dropped).
    #[test]
    fn reader_auto_replies_never_deadlock_against_full_write_queue() {
        let s2c = Pipe::new();
        let (write_signal_tx, write_signal_rx) = mpsc::channel();
        let mut writer = StuckWriter::new(0); // wedged from the first byte
        writer.write_signal = Some(write_signal_tx);
        let release_handle = writer.clone();

        let (done_tx, done_rx) = mpsc::channel();
        let reader = s2c.clone();
        std::thread::spawn(move || {
            let transport = Transport::new(reader, writer, false);
            // id 1: enqueued, then the writer thread blocks on it forever.
            let result = transport.request("ping", json!({}), Duration::from_secs(10));
            done_tx.send(result).unwrap();
        });

        // Wait until the writer thread picked up the request frame (so the
        // pending entry exists), then flood the reader with server->client
        // requests: auto-replies must be dropped, never block the reader.
        write_signal_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("writer never saw the request frame");
        for i in 0..(WRITE_QUEUE_CAPACITY * 2) {
            inject(
                &s2c,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 1000 + i,
                    "method": "workspace/configuration",
                    "params": { "items": [] },
                }),
            );
        }
        // The response to our request must still be dispatched.
        inject(&s2c, &json!({ "jsonrpc": "2.0", "id": 1, "result": 42 }));

        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reader deadlocked against the full write queue");
        assert_eq!(result.unwrap(), json!(42));

        release_handle.release();
        s2c.close();
    }

    #[test]
    fn oversized_content_length_is_a_framing_error() {
        let len = MAX_CONTENT_LENGTH + 1;
        let framed = format!("Content-Length: {len}\r\n\r\nx");
        let mut reader = std::io::Cursor::new(framed.into_bytes());
        assert!(read_message(&mut reader).is_none());

        // Sanity: a normal message still parses.
        let body = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let mut ok = Vec::new();
        ok.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        ok.extend_from_slice(body);
        let mut reader = std::io::Cursor::new(ok);
        assert!(read_message(&mut reader).is_some());
    }
}
