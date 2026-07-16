//! Language-server child process management and typed request wrappers.
//!
//! One [`LspClient`] per configured language. The client owns the child
//! process, its framed [`Transport`], and a drained-stderr ring buffer, and
//! tracks failure state (consecutive timeouts, broken pipes) so callers can
//! fall back to tree-sitter without ever failing the indexing run.

use std::collections::VecDeque;
use std::io::BufRead;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lsp_types::{
    CallHierarchyItem, CallHierarchyOutgoingCall, DocumentSymbolResponse, GotoDefinitionResponse,
    Location,
};
use serde_json::{json, Value};

use super::config::LspServerConfig;
use super::path_to_uri;
use super::transport::{Transport, TransportError};

/// Why a server could not be started, plus any stderr it left behind.
#[derive(Debug)]
pub struct SpawnError {
    pub reason: String,
    pub stderr: Vec<String>,
}

/// Default per-request timeout (overridable via `timeout_ms`).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for the `initialize` handshake.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
/// One-time grace period for the first request after `initialize` (server
/// warmup: many servers index the workspace before answering).
const WARMUP_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for the `shutdown` request and for waiting on process exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// Consecutive request timeouts before the server is declared failed.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 3;
/// Lines of server stderr retained for diagnostics.
const STDERR_RING_CAPACITY: usize = 40;

/// A running (or failed) language server connection.
pub struct LspClient {
    transport: Transport,
    child: Child,
    stderr_lines: Arc<Mutex<VecDeque<String>>>,
    request_timeout: Duration,
    /// Whether `timeout_ms` was configured explicitly. An explicit timeout is
    /// honored as-is (no warmup extension), so tests and impatient users get
    /// deterministic deadlines.
    explicit_timeout: bool,
    /// `Some(reason)` once the server has been declared unusable.
    failed: Option<String>,
    consecutive_timeouts: u32,
    /// The first request after `initialize` gets a longer warmup deadline
    /// (unless the timeout was configured explicitly).
    warmup_pending: bool,
    shut_down: bool,
    /// Server-reported name from `initialize` (if any).
    pub server_name: Option<String>,
    /// Server-reported version from `initialize` (if any).
    pub server_version: Option<String>,
    /// Raw negotiated `capabilities` object from `initialize`.
    capabilities: Value,
}

