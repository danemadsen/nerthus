//! The connections' side of the server: a listener on 127.0.0.1 whose
//! connections each get a thread, HTTP/1.1 requests read off them, and
//! the JSON-RPC of MCP's Streamable HTTP transport answered.
//! `initialize`, `ping` and `tools/list` are answered here; `tools/call`
//! goes to the main thread, and its answer comes back as server-sent
//! events, with progress and keep-alive comments while it runs (or as one
//! JSON answer to clients that take no events). The server keeps no
//! sessions, so a client carries on across restarts of Nerthus.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bevy::winit::{EventLoopProxy, WinitUserEvent};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::{json, Value};

use super::{tools, CallUpdate, ToolCall};

/// The port the server listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47110;
/// The MCP endpoint.
const ENDPOINT: &str = "/mcp";
/// The protocol revisions spoken, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
/// How long a call's event stream may stay quiet before a comment goes
/// out, so the client's timeouts do not fire while a build runs.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// How long a connection may idle between requests.
const IDLE_LIMIT: Duration = Duration::from_secs(600);
/// How often the listener and blocked reads look whether the server is
/// stopping.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long a write may block before the client counts as gone.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// What clients are told about the tools as they connect.
const INSTRUCTIONS: &str = "Nerthus is a node-based terrain generator. These tools work on the project open in the user's editor: \
every edit shows at once in its node graph and can be undone (the undo tool, or Ctrl Z in the editor). Nodes are identified by \
integer ids (list_nodes, add_node). A typical session: get_project for an overview; search_node_types, describe_node_type and \
suggest_nodes to choose nodes (list_examples and open_example give finished projects to start from); create_graph to make a \
whole graph at once, or add_node, connect_nodes and set_parameters a node at a time; validate_graph to check it; get_node_image, \
analyze_terrain, or capture_view for the 3D view, to look at results; mark_for_export and build_and_export (or run_project, for \
any saved project with build options) to write files, and analyze_build to read a build folder. Heights are fractions (0 to 1) of \
the terrain's height. Nodes build at the preview resolution (1024 by default); higher resolutions take longer. Calls run one at a \
time, and a call that builds waits for its build.";

/// A listening server. It stops when dropped: the listener at once,
/// each connection at its next request.
pub struct Listener {
    pub port: u16,
    shared: Arc<Shared>,
}

impl Listener {
    /// Listens on 127.0.0.1:`port`, handing tool calls to `calls` and
    /// waking the editor's event loop with `waker` for each.
    pub fn start(port: u16, calls: Sender<ToolCall>, waker: Option<EventLoopProxy<WinitUserEvent>>) -> io::Result<Self> {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shared = Shared {
            calls,
            waker,
            stopping: AtomicBool::new(false),
            in_flight: Mutex::new(Vec::new()),
        };
        let shared = Arc::new(shared);
        let accepting = shared.clone();
        let thread = std::thread::Builder::new().name("mcp-listener".to_string());
        thread.spawn(move || accept(&listener, &accepting))?;
        Ok(Self { port, shared })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Relaxed);
    }
}

/// What the listener and its connections share.
struct Shared {
    calls: Sender<ToolCall>,
    waker: Option<EventLoopProxy<WinitUserEvent>>,
    stopping: AtomicBool,
    /// The calls under way by JSON-RPC id, for `notifications/cancelled`.
    in_flight: Mutex<Vec<(Value, Arc<AtomicBool>)>>,
}

