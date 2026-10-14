//! What the model is told beyond the catalogue: how the example projects
//! Nerthus comes with use each node type (how often, with which values,
//! fed by and feeding what), which modifiers read their node's input,
//! which types generate terrain of their own, and what `calibrate_nodes`
//! measured generators to make.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use nerthus_engine::{NodeData, NodeSpec, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ui::file_windows::BUNDLED_EXAMPLES;

/// The modifiers that work off their node's input (`Parent.In`): on a node
/// nothing feeds they make an empty or unchanged result.
pub(crate) const READS_NODE_INPUT: [&str; 7] = ["Difference", "Height", "Influence", "MatchHeight", "Max", "Min", "Slope"];

/// The calibration's file in the data folder.
const CALIBRATION_FILE: &str = "calibration.json";

/// A connection between two types: (from type, from port, to type, to port).
type Link = (String, String, String, String);

/// How the bundled examples use the node types.
#[derive(Default)]
pub(crate) struct ExampleUsage {
    /// How many example projects use each type.
    pub projects: HashMap<String, usize>,
    /// How often each connection between two types is made.
    pub links: HashMap<Link, usize>,
    /// Numbers set other than the default: (type, parameter) → values.
    numbers: HashMap<(String, String), Vec<f64>>,
    /// Other values set other than the default: (type, parameter) → each
    /// value's text and how often.
    choices: HashMap<(String, String), HashMap<String, usize>>,
}

/// The bundled examples' projects, by file name.
pub(crate) fn bundled_projects() -> &'static [(String, Project)] {
    static PROJECTS: OnceLock<Vec<(String, Project)>> = OnceLock::new();
    PROJECTS.get_or_init(|| {
        let mut projects = Vec::new();
        for (name, bytes) in BUNDLED_EXAMPLES {
            if !name.ends_with(".terrain") {
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim_start_matches('\u{feff}');
            match Project::parse(text) {
                Ok(project) => projects.push((name.to_string(), project)),
                Err(error) => log::warn!("the example {name} does not open: {error}"),
            }
        }
        projects
    })
}

/// The examples' usage, worked out once.
pub(crate) fn usage() -> &'static ExampleUsage {
    static USAGE: OnceLock<ExampleUsage> = OnceLock::new();
    USAGE.get_or_init(|| {
        let mut usage = ExampleUsage::default();
        for (_, project) in bundled_projects() {
            usage.add(project);
        }
        usage
    })
}

impl ExampleUsage {
    fn add(&mut self, project: &Project) {
        let nodes = &project.terrain().nodes;
        let mut types: Vec<&str> = nodes.values().map(|node| node.type_name.as_str()).collect();
        types.sort_unstable();
        types.dedup();
        for type_name in types {
            *self.projects.entry(type_name.to_string()).or_default() += 1;
        }
        for node in nodes.values() {
            self.add_values(node);
            for port in &node.ports {
                let Some(record) = &port.record else {
                    continue;
                };
                let Some(source) = nodes.get(&record.from) else {
                    continue;
                };
                let link = (source.type_name.clone(), record.from_port.clone(), node.type_name.clone(), port.name.clone());
                *self.links.entry(link).or_default() += 1;
            }
        }
    }

    fn add_values(&mut self, node: &NodeData) {
        let Some(spec) = nerthus_engine::Catalogue::get().node(&node.type_name) else {
            return;
        };
        for parameter in &spec.parameters {
            if parameter.is_seed() {
                continue;
            }
            let Some(value) = node.values.get(&parameter.name) else {
                continue;
            };
            if spec.defaults.get(&parameter.name) == Some(value) {
                continue;
            }
            let key = (node.type_name.clone(), parameter.name.clone());
            match value {
                Value::Number(number) => self.numbers.entry(key).or_default().extend(number.as_f64()),
                Value::String(text) => *self.choices.entry(key).or_default().entry(text.clone()).or_default() += 1,
                Value::Bool(flag) => *self.choices.entry(key).or_default().entry(flag.to_string()).or_default() += 1,
                _ => {}
            }
        }
    }

    /// How many examples use the type.
    pub fn project_count(&self, type_name: &str) -> usize {
        self.projects.get(type_name).copied().unwrap_or(0)
    }

