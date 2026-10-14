//! The LightWave scene importer (`LWSImporter`): a scene file (`.lws`, or
//! a motion file `.mot`) places LightWave objects, null objects, lights
//! and cameras in a hierarchy, each animated by envelopes. The objects are
//! imports of their own, and the scene's meshes are theirs; a scene left
//! without meshes gets a skeleton mesh of its node graph when it is
//! animated.

mod animation;

use animation::{Envelope, KEY_SIZE, Key, Resolver};

use crate::batch::{BatchLoader, Graph, LoadedScene, SceneGraph, merged_animations, merged_graph, merged_meshes};
use crate::fast_atof::{fast_atoreal_move, strtoul10, strtoul16};
use crate::io::FileSystem;
use crate::matrix::Matrix4;
use crate::node::Node;
use crate::parsing::{at, is_line_end, is_space_or_newline, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_at, text_file_to_buffer};
use crate::registry::{FileInput, ImportProperties};
use crate::scene::{ImportError, check_allocation, fail};
use crate::skeleton;

/// `LWS::Element::Parse`'s depth limit.
const MAX_ELEMENT_DEPTH: i32 = 1000;

/// Node hierarchies deeper than this stand for the native recursion that
/// would overflow the stack.
const MAX_GRAPH_DEPTH: usize = 1024;

/// How often the element loop may start over before it counts as never
/// ending (see `read_elements`).
const MAX_PASSES: usize = 3;

/// `AI_LWS_MASK`.
const NUMBER_MASK: u32 = 0xffff_ffff >> 4;

/// `LWS::NodeDesc` types.
const OBJECT: u32 = 1;
const LIGHT: u32 = 2;
const CAMERA: u32 = 3;

/// An element of the file (`LWS::Element`): a line's first word, the rest
/// of the line, and the lines of the `{` block it opens.
#[derive(Default)]
struct Element {
    tokens: [Vec<u8>; 2],
    children: Vec<Element>,
}

impl Element {
    /// `Parse`: lines up to one that starts with `}`, or the end of the
    /// text. A `Plugin` line's contents are skipped up to an `EndPlugin`
    /// line.
    fn parse(&mut self, buffer: &[u8], cursor: &mut usize, depth: i32) -> Result<(), ImportError> {
        if depth > MAX_ELEMENT_DEPTH {
            return fail("Maximum recursion depth exceeded in LWS::Element::Parse");
        }
        let end = buffer.len();
        loop {
            let (position, more) = skip_spaces_and_line_end(buffer, *cursor, end);
            *cursor = position;
            if !more {
                return Ok(());
            }
            let mut opens_block = false;
            if at(buffer, *cursor) == b'{' {
                *cursor = skip_spaces(buffer, *cursor + 1, end).0;
                opens_block = true;
            } else if at(buffer, *cursor) == b'}' {
                return Ok(());
            }

            let mut child = Element::default();
            let start = *cursor;
            while !is_space_or_newline(at(buffer, *cursor)) {
                *cursor += 1;
            }
            child.tokens[0] = buffer[start..*cursor].to_vec();
            *cursor = skip_spaces(buffer, *cursor, end).0;

            if child.tokens[0] == b"Plugin" {
                self.children.push(child);
                loop {
                    let (position, more) = skip_spaces_and_line_end(buffer, *cursor, end);
                    *cursor = position;
                    if !more || starts_with_at(buffer, *cursor, b"EndPlugin") {
                        break;
                    }
                    *cursor = skip_line(buffer, *cursor, end).0;
                }
            } else {
                let start = *cursor;
                while !is_line_end(at(buffer, *cursor)) {
                    *cursor += 1;
                }
                child.tokens[1] = buffer[start..*cursor].to_vec();
                if opens_block {
                    child.parse(buffer, cursor, depth + 1)?;
                }
                self.children.push(child);
            }
            *cursor = skip_line(buffer, *cursor, end).0;
        }
    }
}