impl Shared {
    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }

    /// Hands a call to the main thread and wakes the editor for it; false
    /// once the editor has stopped taking calls.
    fn send(&self, call: ToolCall) -> bool {
        if self.calls.send(call).is_err() {
            return false;
        }
        if let Some(waker) = &self.waker {
            // A closed event loop means Nerthus is quitting anyway.
            let _ = waker.send_event(WinitUserEvent::WakeUp);
        }
        true
    }

    fn track(&self, id: &Value, cancelled: &Arc<AtomicBool>) {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        in_flight.push((id.clone(), cancelled.clone()));
    }

    fn untrack(&self, id: &Value) {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(position) = in_flight.iter().position(|(tracked, _)| tracked == id) {
            in_flight.remove(position);
        }
    }

    /// `notifications/cancelled`: the call with that id stops being waited
    /// for.
    fn cancel(&self, id: &Value) {
        let in_flight = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        for (tracked, cancelled) in in_flight.iter() {
            if tracked == id {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// Takes connections until the server stops.
fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stopping() {
        match listener.accept() {
            Ok((stream, _)) => {
                let connection_shared = shared.clone();
                let thread = std::thread::Builder::new().name("mcp-connection".to_string());
                let spawned = thread.spawn(move || serve_connection(stream, connection_shared));
                if let Err(error) = spawned {
                    log::warn!("An MCP connection could not be served: {error}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                log::warn!("The MCP server could not take a connection: {error}");
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// Blocking reads that wake to look whether the server is stopping, and
/// writes that give up on a stalled client.
fn configure(stream: &TcpStream) -> io::Result<()> {
    // Accepted sockets can inherit the listener's non-blocking mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(POLL_INTERVAL))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_nodelay(true)
}

/// Serves one connection's requests until it closes.
fn serve_connection(stream: TcpStream, shared: Arc<Shared>) {
    if configure(&stream).is_err() {
        return;
    }
    let mut connection = Connection { stream, buffer: Vec::new(), shared };
    loop {
        let request = match connection.read_request() {
            Ok(request) => request,
            Err(ReadError::Closed) => return,
            Err(ReadError::Rejected(status, reason)) => {
                let _ = connection.reply(status, reason, None, &[], &[]);
                return;
            }
        };
        let handled = connection.handle(&request);
        if handled.is_err() || !request.keeps_alive() {
            return;
        }
    }
}

/// Why no request was read: the client went (or idled too long, or the
/// server is stopping), or what it sent cannot be served.
enum ReadError {
    Closed,
    Rejected(u16, &'static str),
}

/// An HTTP request, its header names in lower case.
struct Request {
    method: String,
    path: String,
    version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        let found = self.headers.iter().find(|(header, _)| header == name);
        found.map(|(_, value)| value.as_str())
    }

    /// Whether the connection carries on after this request.
    fn keeps_alive(&self) -> bool {
        let connection = self.header("connection").map(str::to_ascii_lowercase);
        match connection.as_deref() {
            Some(value) if value.contains("close") => false,
            Some(value) if value.contains("keep-alive") => true,
            _ => self.version == "HTTP/1.1",
        }
    }
}

/// `METHOD /path?query HTTP/1.1` and the headers after it.
fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let version = parts.next()?.to_string();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let path = target.split('?').next().unwrap_or_default().to_string();
    let request = Request {
        method,
        path,
        version,
        headers,
        body: Vec::new(),
    };
    Some(request)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Whether a request comes from this machine: its host is the loopback
/// address, and so is the page that sent it, when a browser did
/// (rebinding a domain to 127.0.0.1 gets no further).
fn trusted(request: &Request) -> bool {
    let host_trusted = request.header("host").is_none_or(is_local_authority);
    let origin_trusted = request.header("origin").is_none_or(|origin| {
        let authority = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://"));
        authority.is_some_and(is_local_authority)
    });
    host_trusted && origin_trusted
}

/// `127.0.0.1:47110`, `localhost`, `[::1]:47110` and the like.
fn is_local_authority(authority: &str) -> bool {
    let host = if authority.starts_with('[') {
        let end = authority.find(']').map_or(authority.len(), |end| end + 1);
        &authority[..end]
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let host = host.to_ascii_lowercase();
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/// A connection, and what has been read off it but not used yet.
struct Connection {
    stream: TcpStream,
    buffer: Vec<u8>,
    shared: Arc<Shared>,
}

impl Connection {
    /// More bytes from the client.
    fn fill(&mut self, idle_since: Instant) -> Result<(), ReadError> {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            if self.shared.stopping() {
                return Err(ReadError::Closed);
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(ReadError::Closed),
                Ok(count) => {
                    self.buffer.extend_from_slice(&chunk[..count]);
                    return Ok(());
                }
                Err(error) => match error.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                        if idle_since.elapsed() > IDLE_LIMIT {
                            return Err(ReadError::Closed);
                        }
                    }
                    io::ErrorKind::Interrupted => {}
                    _ => return Err(ReadError::Closed),
                },
            }
        }
    }

    /// The next `count` bytes.
    fn take(&mut self, count: usize) -> Result<Vec<u8>, ReadError> {
        let idle_since = Instant::now();
        while self.buffer.len() < count {
            self.fill(idle_since)?;
        }
        let taken = self.buffer.drain(..count).collect();
        Ok(taken)
    }

    /// The next line, without its CRLF.
    fn take_line(&mut self) -> Result<String, ReadError> {
        let idle_since = Instant::now();
        loop {
            if let Some(end) = find(&self.buffer, b"\r\n") {
                let line: Vec<u8> = self.buffer.drain(..end + 2).collect();
                let text = String::from_utf8_lossy(&line[..end]).into_owned();
                return Ok(text);
            }
            if self.buffer.len() > MAX_HEAD_BYTES {
                return Err(ReadError::Rejected(400, "Bad Request"));
            }
            self.fill(idle_since)?;
        }
    }

    fn read_request(&mut self) -> Result<Request, ReadError> {
        let idle_since = Instant::now();
        let head_end = loop {
            if let Some(end) = find(&self.buffer, b"\r\n\r\n") {
                break end;
            }
            if self.buffer.len() > MAX_HEAD_BYTES {
                return Err(ReadError::Rejected(431, "Request Header Fields Too Large"));
            }
            self.fill(idle_since)?;
        };
        let head_bytes: Vec<u8> = self.buffer.drain(..head_end + 4).collect();
        let head = String::from_utf8_lossy(&head_bytes[..head_end]).into_owned();
        let mut request = parse_head(&head).ok_or(ReadError::Rejected(400, "Bad Request"))?;
        let expects_continue = request.header("expect").is_some_and(|expect| expect.eq_ignore_ascii_case("100-continue"));
        if expects_continue {
            let _ = self.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        request.body = self.read_body(&request)?;
        Ok(request)
    }

    fn read_body(&mut self, request: &Request) -> Result<Vec<u8>, ReadError> {
        let chunked = request.header("transfer-encoding").is_some_and(|encoding| encoding.to_ascii_lowercase().contains("chunked"));
        if chunked {
            return self.read_chunked_body();
        }
        let length = match request.header("content-length") {
            Some(text) => text.trim().parse::<usize>().map_err(|_| ReadError::Rejected(400, "Bad Request"))?,
            None => 0,
        };
        if length > MAX_BODY_BYTES {
            return Err(ReadError::Rejected(413, "Payload Too Large"));
        }
        self.take(length)
    }

    fn read_chunked_body(&mut self) -> Result<Vec<u8>, ReadError> {
        let mut body = Vec::new();
        loop {
            let line = self.take_line()?;
            let size_text = line.split(';').next().unwrap_or_default().trim();
            let size = usize::from_str_radix(size_text, 16).map_err(|_| ReadError::Rejected(400, "Bad Request"))?;
            if size == 0 {
                // Trailers, up to an empty line.
                while !self.take_line()?.is_empty() {}
                return Ok(body);
            }
            if body.len() + size > MAX_BODY_BYTES {
                return Err(ReadError::Rejected(413, "Payload Too Large"));
            }
            let chunk = self.take(size + 2)?;
            body.extend_from_slice(&chunk[..size]);
        }
    }

    fn handle(&mut self, request: &Request) -> io::Result<()> {
        if !trusted(request) {
            return self.reply(403, "Forbidden", None, &[], &[]);
        }
        if request.path != ENDPOINT {
            return self.reply(404, "Not Found", None, &[], &[]);
        }
        // No stream for the server's own messages (it sends none) and no
        // sessions to delete.
        if request.method != "POST" {
            return self.reply(405, "Method Not Allowed", None, &[], &[("Allow", "POST")]);
        }
        self.post(request)
    }

    fn post(&mut self, request: &Request) -> io::Result<()> {
        if let Some(version) = request.header("mcp-protocol-version")
            && !PROTOCOL_VERSIONS.contains(&version)
        {
            let text = format!("Unsupported protocol version {version}; Nerthus speaks {}.", PROTOCOL_VERSIONS.join(", "));
            let message = error_message(Value::Null, INVALID_REQUEST, &text);
            return self.reply_json(400, "Bad Request", &message);
        }
        let message: Value = match serde_json::from_slice(&request.body) {
            Ok(message) => message,
            Err(error) => {
                let reply = error_message(Value::Null, PARSE_ERROR, &error.to_string());
                return self.reply_json(400, "Bad Request", &reply);
            }
        };
        // Batches (2025-03-26) are answered in one JSON array.
        if let Value::Array(batch) = &message {
            let replies: Vec<Value> = batch.iter().filter_map(|message| answer(&self.shared, message)).collect();
            if replies.is_empty() {
                return self.reply(202, "Accepted", None, &[], &[]);
            }
            let replies = Value::Array(replies);
            return self.reply_json(200, "OK", &replies);
        }
        let accepts_events = request.header("accept").is_some_and(|accept| accept.contains("text/event-stream"));
        let is_tool_call = message.get("method").and_then(Value::as_str) == Some("tools/call") && message.get("id").is_some();
        if accepts_events && is_tool_call {
            return self.stream_tool_call(&message);
        }
        match answer(&self.shared, &message) {
            Some(reply) => self.reply_json(200, "OK", &reply),
            None => self.reply(202, "Accepted", None, &[], &[]),
        }
    }

    /// A `tools/call` answered as server-sent events: progress while it
    /// runs, comments while nothing happens, then the answer.
    fn stream_tool_call(&mut self, message: &Value) -> io::Result<()> {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let pending = match start_call(&self.shared, &id, &params) {
            Ok(pending) => pending,
            Err((code, text)) => {
                let reply = error_message(id, code, &text);
                return self.reply_json(200, "OK", &reply);
            }
        };
        let meta = params.get("_meta");
        let token = meta.and_then(|meta| meta.get("progressToken"));
        let streamed = self.stream_updates(&id, token, &pending);
        // A client that went away has given up on the call.
        if streamed.is_err() {
            pending.cancelled.store(true, Ordering::Relaxed);
        }
        self.shared.untrack(&id);
        streamed
    }

    fn stream_updates(&mut self, id: &Value, token: Option<&Value>, pending: &PendingCall) -> io::Result<()> {
        let mut events = EventStream::open(&mut self.stream)?;
        loop {
            match pending.updates.recv_timeout(KEEP_ALIVE) {
                Ok(CallUpdate::Progress { progress, total, message }) => {
                    let Some(token) = token else {
                        continue;
                    };
                    let mut params = json!({"progressToken": token, "progress": progress});
                    if let Some(total) = total {
                        params["total"] = json!(total);
                    }
                    if let Some(message) = message {
                        params["message"] = json!(message);
                    }
                    let notification = json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": params});
                    events.send(&notification)?;
                }
                Ok(CallUpdate::Finished(result)) => {
                    let reply = json!({"jsonrpc": "2.0", "id": id, "result": result.to_json()});
                    events.send(&reply)?;
                    return events.finish();
                }
                Err(RecvTimeoutError::Timeout) => events.keep_alive()?,
                // A cancelled call is not answered.
                Err(RecvTimeoutError::Disconnected) if pending.cancelled.load(Ordering::Relaxed) => return events.finish(),
                Err(RecvTimeoutError::Disconnected) => {
                    let reply = error_message(id.clone(), INTERNAL_ERROR, "The editor stopped serving MCP calls before this one finished.");
                    events.send(&reply)?;
                    return events.finish();
                }
            }
        }
    }

    fn reply(&mut self, status: u16, reason: &str, content_type: Option<&str>, body: &[u8], headers: &[(&str, &str)]) -> io::Result<()> {
        let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n", body.len());
        if let Some(content_type) = content_type {
            head.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        self.stream.write_all(head.as_bytes())?;
        self.stream.write_all(body)?;
        self.stream.flush()
    }

    fn reply_json(&mut self, status: u16, reason: &str, message: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(message)?;
        self.reply(status, reason, Some("application/json"), &body, &[])
    }
}

/// A response sent as server-sent events, each a chunk as it comes.
struct EventStream<'a> {
    stream: &'a mut TcpStream,
}

impl<'a> EventStream<'a> {
    fn open(stream: &'a mut TcpStream) -> io::Result<Self> {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\r\n";
        stream.write_all(head.as_bytes())?;
        stream.flush()?;
        Ok(Self { stream })
    }

    fn send(&mut self, message: &Value) -> io::Result<()> {
        let data = serde_json::to_string(message)?;
        let event = format!("event: message\ndata: {data}\n\n");
        self.chunk(&event)
    }

    fn keep_alive(&mut self) -> io::Result<()> {
        self.chunk(": keep-alive\n\n")
    }

    fn finish(self) -> io::Result<()> {
        self.stream.write_all(b"0\r\n\r\n")?;
        self.stream.flush()
    }

    fn chunk(&mut self, text: &str) -> io::Result<()> {
        let framed = format!("{:X}\r\n{text}\r\n", text.len());
        self.stream.write_all(framed.as_bytes())?;
        self.stream.flush()
    }
}

fn error_message(id: Value, code: i64, text: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
}

/// The answer to one message; none for notifications and for a client's
/// responses (the server sends no requests to answer).
fn answer(shared: &Shared, message: &Value) -> Option<Value> {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        let is_response = message.get("result").is_some() || message.get("error").is_some();
        if is_response {
            return None;
        }
        return Some(error_message(Value::Null, INVALID_REQUEST, "Not a JSON-RPC request, notification or response."));
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = message.get("id").cloned() else {
        if method == "notifications/cancelled"
            && let Some(request) = params.get("requestId")
        {
            shared.cancel(request);
        }
        return None;
    };
    let outcome = match method {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools::definitions()})),
        "tools/call" => call_and_wait(shared, &id, &params),
        _ => Err((METHOD_NOT_FOUND, format!("Method not found: {method}"))),
    };
    let reply = match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, text)) => error_message(id, code, &text),
    };
    Some(reply)
}

/// `initialize`: the revision the client asked for when it is spoken here,
/// else the newest one.
fn initialize_result(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let spoken = requested.filter(|version| PROTOCOL_VERSIONS.contains(version));
    let version = spoken.unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "nerthus", "title": "Nerthus", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

/// A call handed to the main thread: what it reports, and the flag that
/// cancels it.
struct PendingCall {
    updates: Receiver<CallUpdate>,
    cancelled: Arc<AtomicBool>,
}

fn start_call(shared: &Shared, id: &Value, params: &Value) -> Result<PendingCall, (i64, String)> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err((INVALID_PARAMS, "tools/call needs the tool's name.".to_string()));
    };
    if !tools::exists(name) {
        return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
    }
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let (sender, updates) = crossbeam_channel::unbounded();
    let cancelled = Arc::new(AtomicBool::new(false));
    let call = ToolCall {
        name: name.to_string(),
        arguments,
        updates: sender,
        cancelled: cancelled.clone(),
    };
    shared.track(id, &cancelled);
    if !shared.send(call) {
        shared.untrack(id);
        return Err((INTERNAL_ERROR, "Nerthus is closing.".to_string()));
    }
    Ok(PendingCall { updates, cancelled })
}

/// A `tools/call` answered in one piece, once it is done.
fn call_and_wait(shared: &Shared, id: &Value, params: &Value) -> Result<Value, (i64, String)> {
    let pending = start_call(shared, id, params)?;
    let outcome = loop {
        match pending.updates.recv() {
            Ok(CallUpdate::Progress { .. }) => {}
            Ok(CallUpdate::Finished(result)) => break Ok(result.to_json()),
            // The request still needs an HTTP answer.
            Err(_) if pending.cancelled.load(Ordering::Relaxed) => break Err((INTERNAL_ERROR, "The call was cancelled.".to_string())),
            Err(_) => break Err((INTERNAL_ERROR, "The editor stopped serving MCP calls before this one finished.".to_string())),
        }
    };
    shared.untrack(id);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_are_parsed() {
        let head = "POST /mcp?x=1 HTTP/1.1\r\nHost: 127.0.0.1:47110\r\nContent-Type: application/json\r\nConnection: close";
        let request = parse_head(head).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/mcp");
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert!(!request.keeps_alive());
    }

    #[test]
    fn only_loopback_hosts_and_pages_are_trusted() {
        assert!(is_local_authority("127.0.0.1:47110"));
        assert!(is_local_authority("localhost"));
        assert!(is_local_authority("[::1]:47110"));
        assert!(!is_local_authority("example.com:47110"));
        assert!(!is_local_authority("127.0.0.1.example.com"));

        let mut request = parse_head("POST /mcp HTTP/1.1\r\nHost: localhost:47110").unwrap();
        assert!(trusted(&request));
        request.headers.push(("origin".to_string(), "http://evil.example".to_string()));
        assert!(!trusted(&request));
        request.headers.pop();
        request.headers.push(("origin".to_string(), "http://localhost:6274".to_string()));
        assert!(trusted(&request));
    }

    /// A request to a listener and the whole raw answer.
    fn exchange(port: u16, head: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let request = format!("{head}\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn tool_calls_are_answered_as_events_or_json() {
        let (sender, calls) = crossbeam_channel::unbounded::<ToolCall>();
        let listener = Listener::start(0, sender, None).unwrap();
        // The main thread's part: each call reports progress, then answers
        // with its tool's name.
        std::thread::spawn(move || {
            for call in calls {
                let progress = CallUpdate::Progress { progress: 1.0, total: Some(2.0), message: None };
                let _ = call.updates.send(progress);
                let answer = crate::mcp::ToolResult::text(call.name.clone());
                let _ = call.updates.send(CallUpdate::Finished(answer));
            }
        });
        let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"get_project","arguments":{},"_meta":{"progressToken":"t"}}}"#;
        let streamed = exchange(listener.port, "POST /mcp HTTP/1.1\r\nAccept: application/json, text/event-stream", body);
        assert!(streamed.starts_with("HTTP/1.1 200 OK"), "{streamed}");
        assert!(streamed.contains("Content-Type: text/event-stream"));
        assert!(streamed.contains("\"method\":\"notifications/progress\""));
        assert!(streamed.contains("\"id\":7"));
        assert!(streamed.contains("\"text\":\"get_project\""));
        assert!(streamed.ends_with("0\r\n\r\n"));

        let answered = exchange(listener.port, "POST /mcp HTTP/1.1\r\nAccept: application/json", body);
        assert!(answered.contains("Content-Type: application/json"), "{answered}");
        assert!(answered.contains("\"text\":\"get_project\""));
        assert!(!answered.contains("notifications/progress"));

        let unknown = r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"nonsense"}}"#;
        let refused = exchange(listener.port, "POST /mcp HTTP/1.1\r\nAccept: application/json", unknown);
        assert!(refused.contains("-32602"), "{refused}");
    }

    #[test]
    fn other_methods_paths_and_pages_are_refused() {
        let (sender, _calls) = crossbeam_channel::unbounded::<ToolCall>();
        let listener = Listener::start(0, sender, None).unwrap();
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        assert!(exchange(listener.port, "GET /mcp HTTP/1.1", "").starts_with("HTTP/1.1 405"));
        assert!(exchange(listener.port, "POST /other HTTP/1.1", ping).starts_with("HTTP/1.1 404"));
        assert!(exchange(listener.port, "POST /mcp HTTP/1.1\r\nOrigin: http://evil.example", ping).starts_with("HTTP/1.1 403"));
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(exchange(listener.port, "POST /mcp HTTP/1.1", notification).starts_with("HTTP/1.1 202"));
        assert!(exchange(listener.port, "POST /mcp HTTP/1.1", "{not json").starts_with("HTTP/1.1 400"));
        let answered = exchange(listener.port, "POST /mcp HTTP/1.1", ping);
        assert!(answered.contains("\"result\":{}"), "{answered}");
    }

    #[test]
    fn initialize_agrees_on_a_revision() {
        let asked = json!({"protocolVersion": "2025-06-18"});
        assert_eq!(initialize_result(&asked)["protocolVersion"], "2025-06-18");
        let unknown = json!({"protocolVersion": "2099-01-01"});
        assert_eq!(initialize_result(&unknown)["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }
}
