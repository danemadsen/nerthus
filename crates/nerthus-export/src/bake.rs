//! Baking: a node and everything upstream of it built once at
//! the bake resolution, each node's outputs kept on disk and the nodes
//! marked `Baked`, so later builds take the stored maps instead of
//! building them again. Hybrid and tiled builds need it: they build the
//! terrain a bucket at a time and cannot run generators on a part of the
//! terrain, so they cut each bucket out of the baked maps instead.
//!
//! The maps are `Cache.Offload` files: raw floats plus a `.meta`, named
//! `<node>_<port>.bin` in `Cache\Bakes\<file name>_<project id>` next to
//! the project (or under the central cache folder).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nerthus_engine::{BuildResult, Catalogue, Engine, Map, NodeData, NodeState, Region, Resolution, StateOverride, Terrain};

use crate::raw;
use crate::tiles;

/// Where a project's baked maps are (`Cache.Filename` for
/// `CacheClusters.Bakes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeStore {
    folder: PathBuf,
}

impl BakeStore {
    /// The `Bakes` cache cluster's folder, then `<file name>_<project id>`.
    /// With `Locations.CentralizeCacheBake` the cluster folder is under
    /// `Locations.Cache`; that string is never interpolated, so the
    /// folder is literally named `{cluster}`.
    pub fn new(project_path: &Path, project_id: &str, central_cache: Option<&Path>) -> Self {
        let cluster_folder = match central_cache {
            Some(root) => root.join("{cluster}"),
            None => {
                let project_folder = project_path.parent().unwrap_or(Path::new("."));
                project_folder.join("Cache").join("Bakes")
            }
        };
        let stem = project_path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        let folder = cluster_folder.join(format!("{stem}_{project_id}"));
        Self { folder }
    }

    /// `Cache.Filename`'s folder for `CacheClusters.Primary`: the previews
    /// Smart Cache and Cache to Disk keep, in `Cache\Primary\<file
    /// name>_<project id>` beside the project, or under the central cache
    /// folder (`Locations.CentralizeCacheFreeze`, whose cluster folder is
    /// named `{cluster}` as the bakes' is).
    pub fn previews(project_path: &Path, project_id: &str, central_cache: Option<&Path>) -> Self {
        let cluster_folder = match central_cache {
            Some(root) => root.join("{cluster}"),
            None => {
                let project_folder = project_path.parent().unwrap_or(Path::new("."));
                project_folder.join("Cache").join("Primary")
            }
        };
        let stem = project_path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        let folder = cluster_folder.join(format!("{stem}_{project_id}"));
        Self { folder }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// `Cache.Filename`.
    pub fn file(&self, node: i32, port: &str) -> PathBuf {
        self.folder.join(format!("{node}_{port}.bin"))
    }

    /// `Cache.Offload`: the map written as a `.graw` (a one-channel map in
    /// the stored range), its `.meta` ending with the cache's thumbprint
    /// (`GUID`, a fresh one per build).
    pub fn write(&self, node: i32, port: &str, map: &Map) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let thumbprint = uuid::Uuid::new_v4().simple().to_string();
        let extra = [("GUID", thumbprint)];
        let range = nerthus_nodes::graw::stored_range();
        raw::write_graw_with(&self.file(node, port), map, range, &extra)
    }

    /// `Cache.Reload`: the stored map with its intent (a map stored in
    /// another range comes back in [0, 1] and sets the stored range), or
    /// `None` when there is no file.
    pub fn read(&self, node: i32, port: &str) -> io::Result<Option<Map>> {
        let path = self.file(node, port);
        if !path.exists() {
            return Ok(None);
        }
        let (map, _) = nerthus_nodes::graw::read(&path).map_err(io::Error::other)?;
        Ok(Some(map))
    }

    /// A whole node's stored maps: every output port's stored map,
    /// or `None` when one is missing (the node then builds as usual).
    pub fn read_node(&self, node: &NodeData) -> Option<Vec<(String, Map)>> {
        let mut maps = Vec::new();
        for port in node.ports.iter().filter(|port| port.is_output()) {
            let map = self.read(node.id, &port.name).ok()??;
            maps.push((port.name.clone(), map));
        }
        Some(maps)
    }

    /// `Cache.DeleteOffloaded` for each of a node's ports.
    pub fn delete_node(&self, node: &NodeData) {
        let mut ports: Vec<String> = node.ports.iter().filter(|port| port.is_output()).map(|port| port.name.clone()).collect();
        ports.push("Unmodified".to_string());
        for port in ports {
            let path = self.file(node.id, &port);
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(path.with_extension("meta"));
        }
    }

    /// Whether a node's maps are all on disk.
    pub fn has_node(&self, node: &NodeData) -> bool {
        node.ports.iter().filter(|port| port.is_output()).all(|port| self.file(node.id, &port.name).exists())
    }
}

