//! Building and looking: a node built at the preview resolution (by
//! previewing it, as selecting it in the editor does), pictures of its
//! outputs and of the editor, one of its maps written to a file, export
//! marks, and Build and Export.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError};
use image::RgbImage;
use nerthus_engine::json::f32_value;
use nerthus_engine::Map;
use nerthus_export::{FileFormat, WriteOptions};
use nerthus_render::{CapturedPixels, ScreenshotRequest};
use serde_json::{json, Value};

use super::arguments::Arguments;
use super::graph_tools::output_port;
use super::images::{self, ImageFormat};
use super::jobs::{HistoryEntry, Job};
use super::terrain_report::Heightfield;
use super::{Step, ToolContext, ToolResult};
use crate::editor::{Editor, PREVIEW_RESOLUTIONS};
use crate::export_build::{ExportBuild, LogLine};
use crate::viewport_render::RenderJob;

/// The file formats, by the names tools use.
pub(crate) const FORMATS: [(&str, FileFormat); 15] = [
    ("graw", FileFormat::PureRaw),
    ("half_raw", FileFormat::HalfRaw),
    ("half_r16", FileFormat::HalfRaw16),
    ("ushort_raw", FileFormat::UshortRaw),
    ("ushort_r16", FileFormat::UshortRaw16),
    ("float_raw", FileFormat::FloatRaw),
    ("float_r32", FileFormat::FloatRaw32),
    ("jpeg", FileFormat::Jpeg),
    ("png8", FileFormat::Png8),
    ("png16", FileFormat::Png16),
    ("png64", FileFormat::Png64),
    ("tiff8", FileFormat::Tiff8),
    ("tiff16", FileFormat::Tiff16),
    ("tiff32", FileFormat::Tiff32),
    ("exr", FileFormat::Exr),
];

/// How long each wait may last.
const BUILD_LIMIT: Duration = Duration::from_secs(30 * 60);
const EXPORT_LIMIT: Duration = Duration::from_secs(4 * 60 * 60);
const CAPTURE_LIMIT: Duration = Duration::from_secs(60);
const THREAD_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long a capture waits for the preview to settle before it is taken
/// as things stand.
const SETTLE_LIMIT: Duration = Duration::from_secs(120);

const ENGINE_PAUSED: &str = "The editor's engine is suspended (Preview › Engine), so nothing builds; ask the user to resume it.";

pub(crate) fn format_named(name: &str) -> Result<FileFormat, String> {
    let wanted = name.trim().to_ascii_lowercase();
    let found = FORMATS.iter().find(|(key, _)| *key == wanted);
    found.map(|(_, format)| *format).ok_or_else(|| {
        let keys: Vec<&str> = FORMATS.iter().map(|(key, _)| *key).collect();
        format!("There is no file format \"{name}\"; the formats are {}.", keys.join(", "))
    })
}

pub(crate) fn format_key(format: FileFormat) -> &'static str {
    let found = FORMATS.iter().find(|(_, candidate)| *candidate == format);
    found.map_or("graw", |(key, _)| key)
}

/// How a picture of a map is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Style {
    /// As the 2D view shows it.
    Rendered,
    /// The values themselves.
    Data,
}

/// What to make of a node once it is built.
pub(crate) enum AfterBuild {
    Report,
    Image { port: Option<String>, style: Style, size: u32, format: ImageFormat },
    Write { port: Option<String>, path: PathBuf, format: FileFormat },
    /// The output read as land (analyze_terrain).
    Analyze { port: Option<String>, settings: TerrainSettings },
}

/// How analyze_terrain reads a heightfield: the terrain's width and the
/// height a 1 stands for (metres), the steepest gentle ground (degrees)
/// and the share taken as the middle.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainSettings {
    pub metres_across: f64,
    pub metres_up: f64,
    pub gentle_degrees: f64,
    pub centre_fraction: f64,
}

