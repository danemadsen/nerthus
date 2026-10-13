//! Every example project in the folder `NERTHUS_EXAMPLES` names loads.

use std::fs;
use std::path::PathBuf;

use nerthus_engine::Project;

fn examples_directory() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("NERTHUS_EXAMPLES")?);
    directory.is_dir().then_some(directory)
}

#[test]
fn example_projects_load() {
    let Some(directory) = examples_directory() else {
        eprintln!("NERTHUS_EXAMPLES is not an examples folder; skipping");
        return;
    };
    let mut loaded = 0;
    for entry in fs::read_dir(directory).expect("examples are readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("terrain") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("example is readable");
        let project = Project::parse(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(project.unknown_types.is_empty(), "{}: unknown node types {:?}", path.display(), project.unknown_types);
        let terrain = project.terrain();
        assert!(!terrain.nodes.is_empty(), "{} has no nodes", path.display());
        for node in terrain.nodes.values() {
            for port in &node.ports {
                if let Some(record) = &port.record {
                    assert_eq!(record.to, node.id, "{}: record on node {} points elsewhere", path.display(), node.id);
                }
            }
        }
        loaded += 1;
    }
    assert!(loaded > 50, "only {loaded} examples found");
}