/// A node's `SaveDefinition`, if it has one.
fn has_save_definition(node: &NodeData) -> bool {
    node.base.get("SaveDefinition").is_some_and(|value| !value.is_null())
}

/// Whether `consumer` takes an input from `source`.
fn takes_input_from(consumer: &NodeData, source: i32) -> bool {
    consumer.ports.iter().any(|port| port.is_input() && port.record.as_ref().is_some_and(|record| record.from == source))
}

/// `Port.IsConnected` for an output port: some node's input reads it.
fn output_is_connected(terrain: &Terrain, node: i32, port: &str) -> bool {
    terrain.nodes.values().any(|other| {
        other.ports.iter().any(|input| input.is_input() && input.record.as_ref().is_some_and(|record| record.from == node && record.from_port == port))
    })
}

/// Whether a baked node's maps must be loaded — a node
/// that is not baked reads them, a command-line build exports it (it has a
/// `SaveDefinition`), or nothing reads any of its outputs.
pub fn is_crucial(terrain: &Terrain, node: &NodeData, swarm: bool) -> bool {
    let read_by_unbaked = terrain.nodes.values().any(|other| StateOverride::of(other) != StateOverride::Baked && takes_input_from(other, node.id));
    if swarm && has_save_definition(node) {
        return true;
    }
    let outputs_unread = node.ports.iter().filter(|port| port.is_output()).all(|port| !output_is_connected(terrain, node.id, &port.name));
    if outputs_unread {
        return true;
    }
    read_by_unbaked
}

/// Nodes nothing is connected to or from that do not
/// export. (The test also rules out every node with a connected input,
/// through `Connection.ParentId`, so only unconnected nodes qualify.)
pub fn orphans(terrain: &Terrain) -> Vec<i32> {
    let mut found = Vec::new();
    for node in terrain.nodes.values() {
        let feeds_something = terrain.nodes.values().any(|other| takes_input_from(other, node.id));
        let has_connected_input = node.ports.iter().any(|port| port.is_input() && port.record.is_some());
        if feeds_something || has_connected_input {
            continue;
        }
        if crate::build::exports(node) {
            continue;
        }
        found.push(node.id);
    }
    found
}

/// Whether a node's type carries `RequiresBakingAttribute`.
pub fn requires_baking(node: &NodeData) -> bool {
    let catalogue = Catalogue::get();
    catalogue.node(&node.type_name).is_some_and(|spec| spec.has_flag("RequiresBaking"))
}

/// The nodes Bake Required Nodes bakes (`Shell.miBakeGates_Click`): those
/// that need baking, unless they are orphans, in the project's order.
pub fn required_nodes(terrain: &Terrain) -> Vec<i32> {
    let orphans = orphans(terrain);
    terrain.nodes.values().filter(|node| requires_baking(node) && !orphans.contains(&node.id)).map(|node| node.id).collect()
}

/// The pre-build checks' first test for Hybrid and tiled builds: a node that
/// needs baking and is not baked.
pub fn has_unbaked_generators(terrain: &Terrain) -> bool {
    terrain.nodes.values().any(|node| requires_baking(node) && StateOverride::of(node) != StateOverride::Baked)
}

/// Why a bake stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// A node failed to build (with its message).
    Failed(String),
    /// The maps could not be written.
    Io(String),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeError::Failed(message) | BakeError::Io(message) => formatter.write_str(message),
        }
    }
}