/// A picture of the editor to take.
#[derive(Clone, Copy)]
pub(crate) struct Capture {
    window: bool,
    size: u32,
    format: ImageFormat,
}

/// What a call waits for.
pub(crate) enum Wait {
    /// The build of `node` asked for after `since` (a build generation),
    /// then what to do with it.
    Build { node: i32, since: u64, then: AfterBuild },
    /// The preview building and rendering, before a capture.
    Settle { capture: Capture, since: Instant },
    /// A capture, by its id.
    Capture { id: u64, capture: Capture },
    /// Work on a helper thread.
    Thread(Receiver<ToolResult>),
    /// The Build and Export under way.
    Export,
    /// Work on a thread of its own that reports how it goes.
    Job(Job),
}

impl Wait {
    pub fn time_limit(&self) -> Duration {
        match self {
            Wait::Build { .. } => BUILD_LIMIT,
            Wait::Settle { .. } => SETTLE_LIMIT + CAPTURE_LIMIT,
            Wait::Capture { .. } => CAPTURE_LIMIT,
            Wait::Thread(_) => THREAD_LIMIT,
            Wait::Export => EXPORT_LIMIT,
            Wait::Job(job) => job.limit,
        }
    }

    pub fn timeout_message(&self) -> &'static str {
        match self {
            Wait::Build { .. } => "The build is taking too long to wait for; it carries on in the editor (get_project shows how it is going).",
            Wait::Settle { .. } | Wait::Capture { .. } => "The view could not be captured in time.",
            Wait::Thread(_) => "Making the answer took too long.",
            Wait::Export => "Build and Export is taking too long to wait for; it carries on (get_project shows how it is going).",
            Wait::Job(_) => "The work ran past its time limit and was stopped.",
        }
    }

    /// Its time ran out: a job stops; anything else carries on unheard.
    pub fn time_out(&self) {
        if let Wait::Job(job) = self {
            job.stop();
        }
    }

    pub fn is_capture(&self) -> bool {
        matches!(self, Wait::Capture { .. })
    }

    /// The client gave up: a Build and Export it started stops; anything
    /// else simply finishes unheard.
    pub fn abandon(&self, context: &mut ToolContext) {
        if let Wait::Job(job) = self {
            job.stop();
        }
        if let Wait::Export = self
            && let Some(build) = &context.editor.export_build
            && !build.finished
        {
            build.cancel();
        }
    }

    /// How far along it is: done, out of, and what it is on.
    pub fn progress(&self, context: &ToolContext) -> Option<(f64, Option<f64>, Option<String>)> {
        match self {
            Wait::Build { .. } if context.editor.building => {
                let percent = (context.editor.build_progress as f64 * 100.0).round();
                Some((percent, Some(100.0), Some("Building".to_string())))
            }
            Wait::Export => {
                let build = context.editor.export_build.as_ref()?;
                Some((build.built as f64, Some(build.total as f64), build.current.clone()))
            }
            Wait::Job(job) => job.progress(),
            _ => None,
        }
    }

    /// One frame's look: the next step once there is one.
    pub fn poll(&mut self, context: &mut ToolContext) -> Option<Step> {
        match self {
            Wait::Build { node, since, then } => {
                let node = *node;
                let since = *since;
                poll_build(node, since, then, context)
            }
            Wait::Settle { capture, since } => {
                let settled = preview_settled(context.editor);
                if !settled && since.elapsed() < SETTLE_LIMIT {
                    return None;
                }
                Some(request_capture(*capture, context))
            }
            Wait::Capture { id, capture } => poll_capture(*id, *capture, context),
            Wait::Thread(receiver) => match receiver.try_recv() {
                Ok(result) => Some(Step::Done(result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Step::Done(ToolResult::error("The work stopped without an answer."))),
            },
            Wait::Export => poll_export(context),
            Wait::Job(job) => job.poll(context.history),
        }
    }
}