impl LspClient {
    /// Spawn the configured server and run the `initialize`/`initialized`
    /// handshake. Returns a human-readable reason (plus captured stderr) on
    /// failure.
    pub fn spawn(config: &LspServerConfig, root: &Path, verbose: bool) -> Result<Self, SpawnError> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|e| SpawnError {
            reason: format!("failed to spawn `{}`: {e}", config.command),
            stderr: Vec::new(),
        })?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take().expect("piped stdin");
        let stderr = child.stderr.take().expect("piped stderr");

        // Drain stderr into a bounded ring buffer for diagnostics.
        let stderr_lines = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_RING_CAPACITY)));
        {
            let stderr_lines = Arc::clone(&stderr_lines);
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines().map_while(|l| l.ok()) {
                    let mut ring = stderr_lines.lock().unwrap();
                    if ring.len() >= STDERR_RING_CAPACITY {
                        ring.pop_front();
                    }
                    ring.push_back(line);
                }
            });
        }

        let transport = Transport::new(stdout, stdin, verbose);

        let mut client = Self {
            transport,
            child,
            stderr_lines,
            request_timeout: config
                .timeout_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT),
            explicit_timeout: config.timeout_ms.is_some(),
            failed: None,
            consecutive_timeouts: 0,
            warmup_pending: true,
            shut_down: false,
            server_name: None,
            server_version: None,
            capabilities: Value::Null,
        };

        client.initialize(config, root).map_err(|reason| {
            let stderr = client.recent_stderr();
            client.kill();
            SpawnError { reason, stderr }
        })?;

        Ok(client)
    }

    /// Run the `initialize` request and `initialized` notification.
    fn initialize(&mut self, config: &LspServerConfig, root: &Path) -> Result<(), String> {
        let root_uri = path_to_uri(root);
        let mut params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "workspaceFolders": [{
                "uri": root_uri,
                "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace"),
            }],
            "capabilities": {
                "textDocument": {
                    "documentSymbol": {
                        "hierarchicalDocumentSymbolSupport": true,
                    },
                    "definition": { "linkSupport": true },
                    "implementation": { "linkSupport": true },
                    "references": {},
                    "callHierarchy": {},
                    "synchronization": {
                        "didSave": false,
                        "willSave": false,
                        "willSaveWaitUntil": false,
                    },
                },
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true,
                },
            },
        });
        if let Some(options) = &config.initialization_options {
            params["initializationOptions"] = options.clone();
        }

        let result = self
            .transport
            .request("initialize", params, INITIALIZE_TIMEOUT)
            .map_err(|e| format!("initialize failed: {e}"))?;

        self.server_name = result
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.server_version = result
            .pointer("/serverInfo/version")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);

        self.transport
            .notify("initialized", json!({}))
            .map_err(|e| format!("initialized notification failed: {e}"))?;

        Ok(())
    }

    /// Whether the server has been declared unusable, and why.
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Server name/version reported during `initialize`.
    pub fn server_info(&self) -> (Option<&str>, Option<&str>) {
        (self.server_name.as_deref(), self.server_version.as_deref())
    }

    /// Names of negotiated server capabilities (keys of the `capabilities`
    /// object whose value is not `false`/`null`).
    pub fn capability_names(&self) -> Vec<String> {
        match &self.capabilities {
            Value::Object(map) => map
                .iter()
                .filter(|(_, v)| !matches!(v, Value::Null | Value::Bool(false)))
                .map(|(k, _)| k.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the server negotiated a truthy capability under `key`
    /// (e.g. `"documentSymbolProvider"`).
    pub fn supports(&self, key: &str) -> bool {
        !matches!(
            self.capabilities.get(key),
            None | Some(Value::Null) | Some(Value::Bool(false))
        )
    }

    /// The most recent stderr lines from the server process.
    pub fn recent_stderr(&self) -> Vec<String> {
        self.stderr_lines.lock().unwrap().iter().cloned().collect()
    }

    /// Send a request with failure accounting (timeouts, broken pipes).
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if let Some(reason) = &self.failed {
            return Err(reason.clone());
        }

        let timeout = if self.warmup_pending && !self.explicit_timeout {
            self.request_timeout.max(WARMUP_TIMEOUT)
        } else {
            self.request_timeout
        };

        match self.transport.request(method, params, timeout) {
            Ok(value) => {
                self.warmup_pending = false;
                self.consecutive_timeouts = 0;
                Ok(value)
            }
            Err(TransportError::Timeout) => {
                self.warmup_pending = false;
                self.consecutive_timeouts += 1;
                if self.consecutive_timeouts >= MAX_CONSECUTIVE_TIMEOUTS {
                    self.fail(format!(
                        "{MAX_CONSECUTIVE_TIMEOUTS} consecutive request timeouts (last: {method})"
                    ));
                }
                Err(format!("{method} timed out"))
            }
            Err(TransportError::Closed) => {
                let reason = format!("connection lost during {method} (server exited?)");
                self.fail(reason.clone());
                Err(reason)
            }
            Err(TransportError::Io(e)) => {
                let reason = format!("write failed during {method}: {e}");
                self.fail(reason.clone());
                Err(reason)
            }
            // A JSON-RPC error is a *response*: the server is alive, the
            // request just isn't supported/valid. Don't fail the client.
            Err(TransportError::Rpc(e)) => {
                self.warmup_pending = false;
                self.consecutive_timeouts = 0;
                Err(format!("{method}: {e}"))
            }
        }
    }

    /// Declare the server unusable and kill the process.
    fn fail(&mut self, reason: String) {
        if self.failed.is_none() {
            self.failed = Some(reason);
        }
        self.kill();
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.shut_down = true;
    }

    // ---- Notifications -----------------------------------------------------

    /// `textDocument/didOpen` with full text sync.
    ///
    /// A failed write means the server is gone (e.g. it crashed right after
    /// `initialize`): the client is marked failed so the caller falls back
    /// with the usual once-per-language warning.
    pub fn did_open(&mut self, uri: &str, language_id: &str, text: &str) -> Result<(), String> {
        if let Some(reason) = &self.failed {
            return Err(reason.clone());
        }
        match self.transport.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                let reason = format!("connection lost during textDocument/didOpen: {e}");
                self.fail(reason.clone());
                Err(reason)
            }
        }
    }

    /// `textDocument/didClose`.
    pub fn did_close(&mut self, uri: &str) {
        if self.failed.is_some() {
            return;
        }
        let _ = self.transport.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        );
    }

    // ---- Typed request wrappers --------------------------------------------

    /// `textDocument/documentSymbol`. `Ok(None)` means the server returned
    /// `null` (no symbols).
    pub fn document_symbols(
        &mut self,
        uri: &str,
    ) -> Result<Option<DocumentSymbolResponse>, String> {
        let result = self.request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        serde_json::from_value(result)
            .map(Some)
            .map_err(|e| format!("malformed documentSymbol response: {e}"))
    }

    /// `textDocument/definition` at a 0-based position. Returns the first
    /// resulting location, if any.
    pub fn definition(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<(String, u32)>, String> {
        let result = self.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        let response: GotoDefinitionResponse = serde_json::from_value(result)
            .map_err(|e| format!("malformed definition response: {e}"))?;
        Ok(first_definition_target(response))
    }

    /// `textDocument/prepareCallHierarchy` at a 0-based position.
    pub fn prepare_call_hierarchy(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyItem>, String> {
        let result = self.request(
            "textDocument/prepareCallHierarchy",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        )?;
        if result.is_null() {
            return Ok(Vec::new());
        }
        serde_json::from_value(result)
            .map_err(|e| format!("malformed prepareCallHierarchy response: {e}"))
    }

    /// `callHierarchy/outgoingCalls` for a prepared item.
    pub fn outgoing_calls(
        &mut self,
        item: &CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyOutgoingCall>, String> {
        let item_value = serde_json::to_value(item).map_err(|e| format!("serialize item: {e}"))?;
        let result = self.request("callHierarchy/outgoingCalls", json!({ "item": item_value }))?;
        if result.is_null() {
            return Ok(Vec::new());
        }
        serde_json::from_value(result).map_err(|e| format!("malformed outgoingCalls response: {e}"))
    }

    /// `textDocument/implementation` at a 0-based position.
    pub fn implementation(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Option<(String, u32)>, String> {
        let result = self.request(
            "textDocument/implementation",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
            }),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        let response: GotoDefinitionResponse = serde_json::from_value(result)
            .map_err(|e| format!("malformed implementation response: {e}"))?;
        Ok(first_definition_target(response))
    }

    /// `textDocument/references` at a 0-based position.
    pub fn references(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<(String, u32)>, String> {
        let result = self.request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": false },
            }),
        )?;
        if result.is_null() {
            return Ok(Vec::new());
        }
        let locations: Vec<Location> = serde_json::from_value(result)
            .map_err(|e| format!("malformed references response: {e}"))?;
        Ok(locations.iter().map(location_target).collect())
    }

    /// Graceful shutdown: `shutdown` request (2s) -> `exit` notification ->
    /// bounded wait -> kill. Safe to call more than once.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;

        if self.failed.is_none() {
            let _ = self
                .transport
                .request("shutdown", Value::Null, SHUTDOWN_TIMEOUT);
            let _ = self.transport.notify("exit", Value::Null);
        }

        // Bounded wait for the process to exit, then kill.
        let deadline = std::time::Instant::now() + SHUTDOWN_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Reduce a definition-style response to `(uri, 0-based start line)` of its
/// first target.
fn first_definition_target(response: GotoDefinitionResponse) -> Option<(String, u32)> {
    match response {
        GotoDefinitionResponse::Scalar(location) => Some(location_target(&location)),
        GotoDefinitionResponse::Array(locations) => locations.first().map(location_target),
        GotoDefinitionResponse::Link(links) => links.first().map(|link| {
            (
                link.target_uri.as_str().to_string(),
                link.target_selection_range.start.line,
            )
        }),
    }
}

fn location_target(location: &Location) -> (String, u32) {
    (location.uri.as_str().to_string(), location.range.start.line)
}
