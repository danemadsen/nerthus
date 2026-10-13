//! Prints the resources stored next to a `.tor` project.

fn main() {
    let path = std::env::args().nth(1).expect("usage: inspect_resources <project.tor>");
    let store = nerthus_io::resources::ResourceStore::for_project(std::path::Path::new(&path));
    for entry in &store.entries {
        let size = match &entry.data {
            nerthus_io::resources::ResourceData::Bytes(bytes) => format!("{} bytes", bytes.len()),
            nerthus_io::resources::ResourceData::Floats(values) => format!("{} floats", values.len()),
            nerthus_io::resources::ResourceData::Other => "other".to_string(),
        };
        println!("{:?} id={} parent={} {size} path={:?}", entry.kind, entry.id, entry.parent, entry.original_path);
    }
}