/// Previews `node` and asks for its build anew, so what comes back was
/// built after the call.
pub(crate) fn start_build(editor: &mut Editor, node: i32, port: Option<&str>, then: AfterBuild) -> Result<Step, String> {
    if !editor.engine_active {
        return Err(ENGINE_PAUSED.to_string());
    }
    let since = editor.build_generation();
    editor.preview(Some(node));
    if let Some(port) = port {
        editor.preview_port = port.to_string();
        editor.show_preview();
    }
    editor.needs_build = true;
    Ok(Step::Wait(Wait::Build { node, since, then }))
}

fn poll_build(node: i32, since: u64, then: &mut AfterBuild, context: &mut ToolContext) -> Option<Step> {
    let editor = &*context.editor;
    if !editor.engine_active {
        return Some(Step::Done(ToolResult::error(ENGINE_PAUSED)));
    }
    let cancelled = editor.build_outcome_since(since)?;
    if cancelled {
        return Some(Step::Done(ToolResult::error("The build was cancelled in the editor.")));
    }
    if editor.preview_node != Some(node) {
        return Some(Step::Done(ToolResult::error("The editor went on to preview another node before this one was built; try again.")));
    }
    let then = std::mem::replace(then, AfterBuild::Report);
    context.history.record(build_record(editor, node));
    Some(finish_build(editor, node, then))
}

/// A node's build for the history.
fn build_record(editor: &Editor, node: i32) -> HistoryEntry {
    let mut entry = HistoryEntry::new("build_node");
    entry.project = Some(editor.document.file_name());
    entry.target = editor.document.node(node).map(|data| format!("{} ({node})", data.name));
    entry.resolution = editor.preview_resolution;
    entry.seconds = editor.last_build_seconds as f64;
    for id in nerthus_engine::Engine::build_order(editor.document.terrain(), node) {
        if let Some(error) = editor.status(id).error {
            let name = editor.document.node(id).map(|data| data.name.clone()).unwrap_or_default();
            entry.problems.push(format!("{name} ({id}): {error}"));
        }
    }
    if let Some(error) = &editor.preview_error {
        entry.problems.push(error.clone());
    }
    entry.success = entry.problems.is_empty() && !editor.outputs.is_empty();
    entry
}

fn finish_build(editor: &Editor, node: i32, then: AfterBuild) -> Step {
    let report = build_report(editor, node);
    match then {
        AfterBuild::Report => Step::Done(ToolResult::json(&report)),
        AfterBuild::Image { port, style, size, format } => {
            let (port, map) = match output_map(editor, port.as_deref()) {
                Ok(found) => found,
                Err(problem) => return Step::Done(ToolResult::error(problem)),
            };
            let job = if style == Style::Rendered { render_job(editor, &port) } else { None };
            on_thread(move || image_result(&port, &map, job, size, format, report))
        }
        AfterBuild::Write { port, path, format } => {
            let (_, map) = match output_map(editor, port.as_deref()) {
                Ok(found) => found,
                Err(problem) => return Step::Done(ToolResult::error(problem)),
            };
            let options = editor.user.preferences.export_preferences().write;
            on_thread(move || write_result(&map, &path, format, &options))
        }
        AfterBuild::Analyze { port, settings } => {
            let (port, map) = match output_map(editor, port.as_deref()) {
                Ok(found) => found,
                Err(problem) => return Step::Done(ToolResult::error(problem)),
            };
            on_thread(move || {
                let field = Heightfield::of_map(&map);
                let mut analysed = field.report(settings.metres_across, settings.metres_up, settings.gentle_degrees, settings.centre_fraction);
                analysed["node"] = json!(node);
                analysed["port"] = json!(port);
                if map.channel_count() > 1 {
                    analysed["note"] = json!("The output has several channels (a colour map); its first was read as heights.");
                }
                ToolResult::json(&analysed)
            })
        }
    }
}

