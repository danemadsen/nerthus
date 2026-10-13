//! Update checks (`UpdateService`): the latest release of Nerthus, read
//! from GitHub's releases in the background (with curl, through the proxy
//! Options › General names), as Nerthus starts, after which an Update
//! button shows in the title bar; Help › Check for updates asks again and
//! says what it found. Updating downloads the release's file for this
//! system into the data folder's `Updates` (the Update button counting
//! the percent), then runs it, closing Nerthus first if asked to. A
//! release without a file for this system opens its page instead.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use serde_json::Value;

/// `UpdateToken`: what a release says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateToken {
    /// The version, as the release's tag names it (without its `v`).
    pub version: String,
    /// `ToShortDateString` of the release's date (`M/d/yyyy`).
    pub date: String,
    /// The size of the download for this system, when the release has
    /// one.
    pub size: Option<u64>,
    /// The release's page.
    pub url: String,
    pub notes: String,
    /// Where the download for this system is.
    pub download: Option<String>,
}

impl UpdateToken {
    /// Whether this release is newer than the running Nerthus.
    pub fn is_newer(&self) -> bool {
        is_newer(&self.version, env!("CARGO_PKG_VERSION"))
    }
}

/// What a check found when it was asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateAnswer {
    UpToDate,
    Available(UpdateToken),
    Failed(String),
}

/// The checks: the one running, and what the last one found
/// (`LastManifest`); and the update's download.
#[derive(Default)]
pub struct Updates {
    running: Option<Receiver<Result<Option<UpdateToken>, String>>>,
    /// The check was asked for (Help › Check for updates), so its answer
    /// is shown.
    asked: bool,
    last: Option<UpdateToken>,
    download: Option<Download>,
}

/// The update being downloaded, or downloaded.
struct Download {
    events: Receiver<DownloadEvent>,
    /// How much has arrived, in percent.
    percent: f64,
    finished: bool,
}

/// What the download's worker says.
enum DownloadEvent {
    Progress(f64),
    Finished(PathBuf),
    Failed(String),
}

/// How a download ended.
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadOutcome {
    /// The installer, ready to run.
    Finished(PathBuf),
    Failed(String),
}

/// `Locations.UseProxy` and `Locations.Proxy`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProxySettings {
    pub use_proxy: bool,
    pub proxy: String,
}

impl Updates {
    /// A check in the background (unless one is running).
    pub fn start(&mut self, proxy: ProxySettings, asked: bool) {
        self.asked |= asked;
        if self.running.is_some() {
            return;
        }
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("update check".to_string()).spawn(move || {
            let _ = sender.send(fetch_latest(&proxy));
        });
        match spawned {
            Ok(_) => self.running = Some(receiver),
            Err(error) => log::error!("Error loading update information. {error}"),
        }
    }

    /// Help › Check for updates (`CheckForUpdates`): the release known
    /// already, else a new check whose answer comes later.
    pub fn ask(&mut self, proxy: ProxySettings) -> Option<UpdateAnswer> {
        if let Some(token) = &self.last {
            return Some(answer_for(Some(token.clone())));
        }
        self.start(proxy, true);
        None
    }

    /// Whether a check is running.
    pub fn checking(&self) -> bool {
        self.running.is_some()
    }

    /// A newer release is known (the title bar's Update button).
    pub fn update_available(&self) -> bool {
        self.last.as_ref().is_some_and(UpdateToken::is_newer)
    }

    /// Download Update: the release's file for this system fetched on a
    /// worker (unless it is being fetched already).
    pub fn start_download(&mut self, token: &UpdateToken, proxy: ProxySettings) {
        if self.download.as_ref().is_some_and(|download| !download.finished) {
            return;
        }
        let Some(address) = token.download.clone() else {
            return;
        };
        let Some(folder) = crate::files::data_subfolder("Updates") else {
            log::error!("Nerthus has no data folder to download the update to.");
            return;
        };
        let size = token.size;
        let (sender, receiver) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new().name("update download".to_string()).spawn(move || {
            download(&address, &proxy, &folder, size, &sender);
        });
        match spawned {
            Ok(_) => self.download = Some(Download { events: receiver, percent: 0.0, finished: false }),
            Err(error) => log::error!("{error}"),
        }
    }

    /// The Update button's text while the update downloads, and once it
    /// has: `Downloading 12.3%...`, then `Downloaded`.
    pub fn download_label(&self) -> Option<String> {
        let download = self.download.as_ref()?;
        if download.finished || download.percent >= 100.0 {
            return Some("Downloaded".to_string());
        }
        Some(format!("Downloading {:.1}%...", download.percent))
    }

    /// Whether the download is running.
    pub fn downloading(&self) -> bool {
        self.download.as_ref().is_some_and(|download| !download.finished)
    }