    /// What feeds the type in the examples, most often first: (from type,
    /// from port, into port, times).
    pub fn fed_by(&self, type_name: &str) -> Vec<(&str, &str, &str, usize)> {
        let mut found: Vec<(&str, &str, &str, usize)> = self.links.iter().filter(|(link, _)| link.2 == type_name).map(|(link, count)| (link.0.as_str(), link.1.as_str(), link.3.as_str(), *count)).collect();
        found.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(b.0)));
        found
    }

    /// What the type feeds in the examples, most often first: (from port,
    /// to type, into port, times).
    pub fn feeds(&self, type_name: &str) -> Vec<(&str, &str, &str, usize)> {
        let mut found: Vec<(&str, &str, &str, usize)> = self.links.iter().filter(|(link, _)| link.0 == type_name).map(|(link, count)| (link.1.as_str(), link.2.as_str(), link.3.as_str(), *count)).collect();
        found.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.1.cmp(b.1)));
        found
    }

    /// What the examples set a parameter to, when they change it: how
    /// often, and the lowest, median and highest number, or the most
    /// common choice.
    pub fn parameter(&self, type_name: &str, parameter: &str) -> Option<Value> {
        let key = (type_name.to_string(), parameter.to_string());
        if let Some(values) = self.numbers.get(&key).filter(|values| !values.is_empty()) {
            let mut sorted = values.clone();
            sorted.sort_by(f64::total_cmp);
            let round = |value: f64| (value * 10_000.0).round() / 10_000.0;
            let median = sorted[sorted.len() / 2];
            return Some(json!({"times_set": sorted.len(), "low": round(sorted[0]), "median": round(median), "high": round(sorted[sorted.len() - 1])}));
        }
        let choices = self.choices.get(&key)?;
        let total: usize = choices.values().sum();
        let common = choices.iter().max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))?;
        Some(json!({"times_set": total, "most_common": common.0}))
    }
}

/// The type is a generator: it makes terrain without an input (none of
/// its inputs is required) and takes a seed.
pub(crate) fn is_generator(spec: &NodeSpec) -> bool {
    let fresh = NodeData::new(spec, 0, 0);
    let needs_input = fresh.ports.iter().any(|port| port.is_input() && port.is_required());
    let seeded = spec.parameters.iter().any(|parameter| parameter.is_seed());
    !needs_input && seeded
}

/// What one generator made at one setting.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Measurement {
    pub node_type: String,
    /// The parameter swept, none at the type's defaults.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<f64>,
    pub seeds: Vec<i64>,
    /// The share of the full height range (0 to 1) the output spans,
    /// the median across the seeds.
    pub amplitude: f64,
    /// The lowest and highest span across the seeds.
    pub amplitude_low: f64,
    pub amplitude_high: f64,
    /// The output's mean height, the median across the seeds.
    pub mean_level: f64,
}

/// Every measurement taken with this version of Nerthus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Calibration {
    pub nerthus_version: String,
    pub measured_at: String,
    pub resolution: usize,
    pub measurements: Vec<Measurement>,
}

impl Calibration {
    pub fn for_node(&self, node_type: &str) -> Vec<&Measurement> {
        self.measurements.iter().filter(|measurement| measurement.node_type == node_type).collect()
    }

    /// Folds `fresh` in: a measurement of the same type at the same setting
    /// is replaced, and everything goes when this is from another version.
    pub fn merge(existing: Option<Calibration>, fresh: Calibration) -> Calibration {
        let Some(mut merged) = existing.filter(|existing| existing.nerthus_version == fresh.nerthus_version) else {
            return fresh;
        };
        let identity = |measurement: &Measurement| (measurement.node_type.clone(), measurement.property.clone(), measurement.value.map(|value| (value * 10_000.0).round() as i64));
        for measurement in fresh.measurements {
            let key = identity(&measurement);
            match merged.measurements.iter().position(|existing| identity(existing) == key) {
                Some(index) => merged.measurements[index] = measurement,
                None => merged.measurements.push(measurement),
            }
        }
        merged.measured_at = fresh.measured_at;
        merged.resolution = fresh.resolution;
        merged
    }
}

