//! Long operations' progress window (`ProgressDialog`, fed by the
//! engine's progress events) while a project opens ("Opening file..."),
//! a closing project's previews are written to disk ("Freezing Cached Data
//! to Disk"), the baked maps load ("Loading Baked Cache") and the Region
//! Manager's preview renders ("Loading preview"). The work runs on the
//! build and render threads; the window follows what they report and goes
//! when the work is done. Cancel only puts it away.

use std::collections::VecDeque;

/// The window's text until the work says what it is doing.
pub const DEFAULT_TEXT: &str = "Please wait, this may take some time...";

/// What the build thread says about a long operation as it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkProgress {
    /// What is going on (`ProgressTextChanged`).
    Text(String),
    /// How far along, from 0 to 1 (`ProgressUpdated`).
    Fraction(f32),
    /// A closing project's previews are on disk.
    Frozen,
    /// A request's preparation is over (previews frozen, bakes loaded,
    /// the disk cache read back); its nodes build next.
    Prepared(u64),
}

/// What ends a step of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepEnd {
    /// The closing project's previews are written.
    Frozen,
    /// The build thread prepared a request newer than this one.
    Prepared { after: u64 },
    /// The Region Manager has its preview.
    RegionPreview,
}

/// One operation the window shows: its title, its text until the work
/// says what it is doing, and what ends it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressStep {
    pub title: String,
    pub text: String,
    pub end: StepEnd,
}

impl ProgressStep {
    pub fn new(title: &str, end: StepEnd) -> Self {
        Self::with_text(title, DEFAULT_TEXT, end)
    }

    pub fn with_text(title: &str, text: &str, end: StepEnd) -> Self {
        Self { title: title.to_string(), text: text.to_string(), end }
    }
}

/// The window: the operation under way and those after it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressWindow {
    steps: VecDeque<ProgressStep>,
    pub text: String,
    /// From 0 to 1; a new operation starts at 5%, as the bar's XAML does.
    pub fraction: f32,
    /// Put away with Cancel: the work goes on out of sight.
    pub hidden: bool,
}

impl ProgressWindow {
    pub fn new(steps: Vec<ProgressStep>) -> Self {
        let text = steps.first().map(|step| step.text.clone()).unwrap_or_default();
        Self { steps: steps.into(), text, fraction: 0.05, hidden: false }
    }

    pub fn title(&self) -> &str {
        self.steps.front().map(|step| step.title.as_str()).unwrap_or_default()
    }

    /// Whether the window is over (every operation done).
    pub fn is_done(&self) -> bool {
        self.steps.is_empty()
    }

    /// Takes in what the work reports; returns the steps it ended.
    pub fn take(&mut self, progress: &WorkProgress) -> Vec<StepEnd> {
        match progress {
            WorkProgress::Text(text) => {
                self.text = text.clone();
                Vec::new()
            }
            WorkProgress::Fraction(fraction) => {
                self.fraction = fraction.clamp(0.0, 1.0);
                Vec::new()
            }
            WorkProgress::Frozen => self.end_where(|end| end == StepEnd::Frozen),
            WorkProgress::Prepared(generation) => self.end_where(|end| matches!(end, StepEnd::Prepared { after } if *generation > after)),
        }
    }

    /// The region preview arrived.
    pub fn region_preview_ready(&mut self) -> Vec<StepEnd> {
        self.end_where(|end| end == StepEnd::RegionPreview)
    }

    /// The build thread will not get to it (the engine is suspended): the
    /// steps that wait on it end.
    pub fn abandon_work(&mut self) -> Vec<StepEnd> {
        self.end_where(|end| matches!(end, StepEnd::Frozen | StepEnd::Prepared { .. }))
    }

    /// Ends the current step while `ends` says so, starting the next
    /// afresh.
    fn end_where(&mut self, ends: impl Fn(StepEnd) -> bool) -> Vec<StepEnd> {
        let mut ended = Vec::new();
        while let Some(step) = self.steps.front() {
            if !ends(step.end) {
                break;
            }
            ended.push(step.end);
            self.steps.pop_front();
            self.text = self.steps.front().map(|next| next.text.clone()).unwrap_or_default();
            self.fraction = 0.05;
        }
        ended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_work() {
        let freezing = ProgressStep::new("Freezing Cached Data to Disk", StepEnd::Frozen);
        let opening = ProgressStep::new("Opening file...", StepEnd::Prepared { after: 4 });
        let mut window = ProgressWindow::new(vec![freezing, opening]);
        assert_eq!(window.title(), "Freezing Cached Data to Disk");
        window.take(&WorkProgress::Text("Freezing Mountain...".to_string()));
        window.take(&WorkProgress::Fraction(0.5));
        assert_eq!((window.text.as_str(), window.fraction), ("Freezing Mountain...", 0.5));
        // An older request's preparation does not end anything.
        assert!(window.take(&WorkProgress::Prepared(4)).is_empty());
        assert_eq!(window.take(&WorkProgress::Frozen), vec![StepEnd::Frozen]);
        assert_eq!((window.title(), window.text.as_str()), ("Opening file...", DEFAULT_TEXT));
        assert_eq!(window.take(&WorkProgress::Prepared(5)), vec![StepEnd::Prepared { after: 4 }]);
        assert!(window.is_done());
    }
}