/// Work for a helper thread, its answer waited for.
fn on_thread(work: impl FnOnce() -> ToolResult + Send + 'static) -> Step {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let thread = std::thread::Builder::new().name("mcp-work".to_string());
    let spawned = thread.spawn(move || {
        let _ = sender.send(work());
    });
    match spawned {
        Ok(_) => Step::Wait(Wait::Thread(receiver)),
        Err(error) => Step::Done(ToolResult::error(format!("The work could not start: {error}"))),
    }
}

/// The previewed node's map for `port` (its main output when none is
/// named), as its build left it.
fn output_map(editor: &Editor, port: Option<&str>) -> Result<(String, Arc<Map>), String> {
    if editor.outputs.is_empty() {
        let reason = editor.preview_error.clone().unwrap_or_else(|| "it has no outputs".to_string());
        return Err(format!("The node did not build: {reason}"));
    }
    let wanted = port.unwrap_or("Out");
    let named = editor.outputs.iter().find(|(name, _)| name.eq_ignore_ascii_case(wanted));
    let found = if port.is_none() { named.or(editor.outputs.first()) } else { named };
    found.cloned().ok_or_else(|| {
        let names: Vec<&str> = editor.outputs.iter().map(|(name, _)| name.as_str()).collect();
        format!("The node built no {wanted}; it built {}.", names.join(", "))
    })
}

/// The 2D view's render of the previewed node's `port`.
fn render_job(editor: &Editor, port: &str) -> Option<RenderJob> {
    let mut source = editor.preview_source();
    source.port = port;
    editor.render_job_for(&source, 0)
}

/// An output's size, kind and (for one channel) value range.
fn output_summary(editor: &Editor, port: &str, map: &Map) -> Value {
    let mut entry = json!({"port": port, "width": map.width(), "height": map.height(), "channels": map.channel_count()});
    if let Some(intent) = editor.port_intents.get(port) {
        entry["kind"] = json!(format!("{intent:?}"));
    }
    if map.channel_count() == 1 {
        let values = map.channel(0);
        let (low, high) = images::value_range(values);
        let finite: Vec<f64> = values.iter().filter(|value| value.is_finite()).map(|value| *value as f64).collect();
        let mean = if finite.is_empty() { 0.0 } else { finite.iter().sum::<f64>() / finite.len() as f64 };
        entry["min"] = f32_value(low);
        entry["max"] = f32_value(high);
        entry["mean"] = f32_value(mean as f32);
    }
    entry
}

/// How a node's build went: its outputs, and any errors on the way.
fn build_report(editor: &Editor, node: i32) -> Value {
    let name = editor.document.node(node).map(|data| data.name.clone());
    let outputs: Vec<Value> = editor.outputs.iter().map(|(port, map)| output_summary(editor, port, map)).collect();
    let order = nerthus_engine::Engine::build_order(editor.document.terrain(), node);
    let mut errors = Vec::new();
    for id in order {
        let Some(error) = editor.status(id).error else {
            continue;
        };
        let failed = editor.document.node(id).map(|data| data.name.clone());
        errors.push(json!({"node": id, "name": failed, "error": error}));
    }
    let mut report = json!({
        "node": node,
        "name": name,
        "resolution": editor.preview_resolution,
        "seconds": f32_value((editor.last_build_seconds * 100.0).round() / 100.0),
        "outputs": outputs,
    });
    if !errors.is_empty() {
        report["errors"] = json!(errors);
    }
    if let Some(error) = &editor.preview_error {
        report["error"] = json!(error);
    }
    report
}

