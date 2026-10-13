//! The MCP server (Model Context Protocol): AI assistants such as Claude
//! Code drive the editor through it, making the edits the interface makes,
//! so they show at once and can be undone. It listens on 127.0.0.1 only
//! (Streamable HTTP at `http://127.0.0.1:<port>/mcp`) and is off unless
//! `mcp.dat` is in the data folder (its text, when it has any, is the
//! port), `--mcp` was given, or Tools › MCP Server switched it on.
//!
//! Each connection has a thread of its own (`server`); tool calls come to
//! the main thread and run one at a time in the order they came, so a call
//! that waits for a build holds the next ones back until it is done.

mod arguments;
mod authoring_tools;
mod build_tools;
mod catalogue_tools;
mod edit_tools;
mod file_tools;
mod graph_tools;
mod images;
mod jobs;
mod knowledge;
mod project_tools;
mod review_tools;
mod run_tools;
mod server;
mod terrain_report;
mod tools;
mod values;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::prelude::{App, Plugin, ResMut, Resource, Update};
use bevy::winit::EventLoopProxyWrapper;
use bevy::ecs::system::Res;
use crossbeam_channel::{Receiver, Sender};
use nerthus_render::Screenshots;
use serde_json::{json, Value};

use crate::editor::Editor;
use crate::ui::UiState;
use build_tools::Wait;

pub use server::DEFAULT_PORT;

/// The data folder's marker: when it is there the server starts with
/// Nerthus, on the port its text gives (the default when it gives none).
pub const MCP_FILE: &str = "mcp.dat";

/// How often a waiting call reports its progress at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// What the interface shows of the server, and asks of it.
#[derive(Default)]
pub struct McpStatus {
    /// The address clients connect to, while the server listens.
    pub address: Option<String>,
    /// Tools › MCP Server was clicked: the server goes on or off.
    pub toggle_requested: bool,
}

/// A `tools/call` for the main thread.
pub(crate) struct ToolCall {
    pub name: String,
    pub arguments: Value,
    /// Progress, then the answer.
    pub updates: Sender<CallUpdate>,
    /// Set when the client cancels the call or goes away.
    pub cancelled: Arc<AtomicBool>,
}

/// What a call tells its connection.
pub(crate) enum CallUpdate {
    Progress { progress: f64, total: Option<f64>, message: Option<String> },
    Finished(ToolResult),
}

/// A tool's answer: content blocks (text and images), and whether it
/// reports a failure for the model to read.
pub(crate) struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        let block = json!({"type": "text", "text": text.into()});
        Self { content: vec![block], is_error: false }
    }

    /// A JSON value, pretty-printed as text.
    pub fn json(value: &Value) -> Self {
        let text = serde_json::to_string_pretty(value).unwrap_or_default();
        Self::text(text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        let mut result = Self::text(text);
        result.is_error = true;
        result
    }

    /// The `tools/call` result.
    pub fn to_json(&self) -> Value {
        let mut result = json!({"content": self.content});
        if self.is_error {
            result["isError"] = json!(true);
        }
        result
    }
}

/// What starting (or polling) a call came to: its answer, or something to
/// wait for over the next frames.
pub(crate) enum Step {
    Done(ToolResult),
    Wait(Wait),
}

/// What tools work with on the main thread.
pub(crate) struct ToolContext<'a> {
    pub editor: &'a mut Editor,
    pub ui_state: &'a mut UiState,
    pub screenshots: &'a mut Screenshots,
    /// The id the next in-memory capture goes under.
    pub next_capture: &'a mut u64,
    /// The builds the server ran.
    pub history: &'a mut jobs::BuildHistory,
}

/// A call waiting on the editor.
struct ActiveCall {
    call: ToolCall,
    wait: Wait,
    started: Instant,
    /// The progress reported last, and when.
    reported: Option<(f64, Instant)>,
}

impl ActiveCall {
    /// One frame's look at what the call waits for; the call back while
    /// it still waits.
    fn advance(mut self, context: &mut ToolContext) -> Option<Self> {
        if self.call.cancelled.load(Ordering::Relaxed) {
            self.wait.abandon(context);
            return None;
        }
        if self.started.elapsed() > self.wait.time_limit() {
            self.wait.time_out();
            let result = ToolResult::error(self.wait.timeout_message());
            let _ = self.call.updates.send(CallUpdate::Finished(result));
            return None;
        }
        match self.wait.poll(context) {
            None => {
                self.report_progress(context);
                Some(self)
            }
            // Each stage has its own time limit.
            Some(Step::Wait(next)) => {
                self.wait = next;
                self.started = Instant::now();
                self.reported = None;
                Some(self)
            }
            Some(Step::Done(result)) => {
                let _ = self.call.updates.send(CallUpdate::Finished(result));
                None
            }
        }
    }

    fn report_progress(&mut self, context: &ToolContext) {
        let Some((progress, total, message)) = self.wait.progress(context) else {
            return;
        };
        let unchanged = self.reported.is_some_and(|(reported, _)| reported == progress);
        let too_soon = self.reported.is_some_and(|(_, at)| at.elapsed() < PROGRESS_INTERVAL);
        if unchanged || too_soon {
            return;
        }
        self.reported = Some((progress, Instant::now()));
        let update = CallUpdate::Progress { progress, total, message };
        let _ = self.call.updates.send(update);
    }
}

