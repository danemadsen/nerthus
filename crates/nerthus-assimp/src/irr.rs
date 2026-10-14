//! Irrlicht scenes (`IRRImporter`): an `<irr_scene>` of nodes that load
//! meshes from other files, generate cubes, spheres and skyboxes, or stand
//! for lights, cameras and empty transforms, each with animators. The
//! scene's meshes are the generated ones, in node order, then those of the
//! loaded files.

use crate::batch::{BatchLoader, Graph, LoadedScene, SceneGraph, check_roots, merged_animations, merged_graph, merged_meshes};
use crate::cast::{self, to_unsigned};
use crate::fast_atof::{fast_atoreal_move, strtol10};
use crate::irrmesh::parse_material;
use crate::matrix::Matrix4;
use crate::node::Node;
use crate::parsing::{at, skip_spaces};
use crate::registry::{FileInput, ImportProperties, name_extension};
use crate::scene::{ImportError, Mesh, PRIMITIVE_POLYGON, check_allocation, fail};
use crate::shapes;
use crate::vector::{Vector3, normalize};
use crate::xml::{self, Node as XmlNode};

/// Node nesting deeper than this stands for the native recursion that
/// would overflow the stack.
const MAX_DEPTH: usize = 2048;

/// `AI_CONFIG_IMPORT_IRR_ANIM_FPS`'s default: the animation keys per
/// second.
const FPS: f64 = 100.0;

/// `sizeof(aiQuatKey)` and `sizeof(aiVectorKey)`.
const KEY_SIZE: u64 = 24;

/// `aiComponent_ANIMATIONS | aiComponent_BONEWEIGHTS`, removed from the
/// files static mesh nodes load.
const STATIC_MESH_COMPONENTS: i32 = 0x40 | 0x20;

/// `AI_DEG_TO_RAD`'s factor.
const DEGREES_TO_RADIANS: f32 = 0.0174532925f64 as f32;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// Node types (`IRRImporter::Node::ET`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Light,
    Cube,
    Mesh,
    Skybox,
    Dummy,
    Camera,
    Terrain,
    Sphere,
    AnimatedMesh,
}

/// Animator types (`Animator::AT`) as they are read: "flyStraight" reads as
/// a fly circle.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimatorKind {
    Unknown,
    Rotation,
    FlyCircle,
    FollowSpline,
}

/// An animator, as far as making its channel depends on it.
struct Animator {
    kind: AnimatorKind,
    speed: f32,
    direction: Vector3,
    spline_points: usize,
}

impl Animator {
    fn new() -> Self {
        Animator { kind: AnimatorKind::Unknown, speed: 0.001f64 as f32, direction: [0.0, 1.0, 0.0], spline_points: 0 }
    }
}

/// A scene node (`IRRImporter::Node`), as far as the meshes, the node graph
/// and the import's failures depend on it.
struct SceneNode {
    kind: Kind,
    position: Vector3,
    rotation: Vector3,
    scaling: Vector3,
    sphere_radius: f32,
    poly_count: [u32; 2],
    /// How many `<materials>` the node has (their contents change no mesh).
    materials: usize,
    mesh_path: Vec<u8>,
    request: u32,
    animators: Vec<Animator>,
    children: Vec<SceneNode>,
}