fn image_result(port: &str, map: &Map, job: Option<RenderJob>, size: u32, format: ImageFormat, mut report: Value) -> ToolResult {
    let rendered = job.as_ref().and_then(crate::viewport_render::render);
    let rendered_picture = rendered.and_then(|colours| RgbImage::from_raw(colours.width as u32, colours.height as u32, colours.rgb));
    let style = if rendered_picture.is_some() { "rendered" } else { "data" };
    let (picture, range) = match rendered_picture {
        Some(picture) => (picture, None),
        None => images::map_picture(map),
    };
    let fitted = images::fit(picture, size);
    let block = match images::content(&fitted, format) {
        Ok(block) => block,
        Err(problem) => return ToolResult::error(problem),
    };
    let mut picture_entry = json!({"port": port, "style": style, "width": fitted.width(), "height": fitted.height()});
    if let Some((low, high)) = range {
        picture_entry["grey"] = json!(format!("black is {low}, white is {high}"));
    }
    report["picture"] = picture_entry;
    let mut result = ToolResult::json(&report);
    result.content.push(block);
    result
}

fn write_result(map: &Map, path: &std::path::Path, format: FileFormat, options: &WriteOptions) -> ToolResult {
    let prepared = nerthus_export::clamp_for_export(map);
    match nerthus_export::write_map(format, path, &prepared, options) {
        Ok(()) => {
            let written = json!({"written": path, "format": format_key(format), "width": map.width(), "height": map.height()});
            ToolResult::json(&written)
        }
        Err(error) => ToolResult::error(format!("{} could not be written: {error}", path.display())),
    }
}

/// Whether the preview is built and rendered, so the 3D view shows it.
fn preview_settled(editor: &Editor) -> bool {
    if editor.needs_build || editor.building {
        return false;
    }
    let previewing = editor.preview_node.is_some() && !editor.outputs.is_empty();
    !previewing || editor.render_settled()
}

fn request_capture(capture: Capture, context: &mut ToolContext) -> Step {
    let id = *context.next_capture;
    *context.next_capture += 1;
    let request = if capture.window { ScreenshotRequest::WindowPixels { id } } else { ScreenshotRequest::ViewportPixels { id } };
    context.screenshots.requests.push(request);
    Step::Wait(Wait::Capture { id, capture })
}

fn poll_capture(id: u64, capture: Capture, context: &mut ToolContext) -> Option<Step> {
    let captured = &mut context.screenshots.captured;
    let position = captured.iter().position(|(captured_id, _)| *captured_id == id)?;
    let (_, outcome) = captured.remove(position);
    let step = match outcome {
        Ok(pixels) => on_thread(move || capture_result(&pixels, capture)),
        Err(problem) => Step::Done(ToolResult::error(format!("The view could not be captured: {problem}"))),
    };
    Some(step)
}

fn capture_result(pixels: &CapturedPixels, capture: Capture) -> ToolResult {
    let picture = images::capture_picture(pixels);
    let fitted = images::fit(picture, capture.size);
    let block = match images::content(&fitted, capture.format) {
        Ok(block) => block,
        Err(problem) => return ToolResult::error(problem),
    };
    let view = if capture.window { "window" } else { "3d" };
    let summary = json!({"view": view, "width": fitted.width(), "height": fitted.height(), "captured": [pixels.width, pixels.height]});
    let mut result = ToolResult::json(&summary);
    result.content.push(block);
    result
}

fn poll_export(context: &mut ToolContext) -> Option<Step> {
    let editor = &*context.editor;
    let Some(build) = &editor.export_build else {
        return Some(Step::Done(ToolResult::error("The Build and Export went away before it finished.")));
    };
    if !build.finished {
        return None;
    }
    let mut entry = HistoryEntry::new("build_and_export");
    entry.project = Some(editor.document.file_name());
    entry.resolution = build.resolution;
    entry.seconds = build.seconds() as f64;
    entry.files = build.files;
    entry.folder = build.folder.clone();
    entry.success = build.succeeded();
    entry.problems.extend(build.refusal.clone());
    for line in &build.log {
        match line {
            LogLine::Failed { node, error } => entry.problems.push(format!("{node}: {error}")),
            LogLine::Problem(problem) => entry.problems.push(problem.clone()),
            _ => {}
        }
    }
    let result = export_result(build);
    context.history.record(entry);
    Some(Step::Done(result))
}