/// Bakes `node`. `engine` is a fresh engine set up like
/// the editor's (registry, variables, macros, project path); the nodes
/// upstream that are baked already are loaded into it from `store`, and
/// `built` hands over maps the editor has for a node at the bake
/// resolution (copied instead of built again). Then
/// the node is built at `resolution`; every node of its chain (bypassed
/// ones aside) has its maps written to `store` and is returned, to be
/// marked `Baked`. A failure leaves nothing marked, so no node stays marked
/// without its files.
pub fn bake_node(
    engine: &mut Engine,
    terrain: &Terrain,
    node: i32,
    resolution: Resolution,
    store: &BakeStore,
    built: &dyn Fn(i32) -> Option<Vec<(String, Arc<Map>)>>,
) -> Result<Vec<i32>, BakeError> {
    let Some(target) = terrain.nodes.get(&node) else {
        return Ok(Vec::new());
    };
    if target.type_name == "Macro" {
        return Ok(Vec::new());
    }

    let order = Engine::build_order(terrain, node);
    for id in &order {
        let Some(current) = terrain.nodes.get(id) else {
            continue;
        };
        if StateOverride::of(current) == StateOverride::Baked
            && let Some(maps) = store.read_node(current)
        {
            engine.load_baked(*id, maps);
            continue;
        }
        if let Some(maps) = built(*id) {
            for (port, map) in maps {
                engine.cache.store(*id, &port, map);
            }
            engine.mark_built(*id);
        }
    }

    let result = engine.build(terrain, node, resolution);
    if let BuildResult::Failed(error) = result {
        return Err(BakeError::Failed(format!("Bake failed. Ensure all nodes are valid and can build properly.\n\n{error}")));
    }
    let mut baked = Vec::new();
    for id in &order {
        let Some(current) = terrain.nodes.get(id) else {
            continue;
        };
        let override_state = StateOverride::of(current);
        let state = engine.state(*id);
        if override_state == StateOverride::Bypassed {
            continue;
        }
        if !matches!(state, NodeState::Built | NodeState::Baked) {
            let reason = engine.error(*id).map(ToString::to_string).unwrap_or_else(|| format!("{} did not build.", current.name));
            return Err(BakeError::Failed(format!("Bake failed. Ensure all nodes are valid and can build properly.\n\n{reason}")));
        }
        baked.push(*id);
    }
    for id in &baked {
        let maps: Vec<(String, Arc<Map>)> = engine.cache.ports(*id).map(|(port, map)| (port.clone(), map.clone())).collect();
        for (port, map) in maps {
            store.write(*id, &port, &map).map_err(|error| BakeError::Io(format!("Cannot write the baked maps to {}: {error}", store.folder().display())))?;
        }
    }
    Ok(baked)
}

/// How a loaded bake is fitted to what is being built:
/// resized to the resolution (bilinear in the editor, `HighQuality` on the
/// command line), cut out for a bucket, or cut to a region and
/// resized to the resolution (for a Region build, or for the editor's
/// region preview). A Region build whose definition holds no
/// region has none to cut to.
#[derive(Clone, Copy, Debug)]
pub enum BakeFit<'a> {
    Whole { resolution: Resolution, high_quality: bool },
    Bucket { bucket: &'a tiles::Bucket, geometry: &'a tiles::BucketGeometry },
    Region { region: Option<&'a Region>, resolution: Resolution, high_quality: bool },
}

/// A baked map fitted for the build; the map keeps
/// its intent. None where the fitting fails (a region that is not there,
/// or whose window runs off the map), which leaves the node unbaked.
pub fn fit_baked(map: &Map, fit: BakeFit<'_>) -> Option<Map> {
    let info = map.info;
    let mut fitted = match fit {
        BakeFit::Whole { resolution, high_quality } => resize_to(map, resolution, high_quality),
        BakeFit::Bucket { bucket, geometry } => tiles::crop_baked_for_bucket(map, bucket, geometry),
        BakeFit::Region { region, resolution, high_quality } => {
            let cropped = crop_to_region(map, region?)?;
            resize_to(&cropped, resolution, high_quality)
        }
    };
    fitted.info = info;
    Some(fitted)
}

fn resize_to(map: &Map, resolution: Resolution, high_quality: bool) -> Map {
    use nerthus_nodes::transformer::{resize_with, ResizeMethod};
    if map.resolution() == resolution {
        return map.clone();
    }
    let method = if high_quality { ResizeMethod::HighQuality } else { ResizeMethod::LinearFast };
    resize_with(map, resolution, method)
}

/// The region's window of `map`.
/// Each pixel is read at `y * width + x` of the channel, so a window
/// past the right edge reads on into the next row; one whose first or
/// last pixel falls outside the channel throws (None).
pub fn crop_to_region(map: &Map, region: &Region) -> Option<Map> {
    let (left, top, side) = region.window(map.width());
    let width = map.width() as i64;
    let length = width * map.height() as i64;
    let first = top * width + left;
    let last = (top + side - 1) * width + left + side - 1;
    if side <= 0 || first < 0 || last >= length {
        return None;
    }
    let side = side as usize;
    let mut channels = Vec::with_capacity(map.channel_count());
    for index in 0..map.channel_count() {
        let source = map.channel(index);
        let mut cropped = Vec::with_capacity(side * side);
        for row in 0..side {
            let start = (first + row as i64 * width) as usize;
            cropped.extend_from_slice(&source[start..start + side]);
        }
        channels.push(cropped);
    }
    Some(Map::from_channels(Resolution::square(side), channels))
}

