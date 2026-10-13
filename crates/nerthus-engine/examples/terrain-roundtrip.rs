//! Reads `.terrain` files and writes them back, reporting where the output
//! differs from the input.
//!
//! terrain-roundtrip <file.terrain>... [--show <lines>]

use std::fs;

use nerthus_engine::{to_terrain_json, Project};

fn main() {
    let mut paths = Vec::new();
    let mut show = 3usize;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--show" {
            show = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(3);
            continue;
        }
        paths.push(argument);
    }

    let mut identical = 0;
    for path in &paths {
        let original = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                println!("{path}: cannot read: {error}");
                continue;
            }
        };
        let project = match Project::parse(&original) {
            Ok(project) => project,
            Err(error) => {
                println!("{path}: cannot parse: {error}");
                continue;
            }
        };
        let written = to_terrain_json(&project);
        let original_text = original.trim_start_matches('\u{feff}');
        if written == original_text {
            identical += 1;
            continue;
        }
        let original_lines: Vec<&str> = original_text.split("\r\n").collect();
        let written_lines: Vec<&str> = written.split("\r\n").collect();
        let first = original_lines.iter().zip(&written_lines).position(|(left, right)| left != right).unwrap_or(original_lines.len().min(written_lines.len()));
        println!("{path}: differs at line {} ({} vs {} lines)", first + 1, original_lines.len(), written_lines.len());
        let start = first.saturating_sub(2);
        let original_end = (first + show).min(original_lines.len());
        for line in &original_lines[start..original_end] {
            println!("  - {line}");
        }
        let written_end = (first + show).min(written_lines.len());
        for line in &written_lines[start..written_end] {
            println!("  + {line}");
        }
    }
    println!("{identical} of {} identical", paths.len());
}