/// A Build and Export's outcome: where it went and what it wrote.
pub(crate) fn export_result(build: &ExportBuild) -> ToolResult {
    if let Some(refusal) = &build.refusal {
        return ToolResult::error(format!("Build and Export did not start: {refusal}"));
    }
    let mut files = Vec::new();
    let mut problems = Vec::new();
    for line in &build.log {
        match line {
            LogLine::Wrote(path) => files.push(path.display().to_string()),
            LogLine::Failed { node, error } => problems.push(format!("{node} failed: {error}")),
            LogLine::Problem(problem) => problems.push(problem.clone()),
            LogLine::Built { .. } | LogLine::Step(_) => {}
        }
    }
    let mut outcome = json!({
        "folder": build.folder,
        "resolution": build.resolution,
        "seconds": f32_value((build.seconds() * 10.0).round() / 10.0),
        "files": files,
    });
    if build.cancelled {
        outcome["cancelled"] = json!(true);
    }
    if !problems.is_empty() {
        outcome["problems"] = json!(problems);
    }
    ToolResult::json(&outcome)
}

/// What get_project shows of a Build and Export.
pub(crate) fn export_status(editor: &Editor) -> Value {
    let Some(build) = &editor.export_build else {
        return Value::Null;
    };
    json!({
        "finished": build.finished,
        "cancelled": build.cancelled,
        "refusal": build.refusal,
        "built": build.built,
        "total": build.total,
        "current": build.current,
        "folder": build.folder,
        "files_written": build.files,
        "failures": build.failures,
        "seconds": f32_value((build.seconds() * 10.0).round() / 10.0),
    })
}

fn image_size(arguments: &Arguments, default: u32) -> Result<u32, String> {
    let size = arguments.integer("size")?.unwrap_or(default as i64);
    Ok(size.clamp(64, 2048) as u32)
}

fn image_format(arguments: &Arguments, default: ImageFormat) -> Result<ImageFormat, String> {
    match arguments.string("format")? {
        Some(name) => ImageFormat::from_name(name).ok_or_else(|| "format is png or jpeg.".to_string()),
        None => Ok(default),
    }
}

/// The node's output by name, checked.
fn named_output(arguments: &Arguments, editor: &Editor, node: i32) -> Result<Option<String>, String> {
    let Some(port) = arguments.string("port")? else {
        return Ok(None);
    };
    let data = editor.document.node(node).ok_or("The node is gone.")?;
    let found = output_port(data, Some(port))?;
    Ok(Some(found))
}

pub(crate) fn build_node(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let node = arguments.node("node", editor)?;
    if let Some(resolution) = arguments.integer("resolution")? {
        let resolution = resolution as usize;
        if !PREVIEW_RESOLUTIONS.contains(&resolution) {
            return Err("resolution is 512, 1024, 2048 or 4096.".to_string());
        }
        editor.set_preview_resolution(resolution);
    }
    start_build(editor, node, None, AfterBuild::Report)
}

pub(crate) fn get_node_image(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let node = arguments.node("node", editor)?;
    let port = named_output(arguments, editor, node)?;
    let style = match arguments.string("style")? {
        None | Some("rendered") => Style::Rendered,
        Some("data") => Style::Data,
        Some(other) => return Err(format!("style is rendered or data, not {other}.")),
    };
    let size = image_size(arguments, 512)?;
    let format = image_format(arguments, ImageFormat::Jpeg)?;
    let then = AfterBuild::Image { port: port.clone(), style, size, format };
    start_build(editor, node, port.as_deref(), then)
}

