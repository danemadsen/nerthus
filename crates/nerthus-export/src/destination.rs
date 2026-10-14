//! Where a build writes: the build destination template and the file
//! names in it.
//!
//! Project paths are joined with `\`. Templates and names keep that form while
//! tokens are replaced; [`to_path`] turns the result into a path of this
//! system, taking both `\` and `/` as separators.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

/// The template when the build definition has none.
pub const DEFAULT_TEMPLATE: &str = "<Builds>\\[Filename]\\[+++]";

/// The presets Build Settings offers.
pub const TEMPLATE_PRESETS: [&str; 8] = [
    "<Builds>\\[Filename]\\[+++]\\",
    "<Builds>\\[Filename]\\[Date]\\[+++]\\",
    "<Builds>\\[Date]\\[Filename]\\[+++]\\",
    "<Builds>\\[Filename]\\[Timestamp]",
    "<FileLocation>\\[Filename]\\Builds\\[Timestamp]",
    "<FileLocation>\\[Filename]\\Builds\\[Date]\\[+++]\\",
    "<FileLocation>\\[Date]\\[Filename]\\[+++]\\",
    "<FileLocation>\\[Filename]\\Builds\\[+++]\\",
];

/// The help text Build Settings shows under the destination.
pub const TOKEN_HELP: &str = "The following tokens can be added to the Build Path above. Each token will be replaced with the corresponding value when the build is executed.\n\n<FileLocation>: Directory of the active file.\n<Builds>: Location of build files.\n[Mutation]: Current Mutation.\n[Region]: Name of the current Region, if any.\n[Timestamp]: Current timestamp (file time format).\n[Filename]: Active file name (without extension).\n[Username]: Logged-in user name.\n[Domain]: User domain.\n[Machine]: Host machine name.\n[Time]: Current time.\n[Date]: Current date.\n[Resolution]: Build resolution (X value).";