/// A scene node (`LWS::NodeDesc`), as far as the meshes and the import's
/// failures depend on it.
struct NodeDesc {
    kind: u32,
    /// The object file, for an object loaded from one.
    path: Vec<u8>,
    /// Its load request.
    id: u32,
    number: u32,
    /// The parent's number with its type in the top 4 bits; 0 for none.
    parent: u32,
    channels: Vec<Envelope>,
    pivot: [f32; 3],
    pivot_set: bool,
    children: Vec<usize>,
    parent_resolved: Option<usize>,
}

impl NodeDesc {
    fn new(kind: u32, number: u32) -> Self {
        NodeDesc {
            kind,
            path: Vec::new(),
            id: 0,
            number,
            parent: 0,
            channels: Vec::new(),
            pivot: [0.0; 3],
            pivot_set: false,
            children: Vec::new(),
            parent_resolved: None,
        }
    }

    /// `operator==(unsigned int)`: whether a parent number names this
    /// node.
    fn is_named_by(&self, number: u32) -> bool {
        number != 0 && number >> 28 == self.kind && number & NUMBER_MASK == self.number
    }
}

/// What the file's elements set up.
struct Description {
    nodes: Vec<NodeDesc>,
    first: f64,
    last: f64,
    fps: f64,
    /// The cameras and lights added, which size the scene's lists.
    cameras: u32,
    lights: u32,
}

/// Three floats with blanks between them (`fast_atoreal_move` each).
fn read_floats(line: &[u8], mut cursor: usize) -> Result<[f32; 3], ImportError> {
    let end = line.len();
    let mut values = [0.0f32; 3];
    for (index, slot) in values.iter_mut().enumerate() {
        if index > 0 {
            cursor = skip_spaces(line, cursor, end).0;
        }
        let (value, after) = fast_atoreal_move(line, cursor, true)?;
        *slot = value;
        cursor = after;
    }
    Ok(values)
}

/// `ReadEnvelope`: an `Envelope` block's keys (a value, a time, then up
/// to five parameters as the span type wants) and pre behaviour.
fn read_envelope(element: &Element, envelope: &mut Envelope) -> Result<(), ImportError> {
    let Some(count_line) = element.children.first() else {
        return Ok(());
    };
    // The keys are reserved by the rest of the first line, not its count.
    let (reserved, _) = strtoul10(&count_line.tokens[1], 0);
    check_allocation(reserved as u64, KEY_SIZE)?;
    for child in &element.children[1..] {
        let line = child.tokens[1].as_slice();
        let end = line.len();
        if child.tokens[0] == b"Key" {
            let mut cursor = skip_spaces(line, 0, end).0;
            let (value, after) = fast_atoreal_move(line, cursor, true)?;
            cursor = skip_spaces(line, after, end).0;
            let (time, after) = fast_atoreal_move(line, cursor, true)?;
            envelope.keys.push(Key { time: time as f64, value });
            // The span type is read where the time ended, before any blank.
            let (span, after) = strtoul10(line, after);
            cursor = after;
            let parameters = match span {
                0..=2 => 5,
                5 => 4,
                _ => 0,
            };
            for _ in 0..parameters {
                cursor = skip_spaces(line, cursor, end).0;
                cursor = fast_atoreal_move(line, cursor, true)?.1;
            }
        } else if child.tokens[0] == b"Behaviors" {
            let cursor = skip_spaces(line, 0, end).0;
            envelope.pre = strtoul10(line, cursor).0;
        }
    }
    Ok(())
}

/// `ReadEnvelope_Old`: from the element after `index`, a channel count,
/// then per channel a key count and its keys, one element each whose first
/// word is the key's value and, by a slip, its time too. The index of the
/// last element read, or `None` when the elements run out.
fn read_old_envelopes(elements: &[Element], mut index: usize, node: &mut NodeDesc) -> Result<Option<usize>, ImportError> {
    index += 1;
    if index == elements.len() {
        return Ok(None);
    }
    let (channel_count, _) = strtoul10(&elements[index].tokens[0], 0);
    for channel in 0..channel_count {
        let mut envelope = Envelope::new(channel.wrapping_add(1));
        index += 1;
        if index == elements.len() {
            node.channels.push(envelope);
            return Ok(None);
        }
        let (key_count, _) = strtoul10(&elements[index].tokens[0], 0);
        for _ in 0..key_count {
            index += 1;
            if index == elements.len() {
                node.channels.push(envelope);
                return Ok(None);
            }
            let (value, _) = fast_atoreal_move(&elements[index].tokens[0], 0, true)?;
            envelope.keys.push(Key { time: value as f64, value });
        }
        node.channels.push(envelope);
    }
    Ok(Some(index))
}

