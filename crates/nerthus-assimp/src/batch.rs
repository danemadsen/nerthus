//! The files a scene places as its objects: `BatchLoader`, whose load
//! requests are imports of their own through the scene's filter, and what
//! `SceneCombiner::MergeScenes` makes of the loaded scenes.

use std::cell::RefCell;

use crate::io::FileSystem;
use crate::node::Node;
use crate::postprocess::preprocess_meshes;
use crate::registry::{self, FileInput, ImportProperties};
use crate::scene::{ImportError, Mesh, fail};

/// Scenes that load scenes deeper than this stand for an import that
/// never ends: each nested filter retries a failed lookup through all the
/// filters it wraps, so lookups cost about twice as much per level.
const MAX_DEPTH: usize = 20;

/// `aiComponent_BONEWEIGHTS` and `aiComponent_ANIMATIONS`, the components
/// a load request may remove (`RemoveVCProcess`).
const COMPONENT_BONE_WEIGHTS: i32 = 0x20;
const COMPONENT_ANIMATIONS: i32 = 0x40;

/// A node graph: nodes in one vector, linked by index.
#[derive(Clone)]
pub(crate) struct SceneGraph {
    pub nodes: Vec<Node>,
    pub root: usize,
}

impl SceneGraph {
    /// Copies `other`'s graph below `parent`, as its last child.
    pub fn append(&mut self, parent: usize, other: &SceneGraph) {
        let offset = self.nodes.len();
        for node in &other.nodes {
            let mut copy = node.clone();
            copy.parent = copy.parent.map(|index| index + offset);
            copy.children = copy.children.iter().map(|&child| child + offset).collect();
            self.nodes.push(copy);
        }
        let root = other.root + offset;
        self.nodes[root].parent = Some(parent);
        self.nodes[parent].children.push(root);
    }
}

/// What the port knows of an imported scene's node graph.
#[derive(Clone)]
pub(crate) enum Graph {
    /// `mRootNode` is null.
    Missing,
    /// A root node, in a graph the importer's port does not keep.
    Unknown,
    Known(SceneGraph),
}

/// An imported scene: its meshes, what is known of its node graph, and
/// its animation count (`mNumAnimations`) when known.
#[derive(Clone)]
pub(crate) struct LoadedScene {
    pub meshes: Vec<Mesh>,
    pub graph: Graph,
    pub animations: Option<usize>,
}

/// What an importer keeps from one import to the next when a batch's
/// importer reads several files: the Irrlicht scene importer's camera and
/// light lists, which it never clears, so each scene it makes takes all
/// of them.
#[derive(Default)]
pub(crate) struct ImporterState {
    /// The cameras and lights in the lists.
    pub irr_cameras_and_lights: usize,
    /// Whether a scene imported already owns them.
    pub irr_owned: bool,
}

/// One load request's import (`Importer::ReadFile` on the batch's own
/// importer, which reads through the scene's filter): the importer chosen
/// as for any file, then only `ScenePreprocessor` (and the components the
/// request removes). `Ok(None)` when the import fails; an error when the
/// native code would crash or hang, or when the file's format has no port
/// yet.
fn read_nested(
    path: &[u8],
    properties: &ImportProperties,
    outer: &FileSystem,
    state: &RefCell<ImporterState>,
) -> Result<Option<LoadedScene>, ImportError> {
    if outer.depth() >= MAX_DEPTH {
        return fail("endless loop: scenes that load scenes too deep to finish");
    }
    if !outer.exists_bytes(path) {
        return Ok(None);
    }
    let Some((bytes, opened)) = outer.open_resolved(path) else {
        return Ok(None);
    };
    let Ok(path) = std::str::from_utf8(path) else {
        return Ok(None);
    };
    // A file that is already being imported further out loads itself
    // again at every level, ever more slowly.
    let identity = FileSystem::identify(&opened);
    if let Some(identity) = &identity
        && outer.imports(identity)
    {
        return fail("endless loop: a scene that loads itself");
    }
    let mut files = FileSystem::new(path, Some(outer));
    files.set_identity(identity);
    let input = FileInput { path, bytes: &bytes, files, properties, state };
    let importer = match registry::choose(&input) {
        Ok(importer) => importer,
        Err(error) if error.ends_process() => return Err(error),
        Err(_) => return Ok(None),
    };
    let Some(read) = importer.read else {
        return fail(format!("{} files cannot be read yet", importer.name));
    };
    let result = match importer.read_graph {
        Some(read_graph) => read_graph(&input),
        None => read(&input).map(|scene| LoadedScene { meshes: scene.meshes, graph: Graph::Unknown, animations: None }),
    };
    let mut loaded = match result {
        Ok(loaded) => loaded,
        Err(error) if error.ends_process() => return Err(error),
        Err(_) => return Ok(None),
    };

    preprocess_meshes(&mut loaded.meshes);
    if let Some(components) = properties.remove_components {
        if components & COMPONENT_ANIMATIONS != 0 {
            loaded.animations = Some(0);
        }
        if components & COMPONENT_BONE_WEIGHTS != 0 {
            for mesh in &mut loaded.meshes {
                mesh.bones.clear();
            }
        }
    }
    Ok(Some(loaded))
}

/// A load request (`LoadRequest`).
struct Request {
    file: Vec<u8>,
    properties: ImportProperties,
    references: u32,
    /// The loaded scene, an index into the batch's scenes.
    scene: Option<usize>,
    loaded: bool,
    /// Whether its scene has been taken (and so merged into the scene that
    /// asked for it).
    taken: bool,
    id: u32,
}

