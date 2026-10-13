//! Update checks (`UpdateService`): the latest release of Nerthus, read
//! from GitHub's releases in the background (with curl, through the proxy
//! Options › General names), as Nerthus starts, after which an Update
//! button shows in the title bar; Help › Check for updates asks again and
//! says what it found. Updating opens the release's page.

use std::process::Command;

use crossbeam_channel::{Receiver, TryRecvError};
use serde_json::Value;

/// `UpdateToken`: what a release says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateToken {
    /// The version, as the release's tag names it (without its `v`).
    pub version: String,
    /// `ToShortDateString` of the release's date (`M/d/yyyy`).
    pub date: String,
    /// The download for this system, when the release has one.
    pub size: Option<u64>,
    /// The release's page.
    pub url: String,
    pub notes: String,
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
/// (`LastManifest`).
#[derive(Default)]
pub struct Updates {
    running: Option<Receiver<Result<Option<UpdateToken>, String>>>,
    /// The check was asked for (Help › Check for updates), so its answer
    /// is shown.
    asked: bool,
    last: Option<UpdateToken>,
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

/// The latest release through GitHub's API; none when the repository has
/// published none.
fn fetch_latest(proxy: &ProxySettings) -> Result<Option<UpdateToken>, String> {
    let repository = env!("CARGO_PKG_REPOSITORY");
    let path = repository.trim_start_matches("https://github.com/");
    let address = format!("https://api.github.com/repos/{path}/releases/latest");
    let user_agent = concat!("User-Agent: Nerthus/", env!("CARGO_PKG_VERSION"));
    let mut curl = Command::new("curl");
    curl.args(["--silent", "--show-error", "--location", "--max-time", "20"]);
    curl.args(["--header", "Accept: application/vnd.github+json"]);
    curl.args(["--header", user_agent]);
    curl.args(["--write-out", "\n%{http_code}"]);
    // `HttpClientHandler.UseProxy`: off means no proxy at all; on, the one
    // named, else the system's.
    if !proxy.use_proxy {
        curl.args(["--noproxy", "*"]);
    } else if !proxy.proxy.is_empty() {
        curl.args(["--proxy", proxy.proxy.as_str()]);
    }
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
    UpdateToken { version, date, size, url: text("html_url"), notes: text("body") }
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
                {"name": "nerthus-linux.tar.gz", "size": 10},
                {"name": "nerthus-macos.zip", "size": 20},
                {"name": "nerthus-windows.zip", "size": 30},
            ],
        });
        let token = token_from(&release);
        assert_eq!(token.version, "0.3.1");
        assert_eq!(token.date, "9/4/2026");
        assert_eq!(token.notes, "Notes");
        assert!(token.size.is_some());
    }
}