    /// What the download said since the last frame: its progress, and how
    /// it ended (once). A failed download can be tried again.
    pub fn poll_download(&mut self) -> Option<DownloadOutcome> {
        let download = self.download.as_mut()?;
        let mut outcome = None;
        for event in download.events.try_iter() {
            match event {
                DownloadEvent::Progress(percent) => download.percent = percent,
                DownloadEvent::Finished(path) => {
                    download.finished = true;
                    outcome = Some(DownloadOutcome::Finished(path));
                }
                DownloadEvent::Failed(error) => outcome = Some(DownloadOutcome::Failed(error)),
            }
        }
        if matches!(outcome, Some(DownloadOutcome::Failed(_))) {
            self.download = None;
        }
        outcome
    }

    /// The finished check's answer, when it was asked for.
    pub fn poll(&mut self) -> Option<UpdateAnswer> {
        let receiver = self.running.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("The update check stopped.".to_string()),
        };
        self.running = None;
        let asked = std::mem::take(&mut self.asked);
        let answer = match result {
            Ok(token) => {
                self.last = token.clone();
                answer_for(token)
            }
            Err(error) => {
                log::error!("Error loading update information.");
                log::error!("{error}");
                UpdateAnswer::Failed(error)
            }
        };
        asked.then_some(answer)
    }
}

fn answer_for(token: Option<UpdateToken>) -> UpdateAnswer {
    match token {
        Some(token) if token.is_newer() => UpdateAnswer::Available(token),
        _ => UpdateAnswer::UpToDate,
    }
}

/// The repository's releases (`https://github.com/<owner>/<name>`).
pub fn releases_page() -> String {
    format!("{}/releases", env!("CARGO_PKG_REPOSITORY"))
}

/// curl, quiet but for errors, following redirects, as Nerthus, through
/// the proxy (`HttpClientHandler.UseProxy`: off means no proxy at all;
/// on, the one named, else the system's).
fn curl(proxy: &ProxySettings) -> Command {
    let user_agent = concat!("User-Agent: Nerthus/", env!("CARGO_PKG_VERSION"));
    let mut curl = Command::new("curl");
    curl.args(["--silent", "--show-error", "--location"]);
    curl.args(["--header", user_agent]);
    if !proxy.use_proxy {
        curl.args(["--noproxy", "*"]);
    } else if !proxy.proxy.is_empty() {
        curl.args(["--proxy", proxy.proxy.as_str()]);
    }
    curl
}

/// The latest release through GitHub's API; none when the repository has
/// published none.
fn fetch_latest(proxy: &ProxySettings) -> Result<Option<UpdateToken>, String> {
    let repository = env!("CARGO_PKG_REPOSITORY");
    let path = repository.trim_start_matches("https://github.com/");
    let address = format!("https://api.github.com/repos/{path}/releases/latest");
    let mut curl = curl(proxy);
    curl.args(["--max-time", "20"]);
    curl.args(["--header", "Accept: application/vnd.github+json"]);
    curl.args(["--write-out", "\n%{http_code}"]);
    curl.arg(&address);
    let output = curl.output().map_err(|error| format!("Cannot run curl: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(message);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", text.as_ref()));
    match status.trim() {
        "200" => {}
        // No release published yet.
        "404" => return Ok(None),
        code => return Err(format!("GitHub answered {code}.")),
    }
    let release: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    Ok(Some(token_from(&release)))
}

/// A GitHub release as an `UpdateToken`.
fn token_from(release: &Value) -> UpdateToken {
    let text = |name: &str| release.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
    let tag = text("tag_name");
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let published = text("published_at");
    let date = match chrono::DateTime::parse_from_rfc3339(&published) {
        Ok(date) => date.format("%-m/%-d/%Y").to_string(),
        Err(_) => published,
    };
    let assets = release.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let asset = system_asset(&assets);
    let size = asset.and_then(|asset| asset.get("size")?.as_u64());
    let download = asset.and_then(|asset| asset.get("browser_download_url")?.as_str()).map(str::to_string);
    UpdateToken { version, date, size, url: text("html_url"), notes: text("body"), download }
}