/// `BatchLoader`: each file loaded once however often it is asked for
/// (paths compared as the file system compares them, with the same
/// properties), its scene shared by those that asked.
pub(crate) struct BatchLoader<'a> {
    files: &'a FileSystem,
    requests: Vec<Request>,
    next_id: u32,
    /// The state of the batch's own importer.
    state: RefCell<ImporterState>,
    /// The loaded scenes.
    pub scenes: Vec<LoadedScene>,
}

impl<'a> BatchLoader<'a> {
    pub fn new(files: &'a FileSystem) -> Self {
        BatchLoader { files, requests: Vec::new(), next_id: 0xffff, state: RefCell::default(), scenes: Vec::new() }
    }

    /// The file system the requests are read through (the scene's filter).
    pub fn files(&self) -> &'a FileSystem {
        self.files
    }

    /// `AddLoadRequest`: the request's id, an earlier one's for the same
    /// file with the same properties.
    pub fn add_load_request(&mut self, file: &[u8], properties: &ImportProperties) -> u32 {
        for request in &mut self.requests {
            if self.files.compare_paths(&request.file, file) && request.properties == *properties {
                request.references += 1;
                return request.id;
            }
        }
        let id = self.next_id;
        let request = Request {
            file: file.to_vec(),
            properties: properties.clone(),
            references: 1,
            scene: None,
            loaded: false,
            taken: false,
            id,
        };
        self.requests.push(request);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// `LoadAll`: every request's import, in order.
    pub fn load_all(&mut self) -> Result<(), ImportError> {
        for index in 0..self.requests.len() {
            let request = &self.requests[index];
            let loaded = read_nested(&request.file, &request.properties, self.files, &self.state)?;
            let scene = loaded.map(|scene| {
                self.scenes.push(scene);
                self.scenes.len() - 1
            });
            self.requests[index].scene = scene;
            self.requests[index].loaded = true;
        }
        Ok(())
    }

    /// `GetImport`: a loaded request's scene; the request is forgotten once
    /// all that asked for it have taken it.
    pub fn get_import(&mut self, id: u32) -> Option<usize> {
        let position = self.requests.iter().position(|request| request.id == id && request.loaded)?;
        let scene = self.requests[position].scene;
        self.requests[position].taken = true;
        self.requests[position].references -= 1;
        if self.requests[position].references == 0 {
            self.requests.remove(position);
        }
        scene
    }

    /// The batch deletes the scenes of the requests still open when it
    /// goes: one taken by fewer objects than asked for it was merged into
    /// the scene, which deleted it already.
    pub fn check_released(&self) -> Result<(), ImportError> {
        let deleted_twice = self.requests.iter().any(|request| request.taken && request.scene.is_some());
        if deleted_twice {
            return fail("undefined behaviour: a loaded scene deleted twice");
        }
        Ok(())
    }
}

/// `MergeScenes`' mesh list: the master scene's meshes, then each attached
/// scene's in the order of the attachments. A scene attached again shares
/// the meshes it brought the first time.
pub(crate) fn merged_meshes(master: Vec<Mesh>, scenes: &mut [LoadedScene], attachments: &[(usize, usize)]) -> Vec<Mesh> {
    let mut meshes = master;
    let mut merged = Vec::new();
    for &(scene, _) in attachments {
        if merged.contains(&scene) {
            continue;
        }
        merged.push(scene);
        meshes.append(&mut scenes[scene].meshes);
    }
    meshes
}

/// `MergeScenes`' animation count: the master scene's and every attached
/// scene's (one attached again counted again), when all are known.
pub(crate) fn merged_animations(master: usize, scenes: &[LoadedScene], attachments: &[(usize, usize)]) -> Option<usize> {
    let mut total = master;
    for &(scene, _) in attachments {
        total += scenes[scene].animations?;
    }
    Some(total)
}

/// `MergeScenes` walks every attached scene's node graph: one without a
/// root node crashes it.
pub(crate) fn check_roots(scenes: &[LoadedScene], attachments: &[(usize, usize)]) -> Result<(), ImportError> {
    for &(scene, _) in attachments {
        if matches!(scenes[scene].graph, Graph::Missing) {
            return fail("undefined behaviour: a scene without a root node attached");
        }
    }
    Ok(())
}

/// `MergeScenes`' node graph (`AttachToGraph`): each attached scene's
/// graph, a copy for a scene attached again, added after the children of
/// its attachment node, several to one node in reverse order. `None` when
/// an attached scene's graph is not known.
pub(crate) fn merged_graph(
    master: &SceneGraph,
    scenes: &[LoadedScene],
    attachments: &[(usize, usize)],
) -> Result<Option<SceneGraph>, ImportError> {
    check_roots(scenes, attachments)?;
    let all_known = attachments.iter().all(|&(scene, _)| matches!(scenes[scene].graph, Graph::Known(_)));
    if !all_known {
        return Ok(None);
    }
    let mut graph = master.clone();
    let mut attached_nodes: Vec<usize> = attachments.iter().map(|&(_, node)| node).collect();
    attached_nodes.sort_unstable();
    attached_nodes.dedup();
    // Depth first, children before the node itself, over the master's
    // nodes only.
    let mut order = Vec::new();
    let mut stack = vec![(master.root, false)];
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            order.push(node);
            continue;
        }
        stack.push((node, true));
        for &child in master.nodes[node].children.iter().rev() {
            stack.push((child, false));
        }
    }
    for node in order {
        if attached_nodes.binary_search(&node).is_err() {
            continue;
        }
        for &(scene, at) in attachments.iter().rev() {
            if at != node {
                continue;
            }
            if let Graph::Known(attached) = &scenes[scene].graph {
                graph.append(node, attached);
            }
        }
    }
    Ok(Some(graph))
}