/// `FindLWOFile`: the path as given if it exists (with a separator put
/// after a drive letter), else one or two folders up (a packaged scene's
/// layout), else the path with the separator.
fn find_lwo_file(input: &[u8], files: &FileSystem) -> Vec<u8> {
    let mut path = input.to_vec();
    if input.len() > 3 && input[1] == b':' && input[2] != b'\\' && input[2] != b'/' {
        path = [&input[..1], b":\\".as_slice(), &input[2..]].concat();
    }
    if files.exists_bytes(&path) {
        return input.to_vec();
    }
    let separator = [files.separator() as u8];
    let one_up = [b"..".as_slice(), &separator, &path].concat();
    if files.exists_bytes(&one_up) {
        return one_up;
    }
    let two_up = [b"..".as_slice(), &separator, &one_up].concat();
    if files.exists_bytes(&two_up) {
        return two_up;
    }
    path
}

/// The element loop of `InternReadFile`, from the version element on.
///
/// An old-style motion that runs out of elements leaves the loop's
/// iterator at the end, which the loop then steps past: round to the
/// first element again. Every pass after the first does the same as the
/// one before it, ending at the same place, so the loop never ends unless
/// a pass throws.
fn read_elements(root: &Element, version: u32, motion_file: bool, batch: &mut BatchLoader) -> Result<Description, ImportError> {
    let elements = &root.children;
    let mut description = Description { nodes: Vec::new(), first: 0.0, last: 60.0, fps: 25.0, cameras: 0, lights: 0 };
    let nodes = &mut description.nodes;
    let mut light_count = 0u32;
    let mut camera_count = 0u32;
    let mut object_count = 0u32;
    let mut passes = 1usize;
    let mut index = 1usize;
    while index < elements.len() {
        let element = &elements[index];
        let line = element.tokens[1].as_slice();
        let end = line.len();
        let last_kind = nodes.last().map(|node| node.kind);
        match element.tokens[0].as_slice() {
            b"FirstFrame" => {
                if description.first != 150392.0 {
                    description.first = strtoul10(line, 0).0 as f64 - 1.0;
                }
            }
            b"LastFrame" => {
                if description.last != 150392.0 {
                    description.last = strtoul10(line, 0).0 as f64 - 1.0;
                }
            }
            b"FramesPerSecond" => description.fps = strtoul10(line, 0).0 as f64,
            b"LoadObjectLayer" => {
                let (layer, mut cursor) = strtoul10(line, 0);
                let properties = ImportProperties { lwo_layer: Some(layer as i32), ..ImportProperties::default() };
                let number = if version >= 4 {
                    cursor = skip_spaces(line, cursor, end).0;
                    let (number, after) = strtoul16(line, cursor);
                    cursor = after;
                    number & NUMBER_MASK
                } else {
                    object_count = object_count.wrapping_add(1);
                    object_count.wrapping_sub(1)
                };
                cursor = skip_spaces(line, cursor, end).0;
                let path = find_lwo_file(&line[cursor..], batch.files());
                let mut node = NodeDesc::new(OBJECT, number);
                node.id = batch.add_load_request(&path, &properties);
                node.path = path;
                nodes.push(node);
            }
            b"LoadObject" => {
                let mut cursor = 0;
                let number = if version >= 4 {
                    let (number, after) = strtoul16(line, 0);
                    cursor = skip_spaces(line, after, end).0;
                    number & NUMBER_MASK
                } else {
                    object_count = object_count.wrapping_add(1);
                    object_count.wrapping_sub(1)
                };
                let path = find_lwo_file(&line[cursor..], batch.files());
                let mut node = NodeDesc::new(OBJECT, number);
                node.id = batch.add_load_request(&path, &ImportProperties::default());
                node.path = path;
                nodes.push(node);
            }
            b"AddNullObject" => {
                let number = if version >= 4 {
                    strtoul16(line, 0).0 & NUMBER_MASK
                } else {
                    object_count = object_count.wrapping_add(1);
                    object_count.wrapping_sub(1)
                };
                nodes.push(NodeDesc::new(OBJECT, number));
            }
            b"Channel" => match nodes.last_mut() {
                Some(node) => {
                    let (channel, _) = strtoul10(line, 0);
                    node.channels.push(Envelope::new(channel.wrapping_add(1)));
                }
                None => {
                    if motion_file {
                        object_count = object_count.wrapping_add(1);
                        nodes.push(NodeDesc::new(OBJECT, object_count.wrapping_sub(1)));
                    }
                }
            },
            b"Envelope" => {
                if let Some(envelope) = nodes.last_mut().and_then(|node| node.channels.last_mut()) {
                    read_envelope(element, envelope)?;
                }
            }
            b"ObjectMotion" | b"CameraMotion" | b"LightMotion" if version < 3 => {
                if let Some(node) = nodes.last_mut() {
                    match read_old_envelopes(elements, index, node)? {
                        Some(last_read) => index = last_read,
                        None => {
                            passes += 1;
                            if passes > MAX_PASSES {
                                return fail("endless loop: an old-style motion read past the last element, again and again");
                            }
                            index = 0;
                            continue;
                        }
                    }
                }
            }
            b"Pre/PostBehavior" if version == 2 => {
                if let Some(node) = nodes.last_mut() {
                    let mut cursor = 0;
                    for envelope in &mut node.channels {
                        let (pre, after) = strtoul10(line, cursor);
                        envelope.pre = pre;
                        cursor = skip_spaces(line, after, end).0;
                        let (_post, after) = strtoul10(line, cursor);
                        cursor = skip_spaces(line, after, end).0;
                    }
                }
            }
            b"ParentItem" => {
                if let Some(node) = nodes.last_mut() {
                    node.parent = strtoul16(line, 0).0;
                }
            }
            b"ParentObject" if version < 3 => {
                if let Some(node) = nodes.last_mut() {
                    node.parent = strtoul10(line, 0).0 | (1 << 28);
                }
            }
            b"AddCamera" => {
                let number = if version >= 4 {
                    strtoul16(line, 0).0 & NUMBER_MASK
                } else {
                    camera_count = camera_count.wrapping_add(1);
                    camera_count.wrapping_sub(1)
                };
                nodes.push(NodeDesc::new(CAMERA, number));
                description.cameras = description.cameras.wrapping_add(1);
            }
            b"AddLight" => {
                let number = if version >= 4 {
                    strtoul16(line, 0).0 & NUMBER_MASK
                } else {
                    light_count = light_count.wrapping_add(1);
                    light_count.wrapping_sub(1)
                };
                nodes.push(NodeDesc::new(LIGHT, number));
                description.lights = description.lights.wrapping_add(1);
            }
            // The light settings change nothing here, but their numbers
            // must parse.
            b"LightIntensity" | b"LgtIntensity" if last_kind == Some(LIGHT) => {
                if !line.starts_with(b"(envelope)") {
                    fast_atoreal_move(line, 0, true)?;
                }
            }
            b"LightConeAngle" | b"LightEdgeAngle" if last_kind == Some(LIGHT) => {
                fast_atoreal_move(line, 0, true)?;
            }
            b"LightColor" if last_kind == Some(LIGHT) => {
                read_floats(line, 0)?;
            }
            b"PivotPosition" | b"PivotPoint" => {
                if let Some(node) = nodes.last_mut() {
                    node.pivot = read_floats(line, 0)?;
                    node.pivot_set = true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(description)
}

/// The parents the nodes name, found: each node's children (a node named
/// by two others' parent numbers goes to the first), and the nodes left
/// without a parent, the roots, in order.
fn resolve_parents(nodes: &mut [NodeDesc]) -> Result<Vec<usize>, ImportError> {
    for parent in 0..nodes.len() {
        for child in 0..nodes.len() {
            if child == parent || !nodes[parent].is_named_by(nodes[child].parent) {
                continue;
            }
            if nodes[child].parent_resolved.is_some() {
                continue;
            }
            nodes[parent].children.push(child);
            nodes[child].parent_resolved = Some(parent);
        }
    }
    let roots: Vec<usize> = (0..nodes.len()).filter(|&index| nodes[index].parent_resolved.is_none()).collect();
    if roots.is_empty() {
        return fail("LWS: Unable to find scene root node");
    }
    Ok(roots)
}

/// The pivot node an object file's scene brings (a root with one child):
/// its translation becomes the object's pivot unless the scene file gave
/// one (z back to the right-handed sign), and its child, translation
/// cleared, becomes the root.
fn strip_pivot(graph: &mut SceneGraph, node: &mut NodeDesc) {
    let root = graph.root;
    if graph.nodes[root].children.len() != 1 {
        return;
    }
    if !node.pivot_set {
        let transform = graph.nodes[root].transform.0;
        node.pivot = [transform[3], transform[7], -transform[11]];
    }
    let new_root = graph.nodes[root].children[0];
    graph.root = new_root;
    graph.nodes[new_root].parent = None;
    let transform = &mut graph.nodes[new_root].transform.0;
    transform[3] = 0.0;
    transform[7] = 0.0;
    transform[11] = 0.0;
}

/// `BuildGraph`'s output: the master scene's node graph and the objects'
/// scenes attached to it.
struct GraphBuilder<'a, 'b> {
    graph: SceneGraph,
    /// The master scene's animations: one when any channel is made.
    animations: usize,
    attachments: Vec<(usize, usize)>,
    batch: &'a mut BatchLoader<'b>,
    first: f64,
    last: f64,
    fps: f64,
    /// The cameras and lights reached, which fill the scene's lists.
    cameras: u32,
    lights: u32,
}

impl GraphBuilder<'_, '_> {
    fn add_node(&mut self, parent: usize, transform: Matrix4) -> usize {
        let index = self.graph.nodes.len();
        self.graph.nodes.push(Node::new(Vec::new(), transform, Some(parent)));
        self.graph.nodes[parent].children.push(index);
        index
    }

    /// `BuildGraph` for one scene node at `node`: an object gets a child
    /// moved back by its pivot, which its file's scene is attached to and
    /// its children go under; the node's transform is its bind pose, and
    /// its animation channel is made when the scene's frame range is not
    /// empty.
    fn build(&mut self, descriptions: &mut [NodeDesc], index: usize, node: usize, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_GRAPH_DEPTH {
            return fail("undefined behaviour: a node hierarchy too deep for the stack");
        }
        let mut parent = node;
        let description = &mut descriptions[index];
        match description.kind {
            CAMERA => self.cameras += 1,
            LIGHT => self.lights += 1,
            _ => {}
        }
        if description.kind == OBJECT {
            let mut scene = None;
            if !description.path.is_empty() {
                scene = self.batch.get_import(description.id);
                if let Some(loaded) = scene {
                    match &mut self.batch.scenes[loaded].graph {
                        Graph::Missing => return fail("undefined behaviour: an object's scene without a root node"),
                        Graph::Unknown => {}
                        Graph::Known(attached) => strip_pivot(attached, description),
                    }
                }
            }
            let pivot = description.pivot;
            let attachment = self.add_node(node, Matrix4::translation([-pivot[0], -pivot[1], -pivot[2]]));
            parent = attachment;
            if let Some(loaded) = scene {
                self.attachments.push((loaded, attachment));
            }
        }

        let mut resolver = Resolver::new(&mut description.channels, self.fps);
        self.graph.nodes[node].transform = resolver.bind_pose();
        if self.first != self.last && resolver.extract_channel(self.first)? {
            self.animations = 1;
        }

        let children = description.children.clone();
        for child in children {
            let child_node = self.add_node(parent, Matrix4::IDENTITY);
            self.build(descriptions, child, child_node, depth + 1)?;
        }
        Ok(())
    }
}

/// `MakeLeftHandedProcess` on the master scene's nodes: every transform's
/// third row and column negated (the z scale twice, so kept).
fn make_left_handed(graph: &mut SceneGraph) {
    for node in &mut graph.nodes {
        let transform = &mut node.transform.0;
        for index in [8, 9, 11, 2, 6, 14] {
            transform[index] = -transform[index];
        }
    }
}

/// `InternReadFile`, keeping the node graph.
pub(crate) fn read_graph(input: &FileInput) -> Result<LoadedScene, ImportError> {
    let buffer = text_file_to_buffer(input.bytes, false)?;
    let mut root = Element::default();
    let mut cursor = 0;
    root.parse(&buffer, &mut cursor, 0)?;

    let Some(magic) = root.children.first() else {
        return fail("undefined behaviour: the first element of a file without elements");
    };
    let motion_file = magic.tokens[0] == b"LWMO";
    if magic.tokens[0] != b"LWSC" && !motion_file {
        return fail("LWS: Not a LightWave scene, magic tag LWSC not found");
    }
    let Some(version_element) = root.children.get(1).filter(|element| !element.tokens[0].is_empty()) else {
        // "Invalid LWS file": the import ends with an empty scene, not even
        // a root node.
        return Ok(LoadedScene { meshes: Vec::new(), graph: Graph::Missing, animations: Some(0) });
    };
    let version = strtoul10(&version_element.tokens[0], 0).0;

    let mut batch = BatchLoader::new(&input.files);
    let mut description = read_elements(&root, version, motion_file, &mut batch)?;
    let roots = resolve_parents(&mut description.nodes)?;
    batch.load_all()?;

    let master_root = Node::new(b"<LWSRoot>".to_vec(), Matrix4::IDENTITY, None);
    let master = SceneGraph { nodes: vec![master_root], root: 0 };
    let mut builder = GraphBuilder {
        graph: master,
        animations: 0,
        attachments: Vec::new(),
        batch: &mut batch,
        first: description.first,
        last: description.last,
        fps: description.fps,
        cameras: 0,
        lights: 0,
    };
    for scene_root in roots {
        let node = builder.add_node(0, Matrix4::IDENTITY);
        builder.build(&mut description.nodes, scene_root, node, 0)?;
    }
    // Cameras and lights only in parent cycles are never reached: their
    // list entries stay unset, and the conversion and the merge after
    // use them.
    if builder.cameras != description.cameras || builder.lights != description.lights {
        return fail("undefined behaviour: unset camera or light entries used");
    }
    let GraphBuilder { graph: mut master, animations: master_animations, attachments, .. } = builder;
    make_left_handed(&mut master);
    batch.check_released()?;

    let meshes = merged_meshes(Vec::new(), &mut batch.scenes, &attachments);
    let animations = merged_animations(master_animations, &batch.scenes, &attachments);
    if !meshes.is_empty() {
        return Ok(LoadedScene { meshes, graph: Graph::Unknown, animations });
    }
    // Without meshes, an animated scene gets a skeleton of its graph.
    let graph = merged_graph(&master, &batch.scenes, &attachments)?;
    let Some(animation_count) = animations else {
        return fail("a scene object's animations are not kept by its importer's port");
    };
    if animation_count == 0 {
        let graph = graph.map_or(Graph::Unknown, Graph::Known);
        return Ok(LoadedScene { meshes, graph, animations });
    }
    let Some(graph) = graph else {
        return fail("a scene object's node graph is not kept by its importer's port");
    };
    let skeleton = skeleton::build_mesh(&graph.nodes, graph.root, false);
    Ok(LoadedScene { meshes: vec![skeleton], graph: Graph::Known(graph), animations })
}