/// `DownloadUpdate`: the file at `address` written to `folder` under its
/// own name (any older copy deleted first), its progress reported while
/// curl fetches it.
fn download(address: &str, proxy: &ProxySettings, folder: &Path, size: Option<u64>, events: &Sender<DownloadEvent>) {
    let name = address.rsplit('/').next().unwrap_or("nerthus-update").to_string();
    let path = folder.join(&name);
    if let Err(error) = std::fs::create_dir_all(folder) {
        let _ = events.send(DownloadEvent::Failed(error.to_string()));
        return;
    }
    if path.exists() && std::fs::remove_file(&path).is_err() {
        let _ = events.send(DownloadEvent::Failed(format!("Cannot replace {}.", path.display())));
        return;
    }
    let mut curl = curl(proxy);
    curl.arg("--fail");
    curl.arg("--output").arg(&path);
    curl.arg(address);
    curl.stderr(std::process::Stdio::piped());
    let mut child = match curl.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = events.send(DownloadEvent::Failed(format!("Cannot run curl: {error}")));
            return;
        }
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = events.send(DownloadEvent::Failed(error.to_string()));
                return;
            }
        }
        if let Some(total) = size.filter(|total| *total > 0) {
            let arrived = std::fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
            let percent = arrived as f64 / total as f64 * 100.0;
            let _ = events.send(DownloadEvent::Progress(percent.min(99.9)));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if !status.success() {
        let mut error = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut stderr, &mut error);
        }
        let error = error.trim().to_string();
        log::error!("{error}");
        let _ = events.send(DownloadEvent::Failed(error));
        return;
    }
    let _ = events.send(DownloadEvent::Progress(100.0));
    let _ = events.send(DownloadEvent::Finished(path));
}

/// Runs the downloaded installer: opens it as the system would (an
/// installer program runs; a disk image or archive opens).
pub fn run_installer(path: &Path) {
    log::info!("Running the update {}", path.display());
    crate::export_build::open_url(&path.display().to_string());
}

/// The release's download for this system: the first whose name mentions
/// it.
fn system_asset(assets: &[Value]) -> Option<&Value> {
    let names: &[&str] = match std::env::consts::OS {
        "macos" => &["macos", "darwin", "mac"],
        "windows" => &["windows", "win64", "win"],
        _ => &["linux"],
    };
    assets.iter().find(|asset| {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or_default().to_lowercase();
        names.iter().any(|part| name.contains(part))
    })
}

/// `Version.op_GreaterThan`: the numbers before any `-` compared in turn,
/// missing ones as zero.
fn is_newer(candidate: &str, current: &str) -> bool {
    let numbers = |version: &str| -> Vec<u64> {
        let core = version.split(['-', '+']).next().unwrap_or_default();
        core.split('.').map(|part| part.trim().parse().unwrap_or(0)).collect()
    };
    let candidate_numbers = numbers(candidate);
    let current_numbers = numbers(current);
    let length = candidate_numbers.len().max(current_numbers.len());
    for index in 0..length {
        let candidate_part = candidate_numbers.get(index).copied().unwrap_or(0);
        let current_part = current_numbers.get(index).copied().unwrap_or(0);
        if candidate_part != current_part {
            return candidate_part > current_part;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn releases_become_tokens() {
        let release = serde_json::json!({
            "tag_name": "v0.3.1",
            "published_at": "2026-09-04T10:00:00Z",
            "html_url": "https://github.com/owner/nerthus/releases/tag/v0.3.1",
            "body": "Notes",
            "assets": [
                {"name": "nerthus-linux.tar.gz", "size": 10, "browser_download_url": "https://example.com/nerthus-linux.tar.gz"},
                {"name": "nerthus-macos.zip", "size": 20, "browser_download_url": "https://example.com/nerthus-macos.zip"},
                {"name": "nerthus-windows.zip", "size": 30, "browser_download_url": "https://example.com/nerthus-windows.zip"},
            ],
        });
        let token = token_from(&release);
        assert_eq!(token.version, "0.3.1");
        assert_eq!(token.date, "9/4/2026");
        assert_eq!(token.notes, "Notes");
        assert!(token.size.is_some());
        let expected = match std::env::consts::OS {
            "macos" => "https://example.com/nerthus-macos.zip",
            "windows" => "https://example.com/nerthus-windows.zip",
            _ => "https://example.com/nerthus-linux.tar.gz",
        };
        assert_eq!(token.download.as_deref(), Some(expected));
    }

    #[test]
    fn the_button_counts_the_download() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let mut updates = Updates { download: Some(Download { events: receiver, percent: 0.0, finished: false }), ..Updates::default() };
        assert_eq!(updates.download_label().as_deref(), Some("Downloading 0.0%..."));
        sender.send(DownloadEvent::Progress(12.345)).unwrap();
        assert_eq!(updates.poll_download(), None);
        assert_eq!(updates.download_label().as_deref(), Some("Downloading 12.3%..."));
        sender.send(DownloadEvent::Finished(PathBuf::from("setup.exe"))).unwrap();
        assert_eq!(updates.poll_download(), Some(DownloadOutcome::Finished(PathBuf::from("setup.exe"))));
        assert_eq!(updates.download_label().as_deref(), Some("Downloaded"));
        assert!(!updates.downloading());
    }
}
