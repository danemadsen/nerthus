//! Work a call waits for on a thread of its own, reporting how far along
//! it is and stopping when the client gives up or its time runs out
//! (builds of a project file, the runtime check, calibration), and the
//! record of the builds the server ran (`build_history`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use serde_json::{json, Value};

use super::{Step, ToolResult};

/// How many builds the history keeps.
const HISTORY_LIMIT: usize = 200;

/// One build the server ran.
#[derive(Clone, Debug)]
pub(crate) struct HistoryEntry {
    pub when: String,
    pub tool: String,
    pub project: Option<String>,
    /// The node built, when it was one node.
    pub target: Option<String>,
    pub resolution: usize,
    pub seconds: f64,
    pub success: bool,
    pub problems: Vec<String>,
    pub files: usize,
    pub folder: Option<PathBuf>,
}

impl HistoryEntry {
    pub fn new(tool: &str) -> Self {
        Self {
            when: chrono::Local::now().to_rfc3339(),
            tool: tool.to_string(),
            project: None,
            target: None,
            resolution: 0,
            seconds: 0.0,
            success: true,
            problems: Vec::new(),
            files: 0,
            folder: None,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut entry = json!({
            "when": self.when,
            "tool": self.tool,
            "resolution": self.resolution,
            "seconds": (self.seconds * 100.0).round() / 100.0,
            "success": self.success,
        });
        if let Some(project) = &self.project {
            entry["project"] = json!(project);
        }
        if let Some(target) = &self.target {
            entry["node"] = json!(target);
        }
        if self.files > 0 {
            entry["files"] = json!(self.files);
        }
        if let Some(folder) = &self.folder {
            entry["folder"] = json!(folder);
        }
        if !self.problems.is_empty() {
            entry["problems"] = json!(self.problems);
        }
        entry
    }
}

/// The builds the server ran this session, oldest first.
#[derive(Default)]
pub(crate) struct BuildHistory {
    entries: Vec<HistoryEntry>,
}

impl BuildHistory {
    pub fn record(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
        if self.entries.len() > HISTORY_LIMIT {
            self.entries.remove(0);
        }
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// The folder the latest build that wrote one wrote into.
    pub fn latest_folder(&self) -> Option<&PathBuf> {
        self.entries.iter().rev().find_map(|entry| entry.folder.as_ref())
    }
}

/// What a job's thread sends.
pub(crate) enum JobMessage {
    Progress { done: f64, total: Option<f64>, message: Option<String> },
    Finished { result: ToolResult, record: Option<HistoryEntry> },
}

/// A job's thread's side: progress to report, and whether to stop.
#[derive(Clone)]
pub(crate) struct Reporter {
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
}

impl Reporter {
    pub fn progress(&self, done: f64, total: Option<f64>, message: Option<String>) {
        let _ = self.sender.send(JobMessage::Progress { done, total, message });
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The flag a build stops at.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }
}

/// A job running.
pub(crate) struct Job {
    messages: Receiver<JobMessage>,
    cancel: Arc<AtomicBool>,
    pub limit: Duration,
    progress: Option<(f64, Option<f64>, Option<String>)>,
}

impl Job {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn progress(&self) -> Option<(f64, Option<f64>, Option<String>)> {
        self.progress.clone()
    }

    /// What came from the thread: its answer once there is one (its build
    /// recorded in `history`).
    pub fn poll(&mut self, history: &mut BuildHistory) -> Option<Step> {
        loop {
            match self.messages.try_recv() {
                Ok(JobMessage::Progress { done, total, message }) => self.progress = Some((done, total, message)),
                Ok(JobMessage::Finished { result, record }) => {
                    if let Some(record) = record {
                        history.record(record);
                    }
                    return Some(Step::Done(result));
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Step::Done(ToolResult::error("The work stopped without an answer."))),
            }
        }
    }
}

/// Runs `work` on a thread named `name`, waited for for `limit` at most.
pub(crate) fn spawn(name: &str, limit: Duration, work: impl FnOnce(&Reporter) -> (ToolResult, Option<HistoryEntry>) + Send + 'static) -> Step {
    let (sender, messages) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let reporter = Reporter { sender: sender.clone(), cancel: cancel.clone() };
    let thread = std::thread::Builder::new().name(name.to_string());
    let spawned = thread.spawn(move || {
        let (result, record) = work(&reporter);
        let _ = sender.send(JobMessage::Finished { result, record });
    });
    match spawned {
        Ok(_) => Step::Wait(super::build_tools::Wait::Job(Job { messages, cancel, limit, progress: None })),
        Err(error) => Step::Done(ToolResult::error(format!("The work could not start: {error}"))),
    }
}

/// A reporter for work run outside a job (tests).
#[cfg(test)]
pub(crate) fn test_reporter(sender: Sender<JobMessage>) -> Reporter {
    Reporter { sender, cancel: Arc::new(AtomicBool::new(false)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_reports_then_answers() {
        let step = spawn("test-job", Duration::from_secs(10), |reporter| {
            reporter.progress(1.0, Some(2.0), Some("half".to_string()));
            let mut entry = HistoryEntry::new("test");
            entry.files = 3;
            (ToolResult::text("done"), Some(entry))
        });
        let Step::Wait(super::super::build_tools::Wait::Job(mut job)) = step else {
            panic!("the job did not start");
        };
        let mut history = BuildHistory::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let answer = loop {
            if let Some(step) = job.poll(&mut history) {
                break step;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(matches!(answer, Step::Done(result) if !result.is_error));
        assert_eq!(job.progress().map(|(done, _, _)| done), Some(1.0));
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].to_json()["files"], json!(3));
    }
}