pub(crate) fn capture_view(arguments: &Arguments, _context: &mut ToolContext) -> Result<Step, String> {
    let window = match arguments.string("view")? {
        None | Some("3d") => false,
        Some("window") => true,
        Some(other) => return Err(format!("view is 3d or window, not {other}.")),
    };
    let size = image_size(arguments, 1024)?;
    let default_format = if window { ImageFormat::Png } else { ImageFormat::Jpeg };
    let format = image_format(arguments, default_format)?;
    let capture = Capture { window, size, format };
    Ok(Step::Wait(Wait::Settle { capture, since: Instant::now() }))
}

pub(crate) fn export_node_map(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let node = arguments.node("node", editor)?;
    let port = named_output(arguments, editor, node)?;
    let format = format_named(arguments.required_string("format")?)?;
    let mut path = PathBuf::from(arguments.required_string("path")?);
    if path.is_relative() {
        return Err("path must be absolute.".to_string());
    }
    if path.extension().is_none() {
        path.set_extension(format.extension().trim_start_matches('.'));
    }
    let folder_exists = path.parent().is_some_and(std::path::Path::is_dir);
    if !folder_exists {
        return Err(format!("The folder for {} does not exist.", path.display()));
    }
    let then = AfterBuild::Write { port, path, format };
    start_build(editor, node, None, then)
}

pub(crate) fn mark_for_export(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let editor = &mut *context.editor;
    let node = arguments.node("node", editor)?;
    let enabled = arguments.boolean("enabled")?.unwrap_or(true);
    let format = arguments.string("format")?.map(format_named).transpose()?;
    let file_name = arguments.string("file_name")?.map(str::trim).filter(|name| !name.is_empty());
    let marked = editor.document.export_settings(node);
    if !enabled {
        if marked.is_some() {
            editor.document.toggle_export(&[node], FileFormat::PureRaw);
        }
        let outcome = json!({"node": node, "marked": false});
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    let needs_enabling = marked.as_ref().is_some_and(|settings| !settings.enabled);
    if marked.is_none() {
        let first_format = format.unwrap_or_else(|| editor.user.preferences.heightfield_format());
        editor.document.toggle_export(&[node], first_format);
    }
    let edits = format.is_some() || file_name.is_some() || needs_enabling;
    if edits {
        editor.document.edit_export(node, "Export settings", |settings| {
            if let Some(format) = format {
                settings.format = format;
            }
            if let Some(name) = file_name {
                settings.filename = name.to_string();
            }
            settings.enabled = true;
        });
    }
    let settings = editor.document.export_settings(node).ok_or("The node could not be marked for export.")?;
    let outcome = json!({"node": node, "marked": true, "file_name": settings.filename, "format": format_key(settings.format)});
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn build_and_export(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let wait = arguments.boolean("wait")?.unwrap_or(true);
    let editor = &mut *context.editor;
    if editor.document.path.is_none() {
        return Err("Build and Export builds the saved project; save it first (save_project with a path).".to_string());
    }
    if editor.export_build.as_ref().is_some_and(|build| !build.finished) {
        return Err("A Build and Export is already running; get_project shows how it is going.".to_string());
    }
    let exporting = nerthus_export::build::exporting_nodes(editor.document.terrain());
    if exporting.is_empty() {
        return Err("No node is marked for export; mark_for_export marks them.".to_string());
    }
    editor.start_export_build(false);
    let started = editor.export_build.as_ref().is_some_and(|build| !build.finished);
    if editor.document.dirty || !started {
        let reason = editor.messages.last().cloned().unwrap_or_default();
        return Err(format!("Build and Export did not start: the project could not be saved. {reason}"));
    }
    if !wait {
        let outcome = json!({"started": true, "nodes": exporting.len()});
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    Ok(Step::Wait(Wait::Export))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_has_a_name() {
        for format in FileFormat::ALL {
            let key = format_key(format);
            assert_eq!(format_named(key).unwrap(), format);
        }
        assert!(format_named("bmp").is_err());
        assert_eq!(format_named("PNG16").unwrap(), FileFormat::Png16);
    }
}