/// Where the calibration is kept: the data folder.
pub(crate) fn calibration_path() -> Option<PathBuf> {
    crate::user::data_folder().map(|folder| folder.join(CALIBRATION_FILE))
}

/// The calibration taken with this version, if there is one.
pub(crate) fn load_calibration() -> Option<Calibration> {
    let text = std::fs::read_to_string(calibration_path()?).ok()?;
    let calibration: Calibration = serde_json::from_str(&text).ok()?;
    (calibration.nerthus_version == env!("CARGO_PKG_VERSION")).then_some(calibration)
}

pub(crate) fn save_calibration(calibration: &Calibration) -> Result<PathBuf, String> {
    let path = calibration_path().ok_or("There is no data folder to keep the calibration in.")?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("{} cannot be made: {error}", folder.display()))?;
    }
    let text = serde_json::to_string_pretty(calibration).map_err(|error| error.to_string())?;
    std::fs::write(&path, text).map_err(|error| format!("{} cannot be written: {error}", path.display()))?;
    Ok(path)
}

/// What describe_node_type reports of the type's measurements.
pub(crate) fn measured(calibration: &Calibration, node_type: &str) -> Option<Value> {
    let taken = calibration.for_node(node_type);
    if taken.is_empty() {
        return None;
    }
    let percent = |fraction: f64| (fraction * 10_000.0).round() / 100.0;
    let points: Vec<Value> = taken
        .iter()
        .map(|measurement| {
            let at = match (&measurement.property, measurement.value) {
                (Some(property), Some(value)) => format!("{property} = {value}"),
                _ => "defaults".to_string(),
            };
            json!({
                "at": at,
                "fills_range_percent": percent(measurement.amplitude),
                "mean_level_percent": percent(measurement.mean_level),
                "seed_spread_percent": [percent(measurement.amplitude_low), percent(measurement.amplitude_high)],
            })
        })
        .collect();
    Some(json!({
        "resolution": calibration.resolution,
        "note": "The share of the full height range the output spans, as built on this machine. A parameter is not an outcome: some generators scale their height with their size.",
        "points": points,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Catalogue;

    #[test]
    fn the_examples_are_read() {
        assert!(bundled_projects().len() >= 10);
        let usage = usage();
        // The examples' Erosion2 is Nerthus's ErosionPro.
        assert!(usage.project_count("ErosionPro") >= 1);
        assert!(!usage.fed_by("ErosionPro").is_empty());
        assert!(!usage.feeds("ErosionPro").is_empty());
        assert_eq!(usage.project_count("NoSuchNode"), 0);
    }

    #[test]
    fn generators_take_no_input_and_a_seed() {
        let catalogue = Catalogue::get();
        assert!(is_generator(catalogue.node("Mountain").unwrap()));
        assert!(!is_generator(catalogue.node("ErosionPro").unwrap()));
    }

    #[test]
    fn calibrations_merge_by_setting() {
        let measurement = |value: Option<f64>, amplitude: f64| Measurement {
            node_type: "Mountain".to_string(),
            property: value.map(|_| "Height".to_string()),
            value,
            seeds: vec![1],
            amplitude,
            amplitude_low: amplitude,
            amplitude_high: amplitude,
            mean_level: 0.1,
        };
        let calibration = |measurements: Vec<Measurement>| Calibration { nerthus_version: "1".to_string(), measured_at: String::new(), resolution: 512, measurements };
        let old = calibration(vec![measurement(None, 0.2), measurement(Some(0.5), 0.3)]);
        let fresh = calibration(vec![measurement(Some(0.5), 0.4), measurement(Some(1.0), 0.6)]);
        let merged = Calibration::merge(Some(old.clone()), fresh.clone());
        assert_eq!(merged.measurements.len(), 3);
        assert_eq!(merged.measurements[1].amplitude, 0.4);
        // Another version's measurements go.
        let mut other = old;
        other.nerthus_version = "0".to_string();
        assert_eq!(Calibration::merge(Some(other), fresh.clone()), fresh);
        assert!(measured(&merged, "Mountain").is_some());
        assert!(measured(&merged, "Perlin").is_none());
    }
}