/// What the destination tokens stand for.
#[derive(Clone, Debug)]
pub struct DestinationContext {
    /// `BuildDefinition.Destination` (empty for the default).
    pub template: String,
    /// The project file.
    pub project_path: PathBuf,
    /// `Paths.Builds`.
    pub builds_root: PathBuf,
    /// The region being built, if any.
    pub region: Option<String>,
    pub mutation: i32,
    /// The build resolution's width.
    pub resolution: usize,
    pub now: DateTime<Local>,
    pub user_name: String,
    pub domain: String,
    pub machine: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn machine_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// A browser does not tell pages the machine's name.
#[cfg(target_arch = "wasm32")]
fn machine_name() -> String {
    String::new()
}

impl DestinationContext {
    /// The context of a build of `project_path` now, by this user on this
    /// machine.
    pub fn new(template: &str, project_path: &Path, resolution: usize) -> Self {
        let machine = machine_name();
        let user_name = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        // .NET's `UserDomainName` is the machine name outside a Windows
        // domain.
        let domain = std::env::var("USERDOMAIN").unwrap_or_else(|_| machine.clone());
        Self {
            template: template.to_string(),
            project_path: project_path.to_path_buf(),
            builds_root: default_builds_root(),
            region: None,
            mutation: 0,
            resolution,
            now: Local::now(),
            user_name,
            domain,
            machine,
        }
    }
}

/// `Paths.Builds` by default: Documents/Nerthus/Builds.
pub fn default_builds_root() -> PathBuf {
    let user_directories = directories::UserDirs::new();
    let documents = user_directories.as_ref().and_then(|directories| directories.document_dir().map(Path::to_path_buf));
    let base = documents.unwrap_or_else(|| PathBuf::from("."));
    base.join("Nerthus").join("Builds")
}

/// A project path string as a path of this system.
pub fn to_path(text: &str) -> PathBuf {
    let absolute = text.starts_with('/') || text.starts_with('\\');
    let mut path = if absolute { PathBuf::from("/") } else { PathBuf::new() };
    for part in text.split(['\\', '/']) {
        if part.is_empty() {
            continue;
        }
        path.push(part);
    }
    path
}

fn is_absolute(text: &str) -> bool {
    let has_drive = text.len() >= 2 && text.as_bytes()[1] == b':';
    has_drive || to_path(text).is_absolute()
}

/// The last three characters and what comes before them (.NET's
/// `Substring` on a name).
fn split_last_three(text: &str) -> (String, String) {
    let characters: Vec<char> = text.chars().collect();
    let split = characters.len().saturating_sub(3);
    let head: String = characters[..split].iter().collect();
    let tail: String = characters[split..].iter().collect();
    (head, tail)
}

/// The next numbered folder: "001" → "002" (anything unparsable → "001").
pub fn next_counter(counter: &str) -> String {
    let tail = if counter.chars().count() < 3 { "000".to_string() } else { split_last_three(counter).1 };
    let value: i64 = tail.trim().parse().unwrap_or(0);
    format!("{:03}", value + 1)
}

/// The build folder for this build.
pub fn resolve_destination(context: &DestinationContext) -> PathBuf {
    let mut text = if context.template.is_empty() { DEFAULT_TEMPLATE.to_string() } else { context.template.clone() };
    let project_directory = context.project_path.parent().map(|directory| directory.to_string_lossy().into_owned()).unwrap_or_default();
    if text.starts_with('\\') {
        let base = if Path::new(&project_directory).is_dir() { project_directory.clone() } else { context.builds_root.to_string_lossy().into_owned() };
        text = format!("{base}{text}");
    }
    text = text.trim_end_matches('\\').to_string();

    let file_name = context.project_path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let region = context.region.clone().unwrap_or_else(|| "RegionName".to_string());
    let replacements = [
        ("<FileLocation>", project_directory),
        ("<Builds>", context.builds_root.to_string_lossy().into_owned()),
        ("[Region]", region),
        ("[Mutation]", format!("{:03}", context.mutation)),
        ("[Timestamp]", context.now.format("%Y-%m-%d_%H-%M-%S").to_string()),
        ("[Filename]", file_name),
        ("[Username]", context.user_name.clone()),
        ("[Domain]", context.domain.clone()),
        ("[Machine]", context.machine.clone()),
        ("[Time]", context.now.format("%H-%M-%S").to_string()),
        ("[Date]", context.now.format("%Y-%m-%d").to_string()),
        ("[Resolution]", context.resolution.to_string()),
    ];
    for (token, value) in replacements {
        text = text.replace(token, &value);
    }
    if !is_absolute(&text) {
        let current = std::env::current_dir().unwrap_or_default();
        text = format!("{}\\{text}", current.to_string_lossy());
    }

    // `[+++]`: a numbered folder at the end, the first that is free.
    // `\[+++]` is removed; `/[+++]` is the same here.
    if text.contains("[+++]") {
        let base = text.replace("\\[+++]", "").replace("/[+++]", "");
        let mut counter = "001".to_string();
        let mut candidate = format!("{base}\\{counter}");
        while to_path(&candidate).is_dir() {
            counter = next_counter(&counter);
            candidate = format!("{base}\\{counter}");
        }
        text = candidate;
    }
    to_path(&text)
}

/// The next numbered file: when `path` exists, the first free name
/// ending in a three-digit number (`Name_Out001.exr`, `002`, …; a name
/// already ending in digits continues from them).
pub fn next_free_file(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default();
    let (head, tail) = if stem.chars().count() <= 3 { (stem.clone(), String::new()) } else { split_last_three(&stem) };
    let (base, mut number) = match tail.trim().parse::<i64>() {
        Ok(value) => (head, value),
        Err(_) => (stem.clone(), 0),
    };
    if number == 0 {
        number = 1;
    }
    loop {
        let candidate = directory.join(format!("{base}{number:03}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

/// A file name in the build folder (unless already a
/// full path), made unique when files are not overwritten.
pub fn place_in_folder(name: &str, folder: &Path, increment: bool) -> PathBuf {
    let path = if is_absolute(name) { to_path(name) } else { folder.join(to_path(name)) };
    if increment {
        return next_free_file(&path);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn context(template: &str, root: &Path) -> DestinationContext {
        DestinationContext {
            template: template.to_string(),
            project_path: root.join("Projects").join("Canyon.terrain"),
            builds_root: root.join("Builds"),
            region: None,
            mutation: 3,
            resolution: 2048,
            now: Local.with_ymd_and_hms(2026, 10, 12, 9, 5, 7).unwrap(),
            user_name: "dane".to_string(),
            domain: "studio".to_string(),
            machine: "mac".to_string(),
        }
    }

    #[test]
    fn default_template_numbers_builds() {
        let root = std::env::temp_dir().join(format!("nerthus-destination-{}", std::process::id()));
        let first = resolve_destination(&context("", &root));
        assert_eq!(first, root.join("Builds").join("Canyon").join("001"));
        std::fs::create_dir_all(&first).unwrap();
        let second = resolve_destination(&context("", &root));
        assert_eq!(second, root.join("Builds").join("Canyon").join("002"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tokens_are_replaced() {
        let root = std::env::temp_dir().join("nerthus-destination-tokens");
        let template = "<FileLocation>\\[Filename]\\Builds\\[Date]_[Time]\\[Mutation]-[Resolution]-[Username]@[Machine]";
        let folder = resolve_destination(&context(template, &root));
        let expected = root.join("Projects").join("Canyon").join("Builds").join("2026-10-12_09-05-07").join("003-2048-dane@mac");
        assert_eq!(folder, expected);
    }

    #[test]
    fn numbered_folders_take_either_separator() {
        let root = std::env::temp_dir().join("nerthus-destination-slashes");
        let template = format!("{}/Out/[Filename]/[+++]", root.display());
        let folder = resolve_destination(&context(&template, &root));
        assert_eq!(folder, root.join("Out").join("Canyon").join("001"));
    }

    #[test]
    fn counters_count() {
        assert_eq!(next_counter("001"), "002");
        assert_eq!(next_counter("009"), "010");
        assert_eq!(next_counter("x"), "001");
    }

    #[test]
    fn builds_default_to_the_nerthus_folder() {
        let nerthus_builds = Path::new("Nerthus").join("Builds");
        assert!(default_builds_root().ends_with(&nerthus_builds));
    }
}