/// The server: whether it should listen and on which port, the listener,
/// and the calls waiting to run.
#[derive(Resource)]
pub struct McpServer {
    port: u16,
    wanted: bool,
    /// Starting failed (the port is taken): not tried again until asked.
    failed: bool,
    listener: Option<server::Listener>,
    calls: Receiver<ToolCall>,
    call_sender: Sender<ToolCall>,
    queue: VecDeque<ToolCall>,
    active: Option<ActiveCall>,
    next_capture: u64,
    history: jobs::BuildHistory,
}

impl McpServer {
    /// Tools › MCP Server, and starting or stopping as wanted.
    fn switch(&mut self, editor: &mut Editor, waker: Option<&EventLoopProxyWrapper>) {
        if std::mem::take(&mut editor.mcp.toggle_requested) {
            self.wanted = self.listener.is_none();
            self.failed = false;
            if let Err(error) = save_marker(self.wanted, self.port) {
                editor.report(format!("Cannot keep the MCP server setting: {error}"));
            }
        }
        if self.wanted && self.listener.is_none() && !self.failed {
            self.start(editor, waker);
        }
        if !self.wanted && self.listener.is_some() {
            self.stop(editor);
        }
    }

    fn start(&mut self, editor: &mut Editor, waker: Option<&EventLoopProxyWrapper>) {
        let proxy = waker.map(|wrapper| (**wrapper).clone());
        match server::Listener::start(self.port, self.call_sender.clone(), proxy) {
            Ok(listener) => {
                let address = format!("http://127.0.0.1:{}/mcp", listener.port);
                editor.report(format!("MCP server listening on {address}"));
                editor.mcp.address = Some(address);
                self.listener = Some(listener);
            }
            Err(error) => {
                self.failed = true;
                editor.report(format!("The MCP server cannot listen on port {}: {error}", self.port));
            }
        }
    }

    /// The listener goes; calls not answered yet are dropped, and their
    /// connections tell their clients the editor stopped.
    fn stop(&mut self, editor: &mut Editor) {
        self.listener = None;
        self.queue.clear();
        self.active = None;
        editor.mcp.address = None;
        editor.report("MCP server stopped");
    }
}

/// Writes or deletes `mcp.dat`, for the next start.
fn save_marker(on: bool, port: u16) -> std::io::Result<()> {
    let Some(folder) = crate::user::data_folder() else {
        return Ok(());
    };
    let path = folder.join(MCP_FILE);
    if on {
        std::fs::create_dir_all(&folder)?;
        std::fs::write(path, port.to_string())
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

/// `mcp.dat`: none when it is missing, else the port its text gives, if
/// any.
fn saved_marker() -> Option<Option<u16>> {
    let folder = crate::user::data_folder()?;
    let text = std::fs::read_to_string(folder.join(MCP_FILE)).ok()?;
    Some(text.trim().parse().ok())
}

pub struct McpPlugin {
    /// `--mcp`: listen whatever `mcp.dat` says.
    pub enabled: bool,
    /// `--mcp-port`.
    pub port: Option<u16>,
    /// A `--screenshot` run listens only when told to with `--mcp`.
    pub screenshot_run: bool,
}

impl Plugin for McpPlugin {
    fn build(&self, app: &mut App) {
        let marker = if self.screenshot_run { None } else { saved_marker() };
        let wanted = self.enabled || marker.is_some();
        let saved_port = marker.flatten();
        let port = self.port.or(saved_port).unwrap_or(DEFAULT_PORT);
        let (call_sender, calls) = crossbeam_channel::unbounded();
        let server = McpServer {
            port,
            wanted,
            failed: false,
            listener: None,
            calls,
            call_sender,
            queue: VecDeque::new(),
            active: None,
            next_capture: 0,
            history: jobs::BuildHistory::default(),
        };
        app.insert_resource(server);
        app.add_systems(Update, serve);
    }
}

/// Starts or stops the server as asked, takes the calls that came, and
/// runs them in turn.
fn serve(mut server: ResMut<McpServer>, mut editor: ResMut<Editor>, mut ui_state: ResMut<UiState>, mut screenshots: ResMut<Screenshots>, waker: Option<Res<EventLoopProxyWrapper>>) {
    let server = server.as_mut();
    server.switch(&mut editor, waker.as_deref());
    let arrived: Vec<ToolCall> = server.calls.try_iter().collect();
    server.queue.extend(arrived);
    // Captures no call waits for any more (it was cancelled) go.
    let capturing = server.active.as_ref().is_some_and(|active| active.wait.is_capture());
    if !capturing && !screenshots.captured.is_empty() {
        screenshots.captured.clear();
    }
    if server.queue.is_empty() && server.active.is_none() {
        return;
    }
    let mut context = ToolContext {
        editor: &mut editor,
        ui_state: &mut ui_state,
        screenshots: &mut screenshots,
        next_capture: &mut server.next_capture,
        history: &mut server.history,
    };
    if let Some(active) = server.active.take() {
        server.active = active.advance(&mut context);
        if server.active.is_some() {
            return;
        }
    }
    while let Some(call) = server.queue.pop_front() {
        if call.cancelled.load(Ordering::Relaxed) {
            continue;
        }
        log::debug!("MCP call: {}", call.name);
        match tools::start(&call.name, &call.arguments, &mut context) {
            Step::Done(result) => {
                let _ = call.updates.send(CallUpdate::Finished(result));
            }
            Step::Wait(wait) => {
                let active = ActiveCall {
                    call,
                    wait,
                    started: Instant::now(),
                    reported: None,
                };
                server.active = Some(active);
                return;
            }
        }
    }
}
