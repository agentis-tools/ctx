//! Test utilities shared between the library and binary test suites.
//!
//! This module is always compiled (not `#[cfg(test)]`) so that integration
//! tests in the binary crate can use it, but it is `#[doc(hidden)]` and not
//! part of the supported public API.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A scratch git repository for tests.
///
/// All git operations run with the repository root as the working directory,
/// so tests never need to change the process-wide current directory.
pub struct GitRepo {
    pub root: PathBuf,
}

impl GitRepo {
    /// Initialize a fresh git repository in `dir` with a local test identity.
    pub fn init(dir: &Path) -> GitRepo {
        let repo = GitRepo {
            root: dir.to_path_buf(),
        };
        std::fs::create_dir_all(dir).expect("failed to create repo dir");
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "user.name", "Test User"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    /// Write a file (creating parent directories) relative to the repo root.
    pub fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, content).expect("failed to write file");
    }

    /// Stage everything and commit with the given message.
    pub fn commit_all(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", msg]);
    }

    /// Write a single file and commit it.
    pub fn commit_file(&self, rel: &str, content: &str, msg: &str) {
        self.write(rel, content);
        self.commit_all(msg);
    }

    /// Stage everything and commit with a fixed author/committer date.
    ///
    /// `date` is any format git accepts for `GIT_AUTHOR_DATE`, e.g.
    /// `"2020-01-01T12:00:00 +0000"`. Useful for testing `--since` filters.
    pub fn commit_all_with_date(&self, msg: &str, date: &str) {
        self.git(&["add", "-A"]);
        self.git_with_env(
            &["commit", "-q", "--no-gpg-sign", "-m", msg],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    }

    /// Create and switch to a new branch.
    pub fn branch(&self, name: &str) {
        self.git(&["checkout", "-q", "-b", name]);
    }

    /// Run a git command in the repo root, panicking on failure.
    fn git(&self, args: &[&str]) {
        self.git_with_env(args, &[]);
    }

    /// Run a git command with extra environment variables, panicking on failure.
    fn git_with_env(&self, args: &[&str], envs: &[(&str, &str)]) {
        let output = Command::new("git")
            .args(args)
            .envs(envs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .current_dir(&self.root)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// ============================================================================
// Minimal HTTP mock server (for update/self-update tests)
// ============================================================================

/// One canned route: exact request path -> response body.
struct MockRoute {
    path: String,
    content_type: String,
    body: Vec<u8>,
}

/// A minimal, hand-rolled HTTP/1.1 server for tests (no dev-dependencies).
///
/// Serves canned responses for exact paths (404 otherwise), counts every
/// request it receives, and shuts down on drop. Used by the self-update
/// tests together with the `CTX_UPDATE_BASE_URL` override.
pub struct MockServer {
    addr: SocketAddr,
    routes: Arc<Mutex<Vec<MockRoute>>>,
    hits: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Bind to an ephemeral localhost port and start serving.
    pub fn start() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind mock server");
        listener
            .set_nonblocking(true)
            .expect("failed to set nonblocking");
        let addr = listener.local_addr().expect("no local addr");

        let routes: Arc<Mutex<Vec<MockRoute>>> = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let handle = {
            let routes = Arc::clone(&routes);
            let hits = Arc::clone(&hits);
            let shutdown = Arc::clone(&shutdown);
            std::thread::spawn(move || {
                while !shutdown.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = serve_connection(stream, &routes, &hits);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };

        MockServer {
            addr,
            routes,
            hits,
            shutdown,
            handle: Some(handle),
        }
    }

    /// Base URL of the server, e.g. `http://127.0.0.1:49152`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Register a canned response for an exact request path (e.g. `/x/y`).
    pub fn add_route(&self, path: &str, content_type: &str, body: impl Into<Vec<u8>>) {
        self.routes.lock().unwrap().push(MockRoute {
            path: path.to_string(),
            content_type: content_type.to_string(),
            body: body.into(),
        });
    }

    /// Total number of requests received so far (matched or not).
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    routes: &Mutex<Vec<MockRoute>>,
    hits: &AtomicUsize,
) -> std::io::Result<()> {
    // Accepted sockets can inherit the listener's nonblocking mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;

    // Read the request head (GET requests only; no body expected).
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..n]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 64 * 1024 {
            break;
        }
    }
    if request.is_empty() {
        return Ok(());
    }
    hits.fetch_add(1, Ordering::SeqCst);

    let head = String::from_utf8_lossy(&request);
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();

    let routes = routes.lock().unwrap();
    match routes.iter().find(|r| r.path == path) {
        Some(route) => {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                route.content_type,
                route.body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(&route.body)?;
        }
        None => {
            let body = b"not found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(body)?;
        }
    }
    stream.flush()
}