impl SceneNode {
    fn new(kind: Kind) -> Self {
        SceneNode {
            kind,
            position: [0.0; 3],
            rotation: [0.0; 3],
            scaling: [1.0; 3],
            sphere_radius: 1.0,
            poly_count: [100, 100],
            materials: 0,
            mesh_path: Vec::new(),
            request: 0,
            animators: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// The name of a property element and its value attributes in order
/// (both attributes looked up in any case; the last name wins).
fn property(element: &XmlNode) -> (Vec<u8>, Vec<&[u8]>) {
    let mut name = Vec::new();
    let mut values = Vec::new();
    for (key, text) in &element.attributes {
        if key.eq_ignore_ascii_case(b"name") {
            name = text.clone();
        } else if key.eq_ignore_ascii_case(b"value") {
            values.push(text.as_slice());
        }
    }
    (name, values)
}

/// `ReadFloatProperty`: every value attribute parsed, the last kept.
fn read_float_property(element: &XmlNode) -> Result<(Vec<u8>, f32), ImportError> {
    let (name, values) = property(element);
    let mut value = 0.0;
    for text in values {
        value = fast_atoreal_move(text, 0, true)?.0;
    }
    Ok((name, value))
}

/// `ReadVectorProperty`: three numbers, commas between them expected (a
/// missing one only logged).
fn read_vector_property(element: &XmlNode) -> Result<(Vec<u8>, Vector3), ImportError> {
    let (name, values) = property(element);
    let mut vector = [0.0f32; 3];
    for text in values {
        let end = text.len();
        let mut cursor = skip_spaces(text, 0, end).0;
        for (index, component) in vector.iter_mut().enumerate() {
            if index > 0 {
                cursor = skip_spaces(text, cursor, end).0;
                if at(text, cursor) == b',' {
                    cursor = skip_spaces(text, cursor + 1, end).0;
                }
            }
            let (value, after) = fast_atoreal_move(text, cursor, true)?;
            *component = value;
            cursor = after;
        }
    }
    Ok((name, vector))
}

/// `ReadIntProperty`.
fn read_int_property(element: &XmlNode) -> (Vec<u8>, i32) {
    let (name, values) = property(element);
    let value = values.last().map_or(0, |text| strtol10(text, 0).0);
    (name, value)
}

/// `ReadStringProperty`.
fn read_string_property(element: &XmlNode) -> (Vec<u8>, Vec<u8>) {
    let (name, values) = property(element);
    let value = values.last().map_or_else(Vec::new, |text| text.to_vec());
    (name, value)
}

/// `ParseNode` and what it calls: the scene's nodes, their mesh files
/// asked for.
struct SceneReader<'a, 'b> {
    batch: &'a mut BatchLoader<'b>,
    /// Cameras and lights added to the importer's lists.
    cameras_and_lights: &'a mut usize,
}

impl SceneReader<'_, '_> {
    /// `ParseNodeAttributes`.
    fn parse_node_attributes(&mut self, attributes: &XmlNode, node: &mut SceneNode) -> Result<(), ImportError> {
        for attribute in attributes.children.iter().filter(|child| child.is_element()) {
            let element = attribute.name();
            if element.eq_ignore_ascii_case(b"vector3d") {
                let (name, value) = read_vector_property(attribute)?;
                match name.as_slice() {
                    b"Position" => node.position = value,
                    b"Rotation" => node.rotation = value,
                    b"Scale" => node.scaling = value,
                    _ => {}
                }
            } else if element.eq_ignore_ascii_case(b"float") {
                let (name, value) = read_float_property(attribute)?;
                let sizes_shape = (node.kind == Kind::Sphere && name == b"Radius") || (node.kind == Kind::Cube && name == b"Size");
                if sizes_shape {
                    node.sphere_radius = value;
                }
            } else if element.eq_ignore_ascii_case(b"int") {
                if node.kind != Kind::Sphere {
                    continue;
                }
                let (name, value) = read_int_property(attribute);
                match name.as_slice() {
                    b"PolyCountX" => node.poly_count[0] = value as u32,
                    b"PolyCountY" => node.poly_count[1] = value as u32,
                    _ => {}
                }
            } else if element.eq_ignore_ascii_case(b"string") || element.eq_ignore_ascii_case(b"enum") {
                let (name, value) = read_string_property(attribute);
                if value.is_empty() || name == b"Name" {
                    continue;
                }
                if node.kind == Kind::Light && name == b"LightType" {
                    // A light of another type is dropped and the node
                    // becomes an empty one.
                    if !matches!(value.as_slice(), b"Spot" | b"Point" | b"Directional") {
                        *self.cameras_and_lights -= 1;
                        node.kind = Kind::Dummy;
                    }
                    continue;
                }
                // An animated mesh takes any other string as its file.
                let names_mesh = (name == b"Mesh" && node.kind == Kind::Mesh) || node.kind == Kind::AnimatedMesh;
                if !names_mesh || name_extension(&value) == b"irr" {
                    continue;
                }
                let mut properties = ImportProperties::default();
                if node.kind != Kind::AnimatedMesh {
                    properties.remove_components = Some(STATIC_MESH_COMPONENTS);
                }
                node.request = self.batch.add_load_request(&value, &properties);
                node.mesh_path = value;
            }
        }
        Ok(())
    }

    /// `ParseAnimators`: an animator from an `<animators>` element's
    /// `<attributes>`, each property read in order (so a type set later
    /// misses the properties before it).
    fn parse_animators(&mut self, element: &XmlNode, node: &mut SceneNode) -> Result<(), ImportError> {
        let mut animator = Animator::new();
        let Some(attributes) = element.child("attributes") else {
            node.animators.push(animator);
            return Ok(());
        };
        for attribute in &attributes.children {
            let name = attribute.name();
            if name.eq_ignore_ascii_case(b"vector3d") {
                let (property, value) = read_vector_property(attribute)?;
                match animator.kind {
                    AnimatorKind::Rotation if property == b"Rotation" => animator.direction = value,
                    AnimatorKind::FollowSpline => {
                        if property.len() >= 6 && property.starts_with(b"Point") {
                            animator.spline_points += 1;
                        }
                    }
                    AnimatorKind::FlyCircle if property == b"Direction" => {
                        animator.direction = if value == [0.0; 3] { [0.0, 1.0, 0.0] } else { normalize(value) };
                    }
                    _ => {}
                }
            } else if name.eq_ignore_ascii_case(b"float") {
                let (property, value) = read_float_property(attribute)?;
                if property == b"Speed" {
                    animator.speed = value;
                }
            } else if name.eq_ignore_ascii_case(b"string") || name.eq_ignore_ascii_case(b"enum") {
                let (property, value) = read_string_property(attribute);
                if property == b"Type" {
                    animator.kind = match value.as_slice() {
                        b"rotation" => AnimatorKind::Rotation,
                        b"flyCircle" | b"flyStraight" => AnimatorKind::FlyCircle,
                        b"followSpline" => AnimatorKind::FollowSpline,
                        _ => AnimatorKind::Unknown,
                    };
                }
            }
        }
        node.animators.push(animator);
        Ok(())
    }

    /// `ParseNode`: the node's type, its attributes, animators and (but for
    /// empty nodes) materials, then its child nodes.
    fn parse_node(&mut self, element: &XmlNode, depth: usize) -> Result<SceneNode, ImportError> {
        if depth > MAX_DEPTH {
            return undefined("a node hierarchy too deep for the stack");
        }
        let kind_name = element.attribute("type").unwrap_or_default();
        let is = |name: &[u8]| kind_name.eq_ignore_ascii_case(name);
        let kind = if is(b"mesh") || is(b"octTree") {
            Kind::Mesh
        } else if is(b"cube") {
            Kind::Cube
        } else if is(b"skybox") {
            Kind::Skybox
        } else if is(b"camera") {
            Kind::Camera
        } else if is(b"light") {
            Kind::Light
        } else if is(b"sphere") {
            Kind::Sphere
        } else if is(b"animatedMesh") {
            Kind::AnimatedMesh
        } else if is(b"terrain") {
            Kind::Terrain
        } else {
            Kind::Dummy
        };
        if matches!(kind, Kind::Camera | Kind::Light) {
            *self.cameras_and_lights += 1;
        }

        let mut node = SceneNode::new(kind);
        for child in &element.children {
            let name = child.name();
            if name.eq_ignore_ascii_case(b"attributes") {
                self.parse_node_attributes(child, &mut node)?;
            } else if name.eq_ignore_ascii_case(b"animators") {
                self.parse_animators(child, &mut node)?;
            }
            if node.kind != Kind::Dummy && name.eq_ignore_ascii_case(b"materials") {
                parse_material(child)?;
                node.materials += 1;
            }
        }
        for child in &element.children {
            if child.name().eq_ignore_ascii_case(b"node") {
                let child_node = self.parse_node(child, depth + 1)?;
                node.children.push(child_node);
            }
        }
        Ok(node)
    }
}

/// The divide error x86 raises for `INT_MIN / -1` and `INT_MIN % -1`.
fn division_trap() -> ImportError {
    ImportError("undefined behaviour: an integer division that overflows".to_string())
}

/// `Math::gcd` on ints.
fn greatest_common_divisor(mut first: i32, mut second: i32) -> Result<i32, ImportError> {
    loop {
        if first == 0 {
            return Ok(second);
        }
        second = second.checked_rem(first).ok_or_else(division_trap)?;
        if second == 0 {
            return Ok(first);
        }
        first = first.checked_rem(second).ok_or_else(division_trap)?;
    }
}

/// `Math::lcm` on ints (the product wrapping).
fn least_common_multiple(first: i32, second: i32) -> Result<i32, ImportError> {
    let divisor = greatest_common_divisor(first, second)?;
    if divisor == 0 {
        return Ok(0);
    }
    let quotient = first.checked_div(divisor).ok_or_else(division_trap)?;
    Ok(quotient.wrapping_mul(second))
}

/// `FindSuitableMultiple`.
fn suitable_multiple(angle: i32) -> i32 {
    match angle {
        ..3 => 3,
        3..10 => 10,
        10..20 => 20,
        20..30 => 30,
        _ => angle,
    }
}

/// The rotation animator's key count: the longest time any of its angles
/// (in hundredths of a degree a tick, modulo a turn) takes for a whole
/// number of turns, at the key rate. `None` when that is one turn of 360.
fn rotation_keys(direction: Vector3) -> Result<Option<u32>, ImportError> {
    let mut angles = direction.map(|component| cast::to_int((component * 100.0) as f64) % 360);
    if angles[0] * angles[1] != 0 && angles[1] * angles[2] != 0 {
        angles = angles.map(suitable_multiple);
    }
    let mut multiple = 360;
    for angle in angles {
        if angle != 0 {
            multiple = least_common_multiple(multiple, angle)?;
        }
    }
    if multiple == 360 {
        return Ok(None);
    }
    let larger = |first: f32, second: f32| if first < second { second } else { first };
    let mut longest = 0.0f32;
    for (index, &angle) in angles.iter().enumerate() {
        if angle == 0 {
            continue;
        }
        let turns = multiple as f32 / angle as f32;
        longest = if index == 0 { turns } else { larger(longest, turns) };
    }
    Ok(Some(to_unsigned(longest as f64 * FPS)))
}

/// `BuildSingleQuadMesh`: one quad of four (position, normal, uv) corners.
fn quad_mesh(corners: [(Vector3, Vector3, [f32; 2]); 4]) -> Mesh {
    let mut mesh = Mesh {
        primitive_types: PRIMITIVE_POLYGON,
        vertex_count: 4,
        vertices: Some(corners.iter().map(|corner| corner.0).collect()),
        normals: Some(corners.iter().map(|corner| corner.1).collect()),
        faces: Some(vec![vec![0, 1, 2, 3]]),
        ..Mesh::default()
    };
    mesh.texture_coords[0] = Some(corners.iter().map(|corner| [corner.2[0], corner.2[1], 0.0]).collect());
    mesh
}

/// `BuildSkybox`: six quads 20 units wide around the origin, facing in.
fn skybox() -> Vec<Mesh> {
    let l = 10.0f32;
    let sides = [
        [([-l, -l, -l], [0.0, 0.0, 1.0], [1.0, 1.0]), ([l, -l, -l], [0.0, 0.0, 1.0], [0.0, 1.0]), ([l, l, -l], [0.0, 0.0, 1.0], [0.0, 0.0]), ([-l, l, -l], [0.0, 0.0, 1.0], [1.0, 0.0])],
        [([l, -l, -l], [-1.0, 0.0, 0.0], [1.0, 1.0]), ([l, -l, l], [-1.0, 0.0, 0.0], [0.0, 1.0]), ([l, l, l], [-1.0, 0.0, 0.0], [0.0, 0.0]), ([l, l, -l], [-1.0, 0.0, 0.0], [1.0, 0.0])],
        [([l, -l, l], [0.0, 0.0, -1.0], [1.0, 1.0]), ([-l, -l, l], [0.0, 0.0, -1.0], [0.0, 1.0]), ([-l, l, l], [0.0, 0.0, -1.0], [0.0, 0.0]), ([l, l, l], [0.0, 0.0, -1.0], [1.0, 0.0])],
        [([-l, -l, l], [1.0, 0.0, 0.0], [1.0, 1.0]), ([-l, -l, -l], [1.0, 0.0, 0.0], [0.0, 1.0]), ([-l, l, -l], [1.0, 0.0, 0.0], [0.0, 0.0]), ([-l, l, l], [1.0, 0.0, 0.0], [1.0, 0.0])],
        [([l, l, -l], [0.0, -1.0, 0.0], [1.0, 1.0]), ([l, l, l], [0.0, -1.0, 0.0], [0.0, 1.0]), ([-l, l, l], [0.0, -1.0, 0.0], [0.0, 0.0]), ([-l, l, -l], [0.0, -1.0, 0.0], [1.0, 0.0])],
        [([l, -l, l], [0.0, 1.0, 0.0], [0.0, 0.0]), ([l, -l, -l], [0.0, 1.0, 0.0], [1.0, 0.0]), ([-l, -l, -l], [0.0, 1.0, 0.0], [1.0, 1.0]), ([-l, -l, l], [0.0, 1.0, 0.0], [0.0, 1.0])],
    ];
    sides.into_iter().map(quad_mesh).collect()
}

/// A vector times a float, each component.
fn scale(vector: Vector3, factor: f32) -> Vector3 {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

/// A node's transform: the rotation (degrees, `FromEulerAnglesXYZ`), its
/// columns scaled, then the translation.
fn node_transform(node: &SceneNode) -> Matrix4 {
    let radians = scale(node.rotation, DEGREES_TO_RADIANS);
    let mut transform = Matrix4::IDENTITY;
    transform.set_euler_angles_xyz(radians[0], radians[1], radians[2]);
    let m = &mut transform.0;
    for (column, &factor) in node.scaling.iter().enumerate() {
        for row in 0..3 {
            m[row * 4 + column] *= factor;
        }
    }
    for (row, &offset) in node.position.iter().enumerate() {
        m[row * 4 + 3] += offset;
    }
    transform
}

/// `GenerateGraph`'s output: the generated meshes, the node graph, the
/// files' scenes attached to it, and the counts the import depends on.
struct Generator<'a, 'b> {
    graph: SceneGraph,
    /// The master scene's animations: one when any channel is made.
    animations: usize,
    meshes: Vec<Mesh>,
    attachments: Vec<(usize, usize)>,
    /// The scene's materials so far.
    materials: usize,
    batch: &'a mut BatchLoader<'b>,
}

impl Generator<'_, '_> {
    fn add_node(&mut self, parent: usize) -> usize {
        let index = self.graph.nodes.len();
        self.graph.nodes.push(Node::new(Vec::new(), Matrix4::IDENTITY, Some(parent)));
        self.graph.nodes[parent].children.push(index);
        index
    }

    /// `CopyMaterial` (the node's first material, if any) and then
    /// `SetupMapping` on the scene's last material, which needs there to
    /// be one.
    fn copy_material(&mut self, node: &SceneNode) -> Result<(), ImportError> {
        if node.materials > 0 {
            self.materials += 1;
        }
        if self.materials == 0 {
            return undefined("the last material of an empty list");
        }
        Ok(())
    }

    /// An identity node between `node` and its parent, in its place among
    /// the parent's children.
    fn insert_dummy(&mut self, node: usize) -> Result<(), ImportError> {
        let Some(parent) = self.graph.nodes[node].parent else {
            return undefined("a node put above the root");
        };
        let dummy = self.graph.nodes.len();
        let mut dummy_node = Node::new(Vec::new(), Matrix4::IDENTITY, Some(parent));
        dummy_node.children.push(node);
        self.graph.nodes.push(dummy_node);
        for child in &mut self.graph.nodes[parent].children {
            if *child == node {
                *child = dummy;
            }
        }
        self.graph.nodes[node].parent = Some(dummy);
        Ok(())
    }

    /// `ComputeAnimations`: a channel per animator of a known type, its
    /// keys allocated; all but the last get a dummy node above the node.
    /// (The count it compares with grows by each channel made, so with two
    /// or more every channel gets one.)
    fn compute_animations(&mut self, node: &SceneNode, out: usize) -> Result<(), ImportError> {
        let animators: Vec<&Animator> = node.animators.iter().filter(|animator| animator.kind != AnimatorKind::Unknown).collect();
        if animators.is_empty() {
            return Ok(());
        }
        let mut total = animators.len();
        for (current, animator) in animators.iter().enumerate() {
            if current != total - 1 {
                self.insert_dummy(out)?;
            }
            let made = match animator.kind {
                AnimatorKind::Rotation => {
                    if let Some(keys) = rotation_keys(animator.direction)? {
                        check_allocation(keys as u64, KEY_SIZE)?;
                    }
                    true
                }
                AnimatorKind::FlyCircle => {
                    let seconds = (1.0 / animator.speed as f64) / 1000.0;
                    check_allocation(to_unsigned(FPS * seconds) as u64, KEY_SIZE)?;
                    true
                }
                AnimatorKind::FollowSpline => animator.spline_points > 0,
                AnimatorKind::Unknown => false,
            };
            if made {
                self.animations = 1;
                total += 1;
            }
        }
        Ok(())
    }

    /// `GenerateGraph` for one node at `out`: its meshes (a loaded file's
    /// scene attached, or a generated shape), its transform, its
    /// animations, then its children.
    fn generate(&mut self, node: &mut SceneNode, out: usize) -> Result<(), ImportError> {
        match node.kind {
            Kind::Mesh | Kind::AnimatedMesh => {
                if !node.mesh_path.is_empty()
                    && let Some(scene) = self.batch.get_import(node.request)
                {
                    self.attachments.push((scene, out));
                }
            }
            Kind::Sphere => {
                let product = node.poly_count[0].wrapping_mul(node.poly_count[1]);
                let tessellation = match product {
                    ..100 => 2,
                    100..300 => 3,
                    _ => 4,
                };
                let mut positions = Vec::new();
                shapes::sphere(tessellation, &mut positions)?;
                if let Some(mesh) = shapes::make_mesh(&positions, 3)? {
                    self.meshes.push(mesh);
                }
                node.scaling = scale(node.scaling, node.sphere_radius / 2.0);
                self.copy_material(node)?;
            }
            Kind::Cube => {
                let mut positions = Vec::new();
                let corners = shapes::hexahedron(&mut positions, true);
                if let Some(mesh) = shapes::make_mesh(&positions, corners)? {
                    self.meshes.push(mesh);
                }
                node.scaling = scale(node.scaling, node.sphere_radius);
                self.copy_material(node)?;
            }
            Kind::Skybox => {
                if node.materials >= 6 {
                    self.materials += 6;
                    self.meshes.extend(skybox());
                }
            }
            Kind::Light | Kind::Camera | Kind::Terrain | Kind::Dummy => {}
        }

        self.graph.nodes[out].transform = node_transform(node);
        self.compute_animations(node, out)?;
        for child in &mut node.children {
            let child_out = self.add_node(out);
            self.generate(child, child_out)?;
        }
        Ok(())
    }
}

/// `InternReadFile` but for the importer state.
fn read_scene(input: &FileInput, cameras_and_lights: &mut usize) -> Result<LoadedScene, ImportError> {
    let Ok(document) = xml::parse(input.bytes) else {
        return fail(format!("XML parse error while loading IRR file {}", input.path));
    };
    // (A missing scene element is thrown as a pointer, which only
    // `ReadFile`'s catch-all catches: the import fails all the same.)
    let Some(scene_element) = document.child("irr_scene") else {
        return fail("IRR: <irr_scene> not found in file");
    };

    let mut batch = BatchLoader::new(&input.files);
    let mut root = SceneNode::new(Kind::Dummy);
    let mut reader = SceneReader { batch: &mut batch, cameras_and_lights };
    for child in &scene_element.children {
        if child.name().eq_ignore_ascii_case(b"node") {
            let node = reader.parse_node(child, 0)?;
            root.children.push(node);
        }
    }
    batch.load_all()?;

    let master_root = Node::new(b"<IRRRoot>".to_vec(), Matrix4::IDENTITY, None);
    let master = SceneGraph { nodes: vec![master_root], root: 0 };
    let mut generator =
        Generator { graph: master, animations: 0, meshes: Vec::new(), attachments: Vec::new(), materials: 0, batch: &mut batch };
    generator.generate(&mut root, 0)?;
    let Generator { graph: master, animations: master_animations, meshes: generated, attachments, .. } = generator;

    check_roots(&batch.scenes, &attachments)?;
    batch.check_released()?;
    let meshes = merged_meshes(generated, &mut batch.scenes, &attachments);
    let animations = merged_animations(master_animations, &batch.scenes, &attachments);
    if !meshes.is_empty() {
        return Ok(LoadedScene { meshes, graph: Graph::Unknown, animations });
    }
    let graph = merged_graph(&master, &batch.scenes, &attachments)?;
    Ok(LoadedScene { meshes, graph: graph.map_or(Graph::Unknown, Graph::Known), animations })
}

/// `InternReadFile`, keeping the node graph. The importer never clears its
/// camera and light lists, and each scene it makes takes all of them: a
/// second scene made by the same importer would own (and delete) the
/// first's again.
pub(crate) fn read_graph(input: &FileInput) -> Result<LoadedScene, ImportError> {
    let mut cameras_and_lights = 0usize;
    let result = read_scene(input, &mut cameras_and_lights);
    let mut state = input.state.borrow_mut();
    state.irr_cameras_and_lights += cameras_and_lights;
    if result.is_ok() {
        if state.irr_owned {
            return undefined("cameras or lights owned by two imported scenes");
        }
        state.irr_owned = state.irr_cameras_and_lights > 0;
    }
    result
}