/// The baked nodes' maps put in `engine`, fitted with
/// `fit`. Crucial ones (all of them with `force`) are loaded, the others
/// left on disk (`NodeCacheStatus.Offloaded`). A node whose files are
/// missing is left unbaked for this engine and builds as usual. Returns
/// the nodes loaded.
pub fn load_bakes(engine: &mut Engine, terrain: &Terrain, store: &BakeStore, fit: BakeFit<'_>, force: bool, swarm: bool) -> Vec<i32> {
    let mut loaded = Vec::new();
    for node in terrain.nodes.values() {
        if StateOverride::of(node) != StateOverride::Baked {
            continue;
        }
        if !store.has_node(node) {
            continue;
        }
        if !force && !is_crucial(terrain, node, swarm) {
            engine.offload_baked(node.id);
            continue;
        }
        let Some(maps) = store.read_node(node) else {
            continue;
        };
        // A port that does not fit leaves the whole node unbaked.
        let fitted: Option<Vec<(String, Map)>> = maps.into_iter().map(|(port, map)| fit_baked(&map, fit).map(|fitted| (port, fitted))).collect();
        let Some(fitted) = fitted else {
            continue;
        };
        engine.load_baked(node.id, fitted);
        loaded.push(node.id);
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::{ConnectionRecord, RenderIntent};
    use serde_json::Value;

    fn node(type_name: &str, id: i32) -> NodeData {
        let spec = Catalogue::get().node(type_name).expect("the type is in the catalogue");
        NodeData::new(spec, id, 1)
    }

    fn terrain(nodes: Vec<NodeData>) -> Terrain {
        let mut graph = Terrain { id: String::new(), nodes: Default::default(), width: 5000.0, height: 2500.0, extra: serde_json::Map::new() };
        for data in nodes {
            graph.nodes.insert(data.id, data);
        }
        graph
    }

    fn link(terrain: &mut Terrain, from: i32, to: i32, port: &str) {
        let record = ConnectionRecord { from, to, from_port: "Out".to_string(), to_port: port.to_string(), is_valid: true };
        let target = terrain.nodes.get_mut(&to).expect("the node exists");
        target.port_mut(port).expect("the port exists").record = Some(record);
    }

    fn baked(terrain: &mut Terrain, id: i32) {
        terrain.nodes.get_mut(&id).expect("the node exists").set_member("StateOverride", Value::from("Baked"));
    }

    #[test]
    fn only_unconnected_nodes_are_orphans() {
        // Perlin → Thermal; a lone Mountain; a Height fed by nothing but
        // feeding nothing either.
        let mut graph = terrain(vec![node("Perlin", 1), node("Thermal", 2), node("Mountain", 3), node("Height", 4)]);
        link(&mut graph, 1, 2, "In");
        assert_eq!(orphans(&graph), vec![3, 4]);
        // Thermal reads Perlin, so Perlin is not an orphan; Thermal has a
        // connected input, which the test also counts.
        assert_eq!(required_nodes(&graph), vec![1]);
        assert!(has_unbaked_generators(&graph));
    }

    #[test]
    fn crucial_bakes_are_the_ones_unbaked_nodes_read() {
        let mut graph = terrain(vec![node("Perlin", 1), node("Thermal", 2), node("Height", 3)]);
        link(&mut graph, 1, 2, "In");
        link(&mut graph, 2, 3, "In");
        baked(&mut graph, 1);
        baked(&mut graph, 2);
        let perlin = graph.nodes[&1].clone();
        let thermal = graph.nodes[&2].clone();
        // Only Thermal is read by an unbaked node (Height).
        assert!(!is_crucial(&graph, &perlin, false));
        assert!(is_crucial(&graph, &thermal, false));
    }

    #[test]
    fn stored_maps_read_back_with_their_intent() {
        let folder = std::env::temp_dir().join(format!("nerthus-bakes-{}", std::process::id()));
        let project = folder.join("Canyon.terrain");
        let store = BakeStore::new(&project, "1234abcd", None);
        assert_eq!(store.file(7, "Out"), folder.join("Cache").join("Bakes").join("Canyon_1234abcd").join("7_Out.bin"));

        let height = Map::from_data(Resolution::square(2), vec![0.0, 0.25, 0.5, 1.0]);
        store.write(7, "Out", &height).unwrap();
        let read = store.read(7, "Out").unwrap().unwrap();
        assert_eq!(read, height);
        let meta = fs::read_to_string(store.file(7, "Out").with_extension("meta")).unwrap();
        let keys: Vec<&str> = meta.lines().filter_map(|line| line.trim().split('"').nth(1)).collect();
        assert_eq!(keys, vec!["$id", "Channels", "Resolution", "RenderIntent", "Min", "Max", "GUID"]);

        let mut colour = Map::from_channels(Resolution::square(2), vec![vec![0.1; 4], vec![0.2; 4], vec![0.3; 4]]);
        colour.info.render_intent = RenderIntent::Color;
        store.write(7, "Colour", &colour).unwrap();
        assert_eq!(store.read(7, "Colour").unwrap().unwrap(), colour);
        fs::remove_dir_all(&folder).unwrap();
    }
}
