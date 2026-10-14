//! X3D files (`X3DImporter`): an XML scene of grouping nodes, shapes and
//! their geometry, read into a graph of node elements, from which each
//! shape's geometry becomes a mesh. `USE` adds an element again wherever
//! it appears (cycles included), `Inline` reads another file into the
//! graph, and the readers' entering and leaving of elements is copied as
//! it is: an empty group, or one taken by `USE`, leaves one level too
//! many, so later siblings hang higher up (or nowhere, which crashes the
//! next element added).

mod geometry;

use std::path::PathBuf;

use geometry::{Extrusion, HALF_PI, TWO_PI, undefined};

use crate::batch::{Graph, LoadedScene};
use crate::cast;
use crate::io::FileSystem;
use crate::msvc::VectorCapacity;
use crate::parsing::{stof, strtod};
use crate::registry::FileInput;
use crate::scene::{ImportError, Mesh, fail};
use crate::shapes;
use crate::vector::{Vector3, scale};
use crate::xml::{self, Node as XmlNode};

/// Recursion deeper than this stands for the native recursion that would
/// overflow the stack.
const MAX_DEPTH: usize = 2048;

/// Files inlining files deeper than this stand for an overflow too (each
/// level parses a whole document on the stack).
const MAX_INLINE_DEPTH: usize = 256;

/// Graph walks visiting more elements than this (shared elements make
/// them exponential) stand for imports that never end.
const MAX_STEPS: u64 = 50_000_000;

/// `tess` for cones and cylinders, and for spheres.
const ROUND_SEGMENTS: u32 = 30;
const SPHERE_TESSELLATION: u32 = 3;

/// The segments of 2D arcs and circles.
const ARC_SEGMENTS: usize = 10;

/// `sizeof(int32_t)`.
const INDEX_SIZE: u64 = 4;

/// The nodes skipped with what they hold (`Uns_Skip`).
const SKIPPED: [&str; 192] = [
    "CADAssembly", "CADFace", "CADLayer", "CADPart", "IndexedQuadSet", "QuadSet", "ROUTE", "ExternProtoDeclare",
    "ProtoDeclare", "ProtoInstance", "ProtoInterface", "WorldInfo", "DISEntityManager", "DISEntityTypeMapping",
    "EspduTransform", "ReceiverPdu", "SignalPdu", "TransmitterPdu", "ComposedCubeMapTexture",
    "GeneratedCubeMapTexture", "ImageCubeMapTexture", "Background", "Fog", "FogCoordinate", "LocalFog",
    "TextureBackground", "ProximitySensor", "TransformSensor", "VisibilitySensor", "ColorChaser", "ColorDamper",
    "CoordinateChaser", "CoordinateDamper", "OrientationChaser", "OrientationDamper", "PositionChaser",
    "PositionChaser2D", "PositionDamper", "PositionDamper2D", "ScalarChaser", "ScalarDamper", "TexCoordChaser2D",
    "TexCoordDamper2D", "GeoCoordinate", "GeoElevationGrid", "GeoLocation", "GeoLOD", "GeoMetadata", "GeoOrigin",
    "GeoPositionInterpolator", "GeoProximitySensor", "GeoTouchSensor", "GeoTransform", "GeoViewpoint",
    "HAnimDisplacer", "HAnimHumanoid", "HAnimJoint", "HAnimSegment", "HAnimSite", "ColorInterpolator",
    "CoordinateInterpolator", "CoordinateInterpolator2D", "EaseInEaseOut", "NormalInterpolator",
    "OrientationInterpolator", "PositionInterpolator", "PositionInterpolator2D", "ScalarInterpolator",
    "SplinePositionInterpolator", "SplinePositionInterpolator2D", "SplineScalarInterpolator",
    "SquadOrientationInterpolator", "KeySensor", "StringSensor", "Layer", "LayerSet", "Viewport", "Layout",
    "LayoutGroup", "LayoutLayer", "ScreenFontStyle", "ScreenGroup", "Billboard", "Collision", "LOD", "NavigationInfo",
    "OrthoViewpoint", "Viewpoint", "ViewpointGroup", "EXPORT", "IMPORT", "Anchor", "LoadSensor", "Contour2D",
    "ContourPolyline2D", "CoordinateDouble", "NurbsCurve", "NurbsCurve2D", "NurbsOrientationInterpolator",
    "NurbsPatchSurface", "NurbsPositionInterpolator", "NurbsSet", "NurbsSurfaceInterpolator", "NurbsSweptSurface",
    "NurbsSwungSurface", "NurbsTextureCoordinate", "NurbsTrimmedSurface", "BoundedPhysicsModel", "ConeEmitter",
    "ExplosionEmitter", "ForcePhysicsModel", "ParticleSystem", "PointEmitter", "PolylineEmitter", "SurfaceEmitter",
    "VolumeEmitter", "WindPhysicsModel", "LinePickSensor", "PickableGroup", "PointPickSensor", "PrimitivePickSensor",
    "VolumePickSensor", "CylinderSensor", "PlaneSensor", "SphereSensor", "TouchSensor", "ClipPlane", "BallJoint",
    "CollidableOffset", "CollidableShape", "CollisionCollection", "CollisionSensor", "CollisionSpace", "Contact",
    "DoubleAxisHingeJoint", "MotorJoint", "RigidBody", "RigidBodyCollection", "SingleAxisHingeJoint", "SliderJoint",
    "UniversalJoint", "Script", "ComposedShader", "FloatVertexAttribute", "Matrix3VertexAttribute",
    "Matrix4VertexAttribute", "PackagedShader", "ProgramShader", "ShaderPart", "ShaderProgram", "FillProperties",
    "LineProperties", "TwoSidedMaterial", "AudioClip", "Sound", "FontStyle", "Text", "ComposedTexture3D",
    "ImageTexture3D", "PixelTexture3D", "TextureCoordinate3D", "TextureCoordinate4D", "TextureTransformMatrix3D",
    "TextureTransform3D", "MovieTexture", "MultiTexture", "MultiTextureCoordinate", "MultiTextureTransform",
    "PixelTexture", "TextureCoordinateGenerator", "TextureProperties", "TimeSensor", "BooleanFilter",
    "BooleanSequencer", "BooleanToggle", "BooleanTrigger", "IntegerSequencer", "IntegerTrigger", "TimeTrigger",
    "BlendedVolumeStyle", "BoundaryEnhancementVolumeStyle", "CartoonVolumeStyle", "ComposedVolumeStyle",
    "EdgeEnhancementVolumeStyle", "IsoSurfaceVolumeData", "OpacityMapVolumeStyle", "ProjectionVolumeStyle",
    "SegmentedVolumeData", "ShadedVolumeStyle", "SilhouetteEnhancementVolumeStyle", "ToneMappedVolumeStyle",
    "VolumeData",
];

/// `X3DElemType`, in its order (error messages give the number).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Group,
    MetaBoolean,
    MetaDouble,
    MetaFloat,
    MetaInteger,
    MetaSet,
    MetaString,
    Arc2D,
    ArcClose2D,
    Circle2D,
    Disk2D,
    Polyline2D,
    Polypoint2D,
    Rectangle2D,
    TriangleSet2D,
    Box,
    Cone,
    Cylinder,
    Sphere,
    ElevationGrid,
    Extrusion,
    Coordinate,
    Normal,
    TextureCoordinate,
    IndexedFaceSet,
    IndexedLineSet,
    IndexedTriangleSet,
    IndexedTriangleFanSet,
    IndexedTriangleStripSet,
    LineSet,
    PointSet,
    TriangleSet,
    TriangleFanSet,
    /// Never made (`readShape` takes no `TriangleStripSet`), but numbered.
    #[allow(dead_code)]
    TriangleStripSet,
    Color,
    ColorRgba,
    Shape,
    Appearance,
    Material,
    ImageTexture,
    TextureTransform,
    DirectionalLight,
    PointLight,
    SpotLight,
}

impl Kind {
    /// `PostprocessHelper_ElementIsMetadata`.
    fn is_metadata(self) -> bool {
        matches!(self, Kind::MetaBoolean | Kind::MetaDouble | Kind::MetaFloat | Kind::MetaInteger | Kind::MetaString | Kind::MetaSet)
    }

    /// `PostprocessHelper_ElementIsMesh`.
    fn is_mesh(self) -> bool {
        matches!(
            self,
            Kind::Arc2D
                | Kind::ArcClose2D
                | Kind::Box
                | Kind::Circle2D
                | Kind::Cone
                | Kind::Cylinder
                | Kind::Disk2D
                | Kind::ElevationGrid
                | Kind::Extrusion
                | Kind::IndexedFaceSet
                | Kind::IndexedLineSet
                | Kind::IndexedTriangleFanSet
                | Kind::IndexedTriangleSet
                | Kind::IndexedTriangleStripSet
                | Kind::PointSet
                | Kind::LineSet
                | Kind::Polyline2D
                | Kind::Polypoint2D
                | Kind::Rectangle2D
                | Kind::Sphere
                | Kind::TriangleFanSet
                | Kind::TriangleSet
                | Kind::TriangleSet2D
                | Kind::TriangleStripSet
        )
    }

    fn is_light(self) -> bool {
        matches!(self, Kind::DirectionalLight | Kind::PointLight | Kind::SpotLight)
    }
}

/// What an element holds that its meshes are made from.
enum Data {
    None,
    /// A group's choice when it is a `Switch` (`UseChoice`).
    Group { is_static: bool, choice: Option<i32> },
    /// `Coordinate` and `Normal` points.
    Points(Vec<Vector3>),
    TexCoords(Vec<[f32; 2]>),
    /// `Color` (alpha 1) and `ColorRGBA`.
    Colors(Vec<[f32; 4]>),
    /// The 2D geometry and the box, cone, cylinder and sphere: vertices
    /// in faces of `index_count` each.
    Shape { vertices: Vec<Vector3>, index_count: usize },
    Grid { vertices: Vec<Vector3>, coord_index: Vec<i32>, color_per_vertex: bool, normal_per_vertex: bool },
    /// `X3DNodeElementIndexedSet` (the extrusion's vertices its own).
    Indexed(IndexedSet),
    /// `X3DNodeElementSet` (line and triangle fan sets).
    Set { coord_index: Vec<i32>, color_per_vertex: bool, normal_per_vertex: bool },
    /// A triangle set, which is made an indexed set but written and read
    /// as a set: its flags land in padding, so its lists read as other
    /// fields.
    TriangleSet { color_per_vertex: bool },
}

#[derive(Default)]
struct IndexedSet {
    coord_index: Vec<i32>,
    color_index: Vec<i32>,
    normal_index: Vec<i32>,
    tex_coord_index: Vec<i32>,
    color_per_vertex: bool,
    normal_per_vertex: bool,
    vertices: Vec<Vector3>,
}

/// `X3DNodeElementBase`.
struct Element {
    parent: Option<usize>,
    id: Vec<u8>,
    children: Vec<usize>,
    kind: Kind,
    data: Data,
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn node_name(node: &XmlNode) -> String {
    lossy(node.name())
}

/// `MACRO_ATTRREAD_CHECKUSEDEF_RET`: the DEF and USE names.
fn def_use(node: &XmlNode) -> (Vec<u8>, Vec<u8>) {
    let def = node.attribute("DEF").unwrap_or_default().to_vec();
    let usage = node.attribute("USE").unwrap_or_default().to_vec();
    (def, usage)
}

fn read_bool(node: &XmlNode, name: &str, value: &mut bool) {
    if let Some(text) = node.attribute(name) {
        *value = xml::as_bool(text);
    }
}

fn read_float(node: &XmlNode, name: &str, value: &mut f32) {
    if let Some(text) = node.attribute(name) {
        *value = xml::as_float(text);
    }
}

fn read_int(node: &XmlNode, name: &str, value: &mut i32) {
    if let Some(text) = node.attribute(name) {
        *value = xml::as_int(text);
    }
}

/// `tokenize(value, tokens, " ")`: the runs between spaces.
fn tokens(value: &[u8]) -> Vec<&[u8]> {
    value.split(|&byte| byte == b' ').filter(|token| !token.is_empty()).collect()
}

/// `Throw_ConvertFail_Str2Arr*` (naming the attribute as its value).
fn convert_fail(node: &XmlNode, attribute: &str, kind: &str) -> ImportError {
    ImportError(format!("In <{}> failed to convert attribute value \"{attribute}\" from string to array of {kind}.", node_name(node)))
}

/// `getVector3DAttribute` (and `getColor3DAttribute`): exactly three
/// numbers, each by `std::stof`.
fn read_vector3(node: &XmlNode, name: &str, value: &mut Vector3) -> Result<(), ImportError> {
    let Some(text) = node.attribute(name) else {
        return Ok(());
    };
    let parts = tokens(text);
    if parts.len() != 3 {
        return Err(convert_fail(node, name, "floats"));
    }
    *value = [stof(parts[0])?, stof(parts[1])?, stof(parts[2])?];
    Ok(())
}

/// `getVector2DAttribute`.
fn read_vector2(node: &XmlNode, name: &str, value: &mut [f32; 2]) -> Result<(), ImportError> {
    let Some(text) = node.attribute(name) else {
        return Ok(());
    };
    let parts = tokens(text);
    if parts.len() != 2 {
        return Err(convert_fail(node, name, "floats"));
    }
    *value = [stof(parts[0])?, stof(parts[1])?];
    Ok(())
}

/// `atof`, as a float.
fn atof(text: &[u8]) -> f32 {
    strtod(text) as f32
}

/// `getFloatArrayAttribute`: whether the attribute is there.
fn float_array(node: &XmlNode, name: &str, values: &mut Vec<f32>) -> bool {
    let Some(text) = node.attribute(name) else {
        return false;
    };
    values.extend(tokens(text).into_iter().map(atof));
    true
}

/// `getInt32ArrayAttribute`: `(int32_t)atof(...)`.
fn int_array(node: &XmlNode, name: &str) -> Vec<i32> {
    let Some(text) = node.attribute(name) else {
        return Vec::new();
    };
    tokens(text).into_iter().map(|token| cast::to_int(strtod(token))).collect()
}

/// The numbers of a list attribute in groups of `width` (a number left
/// over throws).
fn number_groups(node: &XmlNode, name: &str, width: usize) -> Result<Vec<Vec<f32>>, ImportError> {
    let Some(text) = node.attribute(name) else {
        return Ok(Vec::new());
    };
    let parts = tokens(text);
    if !parts.len().is_multiple_of(width) {
        return Err(convert_fail(node, name, "floats"));
    }
    let groups = parts.chunks(width).map(|group| group.iter().map(|&part| atof(part)).collect()).collect();
    Ok(groups)
}

/// `getVector2DListAttribute`.
fn vector2_list(node: &XmlNode, name: &str) -> Result<Vec<[f32; 2]>, ImportError> {
    let groups = number_groups(node, name, 2)?;
    Ok(groups.into_iter().map(|group| [group[0], group[1]]).collect())
}

/// `getVector3DListAttribute`.
fn vector3_list(node: &XmlNode, name: &str) -> Result<Vec<Vector3>, ImportError> {
    let groups = number_groups(node, name, 3)?;
    Ok(groups.into_iter().map(|group| [group[0], group[1], group[2]]).collect())
}

/// `getColor3DListAttribute`, as colours of alpha 1.
fn color3_list(node: &XmlNode, name: &str) -> Result<Vec<[f32; 4]>, ImportError> {
    let groups = number_groups(node, name, 3)?;
    Ok(groups.into_iter().map(|group| [group[0], group[1], group[2], 1.0]).collect())
}

/// `getColor4DListAttribute`.
fn color4_list(node: &XmlNode, name: &str) -> Result<Vec<[f32; 4]>, ImportError> {
    let groups = number_groups(node, name, 4)?;
    Ok(groups.into_iter().map(|group| [group[0], group[1], group[2], group[3]]).collect())
}

/// `getStringListAttribute`: the words, a word starting with '"' joined
/// to the following ones (without a space before the last) up to one
/// ending with '"'; a quote left open throws.
fn string_list(node: &XmlNode, name: &str) -> Result<Vec<Vec<u8>>, ImportError> {
    let Some(text) = node.attribute(name) else {
        return Ok(Vec::new());
    };
    let mut list = Vec::new();
    let mut joined: Vec<u8> = Vec::new();
    let mut in_quotes = false;
    for token in tokens(text) {
        if in_quotes {
            if token.last() == Some(&b'"') {
                joined.extend_from_slice(&token[..token.len() - 1]);
                list.push(std::mem::take(&mut joined));
                in_quotes = false;
            } else {
                joined.push(b' ');
                joined.extend_from_slice(token);
            }
        } else if token[0] == b'"' {
            joined = token[1..].to_vec();
            in_quotes = true;
        } else {
            list.push(token.to_vec());
        }
    }
    if in_quotes {
        return Err(convert_fail(node, name, "integers"));
    }
    Ok(list)
}

/// Which children an element's reader takes besides metadata.
#[derive(Clone, Copy)]
enum ChildSet {
    /// `Color`, `ColorRGBA`, `Normal`, `TextureCoordinate`.
    Grid,
    /// Those and `Coordinate`.
    Full,
    /// `Color`, `ColorRGBA`, `Coordinate`.
    Lines,
}

/// The reader's state: the element graph and list, the current element,
/// and the files being read (for files inlining themselves).
struct Reader<'a> {
    files: &'a FileSystem,
    elements: Vec<Element>,
    /// `NodeElement_List`, in the order elements were added to it.
    list: Vec<usize>,
    current: Option<usize>,
    open_files: Vec<PathBuf>,
    steps: u64,
}

impl Reader<'_> {
    fn step(&mut self) -> Result<(), ImportError> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return fail("endless loop: an X3D node graph walked without end");
        }
        Ok(())
    }

    /// A new element, its parent the current one (not yet its child).
    fn create(&mut self, kind: Kind, id: Vec<u8>, data: Data) -> usize {
        let element = Element { parent: self.current, id, children: Vec::new(), kind, data };
        self.elements.push(element);
        self.elements.len() - 1
    }

    /// `mNodeElementCur->Children.push_back`: with no current element, a
    /// null pointer written through.
    fn add_to_current(&mut self, element: usize) -> Result<(), ImportError> {
        let Some(current) = self.current else {
            return undefined("an X3D element added to no current element");
        };
        self.elements[current].children.push(element);
        Ok(())
    }

    /// `ParseHelper_Group_Begin`.
    fn group_begin(&mut self, is_static: bool) -> usize {
        let group = self.create(Kind::Group, Vec::new(), Data::Group { is_static, choice: None });
        if let Some(current) = self.current {
            self.elements[current].children.push(group);
        }
        self.list.push(group);
        self.current = Some(group);
        group
    }

    /// `ParseHelper_Node_Enter`.
    fn node_enter(&mut self, element: usize) -> Result<(), ImportError> {
        self.add_to_current(element)?;
        self.current = Some(element);
        Ok(())
    }

    /// `ParseHelper_Node_Exit`.
    fn node_exit(&mut self) {
        if let Some(current) = self.current {
            self.current = self.elements[current].parent;
        }
    }

    fn is_static_group(&self, element: usize) -> bool {
        matches!(self.elements[element].data, Data::Group { is_static: true, .. })
    }

    /// `FindNodeElement_FromNode`: depth first from `start`.
    fn find_from_node(&mut self, start: usize, id: &[u8], kind: Kind, depth: usize) -> Result<Option<usize>, ImportError> {
        if depth > MAX_DEPTH {
            return undefined("an X3D element search deeper than the stack (a USE cycle)");
        }
        self.step()?;
        let element = &self.elements[start];
        if element.kind == kind && element.id == id {
            return Ok(Some(start));
        }
        let children = element.children.clone();
        for child in children {
            if let Some(found) = self.find_from_node(child, id, kind, depth + 1)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// `FindNodeElement`: below the nearest static group up from the
    /// current element, else in the element list.
    fn find_element(&mut self, id: &[u8], kind: Kind) -> Result<Option<usize>, ImportError> {
        let mut walker = self.current;
        while let Some(element) = walker {
            if self.elements[element].kind == Kind::Group && self.is_static_group(element) {
                return self.find_from_node(element, id, kind, 0);
            }
            walker = self.elements[element].parent;
        }
        let found = self.list.iter().copied().find(|&element| self.elements[element].kind == kind && self.elements[element].id == id);
        Ok(found)
    }

    /// `MACRO_USE_CHECKANDAPPLY`: the node must be empty and have no DEF;
    /// the element it names becomes a child of the current one again.
    fn use_element(&mut self, node: &XmlNode, def: &[u8], usage: &[u8], kind: Kind) -> Result<(), ImportError> {
        if !node.children.is_empty() {
            return fail(format!("Node <{}> must be empty.", node_name(node)));
        }
        if !def.is_empty() {
            return fail(format!("\"DEF\" and \"USE\" can not be defined both in <{}>.", node_name(node)));
        }
        let Some(found) = self.find_element(usage, kind)? else {
            return fail(format!("Not found node with name \"{}\" in <{}>.", lossy(usage), node_name(node)));
        };
        self.add_to_current(found)
    }

    /// `skipUnsupportedNode`: text and comments, and the nodes listed, are
    /// passed over; anything else throws.
    fn skip_unsupported(&self, parent: &str, node: &XmlNode) -> Result<(), ImportError> {
        let name = node.name();
        if name.is_empty() && !node.value.is_empty() {
            return Ok(());
        }
        if SKIPPED.iter().any(|&skipped| skipped.as_bytes() == name) {
            return Ok(());
        }
        fail(format!("Unknown node \"{}\" in {parent}.", lossy(name)))
    }

    /// `checkForMetadataNode`: whether the node is metadata (and read).
    fn check_for_metadata(&mut self, node: &XmlNode, depth: usize) -> Result<bool, ImportError> {
        let kind = match node.name() {
            b"MetadataBoolean" => Kind::MetaBoolean,
            b"MetadataDouble" => Kind::MetaDouble,
            b"MetadataFloat" => Kind::MetaFloat,
            b"MetadataInteger" => Kind::MetaInteger,
            b"MetadataSet" => Kind::MetaSet,
            b"MetadataString" => Kind::MetaString,
            _ => return Ok(false),
        };
        self.read_metadata(node, kind, depth)?;
        Ok(true)
    }

    /// `readMetadata*`: only a string's value can throw.
    fn read_metadata(&mut self, node: &XmlNode, kind: Kind, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        if kind == Kind::MetaString {
            string_list(node, "value")?;
        }
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        let element = self.create(kind, def, Data::None);
        let name = match kind {
            Kind::MetaBoolean => "MetadataBoolean",
            Kind::MetaDouble => "MetadataDouble",
            Kind::MetaFloat => "MetadataFloat",
            Kind::MetaInteger => "MetadataInteger",
            Kind::MetaSet => "MetadataSet",
            _ => "MetadataString",
        };
        self.finish_with_metadata(node, element, name, depth)
    }

    /// `childrenReadMetadata`: the element entered (as a child of the
    /// current one), its metadata children read, and left.
    fn children_read_metadata(&mut self, node: &XmlNode, element: usize, name: &str, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return undefined("X3D metadata nested deeper than the stack");
        }
        self.node_enter(element)?;
        for child in &node.children {
            if !self.check_for_metadata(child, depth + 1)? {
                self.skip_unsupported(name, child)?;
            }
        }
        self.node_exit();
        Ok(())
    }

    /// An element's metadata children (or the element added as a child
    /// when there are none), then the element listed.
    fn finish_with_metadata(&mut self, node: &XmlNode, element: usize, name: &str, depth: usize) -> Result<(), ImportError> {
        if !node.children.is_empty() {
            self.children_read_metadata(node, element, name, depth)?;
        } else {
            self.add_to_current(element)?;
        }
        self.list.push(element);
        Ok(())
    }

    /// A geometry element's children (colours, coordinates, normals,
    /// texture coordinates as `children` allows, metadata), then the
    /// element listed.
    fn finish_with_children(&mut self, node: &XmlNode, element: usize, name: &str, children: ChildSet, depth: usize) -> Result<(), ImportError> {
        if node.children.is_empty() {
            self.add_to_current(element)?;
            self.list.push(element);
            return Ok(());
        }
        self.node_enter(element)?;
        for child in &node.children {
            let takes_coordinates = matches!(children, ChildSet::Full | ChildSet::Lines);
            let takes_normals = matches!(children, ChildSet::Full | ChildSet::Grid);
            match child.name() {
                b"Color" => self.read_color(child, false, depth + 1)?,
                b"ColorRGBA" => self.read_color(child, true, depth + 1)?,
                b"Coordinate" if takes_coordinates => self.read_points(child, Kind::Coordinate, depth + 1)?,
                b"Normal" if takes_normals => self.read_points(child, Kind::Normal, depth + 1)?,
                b"TextureCoordinate" if takes_normals => self.read_texture_coordinate(child, depth + 1)?,
                _ => {
                    if !self.check_for_metadata(child, depth + 1)? {
                        self.skip_unsupported(name, child)?;
                    }
                }
            }
        }
        self.node_exit();
        self.list.push(element);
        Ok(())
    }

    /// `ParseFile` for an inlined file: one that cannot be opened throws;
    /// a file read again inside itself never ends.
    fn parse_file(&mut self, path: &[u8], depth: usize) -> Result<(), ImportError> {
        let Some((bytes, resolved)) = self.files.open_resolved(path) else {
            return fail(format!("Failed to open file {}.", lossy(path)));
        };
        let identity = FileSystem::identify(&resolved);
        if let Some(identity) = &identity
            && self.open_files.contains(identity)
        {
            return undefined("an X3D file inlined inside itself (endless recursion)");
        }
        if self.open_files.len() >= MAX_INLINE_DEPTH {
            return undefined("X3D files inlined deeper than the stack");
        }
        let pushed = identity.is_some();
        if let Some(identity) = identity {
            self.open_files.push(identity);
        }
        self.parse_document(&bytes, depth)?;
        if pushed {
            self.open_files.pop();
        }
        Ok(())
    }

    /// `ParseFile` past the opening: nothing for a document that does not
    /// parse or has no `X3D` node; its `head` and `Scene` children read.
    fn parse_document(&mut self, bytes: &[u8], depth: usize) -> Result<(), ImportError> {
        let Ok(document) = xml::parse(bytes) else {
            return Ok(());
        };
        let Some(x3d) = document.find_node("X3D") else {
            return Ok(());
        };
        for child in &x3d.children {
            match child.name() {
                b"head" => {}
                b"Scene" => {
                    self.group_begin(true);
                    self.read_child_nodes(child, "Scene", depth + 1)?;
                    self.node_exit();
                }
                _ => self.skip_unsupported("X3D", child)?,
            }
        }
        Ok(())
    }

    /// `readChildNodes`.
    fn read_child_nodes(&mut self, node: &XmlNode, parent: &str, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return undefined("X3D groups nested deeper than the stack");
        }
        for child in &node.children {
            match child.name() {
                b"Shape" => self.read_shape(child, depth + 1)?,
                b"Group" => {
                    self.start_group(child, false)?;
                    self.read_child_nodes(child, "Group", depth + 1)?;
                    self.node_exit();
                }
                b"StaticGroup" => {
                    self.start_group(child, true)?;
                    self.read_child_nodes(child, "StaticGroup", depth + 1)?;
                    self.node_exit();
                }
                b"Transform" => {
                    self.start_transform(child)?;
                    self.read_child_nodes(child, "Transform", depth + 1)?;
                    self.node_exit();
                }
                b"Switch" => {
                    self.start_switch(child)?;
                    self.read_child_nodes(child, "Switch", depth + 1)?;
                    self.node_exit();
                }
                b"DirectionalLight" => self.read_light(child, Kind::DirectionalLight, depth + 1)?,
                b"PointLight" => self.read_light(child, Kind::PointLight, depth + 1)?,
                b"SpotLight" => self.read_light(child, Kind::SpotLight, depth + 1)?,
                b"Inline" => self.read_inline(child, depth + 1)?,
                _ => {
                    if !self.check_for_metadata(child, depth + 1)? {
                        self.skip_unsupported(parent, child)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `startReadGroup` and `startReadStaticGroup`: an empty group is left
    /// at once (and again by its end).
    fn start_group(&mut self, node: &XmlNode, is_static: bool) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Group);
        }
        let group = self.group_begin(is_static);
        if !def.is_empty() {
            self.elements[group].id = def;
        }
        if node.children.is_empty() {
            self.node_exit();
        }
        Ok(())
    }

    /// `startReadSwitch`: a group keeping only its `whichChoice` child.
    fn start_switch(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut which_choice = -1;
        read_int(node, "whichChoice", &mut which_choice);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Group);
        }
        let group = self.group_begin(false);
        if !def.is_empty() {
            self.elements[group].id = def;
        }
        self.elements[group].data = Data::Group { is_static: false, choice: Some(which_choice) };
        if node.children.is_empty() {
            self.node_exit();
        }
        Ok(())
    }

    /// `startReadTransform` (its matrix moves no mesh): with USE and no
    /// current element, a new group holds the element used.
    fn start_transform(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ignored = [0.0f32; 3];
        read_vector3(node, "center", &mut ignored)?;
        read_vector3(node, "scale", &mut ignored)?;
        read_vector3(node, "translation", &mut ignored)?;
        let mut rotation = Vec::new();
        if float_array(node, "rotation", &mut rotation) && rotation.len() != 4 {
            return fail("<Transform>: rotation vector must have 4 elements.");
        }
        let mut scale_orientation = Vec::new();
        if float_array(node, "scaleOrientation", &mut scale_orientation) && scale_orientation.len() != 4 {
            return fail("<Transform>: scaleOrientation vector must have 4 elements.");
        }
        if !usage.is_empty() {
            let new_group = self.current.is_none();
            if new_group {
                self.group_begin(false);
            }
            self.use_element(node, &def, &usage, Kind::Group)?;
            if new_group && node.children.is_empty() {
                self.node_exit();
            }
            return Ok(());
        }
        let group = self.group_begin(false);
        if !def.is_empty() {
            self.elements[group].id = def;
        }
        if node.children.is_empty() {
            self.node_exit();
        }
        Ok(())
    }

    /// `readDirectionalLight`, `readPointLight`, `readSpotLight`: a light
    /// that is on, after an empty group of its name.
    fn read_light(&mut self, node: &XmlNode, kind: Kind, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ignored_float = 0.0f32;
        let mut ignored_vector = [0.0f32; 3];
        let mut on = true;
        read_float(node, "ambientIntensity", &mut ignored_float);
        match kind {
            Kind::DirectionalLight => {
                read_vector3(node, "color", &mut ignored_vector)?;
                read_vector3(node, "direction", &mut ignored_vector)?;
            }
            Kind::PointLight => {
                read_vector3(node, "attenuation", &mut ignored_vector)?;
                read_vector3(node, "color", &mut ignored_vector)?;
                read_vector3(node, "location", &mut ignored_vector)?;
            }
            _ => {
                read_vector3(node, "attenuation", &mut ignored_vector)?;
                read_vector3(node, "color", &mut ignored_vector)?;
                read_vector3(node, "direction", &mut ignored_vector)?;
                read_vector3(node, "location", &mut ignored_vector)?;
            }
        }
        read_bool(node, "on", &mut on);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        if !on {
            return Ok(());
        }
        // Without a DEF the light is named after its address, which no
        // USE can give.
        let id = if def.is_empty() { b"\0light".to_vec() } else { def };
        let light = self.create(kind, id.clone(), Data::None);
        let group = self.group_begin(false);
        self.elements[group].id = id;
        self.node_exit();
        let name = match kind {
            Kind::DirectionalLight => "DirectionalLight",
            Kind::PointLight => "PointLight",
            _ => "SpotLight",
        };
        self.finish_with_metadata(node, light, name, depth)
    }

    /// `readInline`: a static group, into which the file named first in
    /// `url` (relative to the current directory, "/../" folded) is read;
    /// metadata children are then read into whatever element is current,
    /// which is entered as a child of itself.
    fn read_inline(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut load = true;
        read_bool(node, "load", &mut load);
        let url = string_list(node, "url")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Group);
        }
        let group = self.group_begin(true);
        if !def.is_empty() {
            self.elements[group].id = def;
        }
        if load && let Some(first) = url.first() {
            let mut full_path = self.files.current_directory();
            full_path.extend_from_slice(first);
            fold_parent_directories(&mut full_path);
            let directory = match full_path.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
                Some(slash) => full_path[..=slash].to_vec(),
                None => Vec::new(),
            };
            self.files.push_directory(&directory);
            self.parse_file(&full_path, depth)?;
            self.files.pop_directory();
        }
        if !node.children.is_empty() {
            let Some(current) = self.current else {
                return undefined("an X3D element entered with no current element");
            };
            self.children_read_metadata(node, current, "Inline", depth)?;
        }
        self.node_exit();
        Ok(())
    }

    /// `readShape`.
    fn read_shape(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Shape);
        }
        let shape = self.create(Kind::Shape, def, Data::None);
        if node.children.is_empty() {
            self.add_to_current(shape)?;
            self.list.push(shape);
            return Ok(());
        }
        self.node_enter(shape)?;
        for child in &node.children {
            let child_depth = depth + 1;
            match child.name() {
                b"Appearance" => self.read_appearance(child, child_depth)?,
                b"Arc2D" => self.read_arc_2d(child, Kind::Arc2D, child_depth)?,
                b"ArcClose2D" => self.read_arc_close_2d(child, child_depth)?,
                b"Circle2D" => self.read_arc_2d(child, Kind::Circle2D, child_depth)?,
                b"Disk2D" => self.read_disk_2d(child, child_depth)?,
                b"Polyline2D" => self.read_polyline_2d(child, child_depth)?,
                b"Polypoint2D" => self.read_polypoint_2d(child, child_depth)?,
                b"Rectangle2D" => self.read_rectangle_2d(child, child_depth)?,
                b"TriangleSet2D" => self.read_triangle_set_2d(child, child_depth)?,
                b"Box" => self.read_box(child, child_depth)?,
                b"Cone" => self.read_cone(child, child_depth)?,
                b"Cylinder" => self.read_cylinder(child, child_depth)?,
                b"ElevationGrid" => self.read_elevation_grid(child, child_depth)?,
                b"Extrusion" => self.read_extrusion(child, child_depth)?,
                b"IndexedFaceSet" => self.read_indexed_face_set(child, child_depth)?,
                b"Sphere" => self.read_sphere(child, child_depth)?,
                b"IndexedLineSet" => self.read_indexed_line_set(child, child_depth)?,
                b"LineSet" => self.read_line_set(child, child_depth)?,
                b"PointSet" => self.read_point_set(child, child_depth)?,
                b"IndexedTriangleFanSet" => self.read_indexed_triangles(child, Kind::IndexedTriangleFanSet, child_depth)?,
                b"IndexedTriangleSet" => self.read_indexed_triangles(child, Kind::IndexedTriangleSet, child_depth)?,
                b"IndexedTriangleStripSet" => self.read_indexed_triangles(child, Kind::IndexedTriangleStripSet, child_depth)?,
                b"TriangleFanSet" => self.read_triangle_fan_set(child, child_depth)?,
                b"TriangleSet" => self.read_triangle_set(child, child_depth)?,
                _ => {
                    if !self.check_for_metadata(child, child_depth)? {
                        self.skip_unsupported("Shape", child)?;
                    }
                }
            }
        }
        self.node_exit();
        self.list.push(shape);
        Ok(())
    }

    /// `readAppearance`.
    fn read_appearance(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Appearance);
        }
        let appearance = self.create(Kind::Appearance, def, Data::None);
        if node.children.is_empty() {
            self.add_to_current(appearance)?;
            self.list.push(appearance);
            return Ok(());
        }
        self.node_enter(appearance)?;
        for child in &node.children {
            match child.name() {
                b"Material" => self.read_material(child, depth + 1)?,
                b"ImageTexture" => self.read_image_texture(child, depth + 1)?,
                b"TextureTransform" => self.read_texture_transform(child, depth + 1)?,
                _ => {
                    if !self.check_for_metadata(child, depth + 1)? {
                        self.skip_unsupported("Appearance", child)?;
                    }
                }
            }
        }
        self.node_exit();
        self.list.push(appearance);
        Ok(())
    }

    /// `readMaterial`: its colours can throw.
    fn read_material(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ignored = [0.0f32; 3];
        read_vector3(node, "diffuseColor", &mut ignored)?;
        read_vector3(node, "emissiveColor", &mut ignored)?;
        read_vector3(node, "specularColor", &mut ignored)?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Material);
        }
        let material = self.create(Kind::Material, def, Data::None);
        self.finish_with_metadata(node, material, "Material", depth)
    }

    /// `readImageTexture`: its `url` can throw.
    fn read_image_texture(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        string_list(node, "url")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::ImageTexture);
        }
        let texture = self.create(Kind::ImageTexture, def, Data::None);
        self.finish_with_metadata(node, texture, "ImageTexture", depth)
    }

    /// `readTextureTransform`: its vectors can throw.
    fn read_texture_transform(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ignored = [0.0f32; 2];
        read_vector2(node, "center", &mut ignored)?;
        read_vector2(node, "scale", &mut ignored)?;
        read_vector2(node, "translation", &mut ignored)?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::TextureTransform);
        }
        let transform = self.create(Kind::TextureTransform, def, Data::None);
        self.finish_with_metadata(node, transform, "TextureTransform", depth)
    }

    /// `readColor` and `readColorRGBA`.
    fn read_color(&mut self, node: &XmlNode, alpha: bool, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let colors = if alpha { color4_list(node, "color")? } else { color3_list(node, "color")? };
        let kind = if alpha { Kind::ColorRgba } else { Kind::Color };
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        let element = self.create(kind, def, Data::Colors(colors));
        let name = if alpha { "ColorRGBA" } else { "Color" };
        self.finish_with_metadata(node, element, name, depth)
    }

    /// `readCoordinate` (`point`) and `readNormal` (`vector`).
    fn read_points(&mut self, node: &XmlNode, kind: Kind, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let (attribute, name) = if kind == Kind::Coordinate { ("point", "Coordinate") } else { ("vector", "Normal") };
        let points = vector3_list(node, attribute)?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        let element = self.create(kind, def, Data::Points(points));
        self.finish_with_metadata(node, element, name, depth)
    }

    /// `readTextureCoordinate`.
    fn read_texture_coordinate(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let points = vector2_list(node, "point")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::TextureCoordinate);
        }
        let element = self.create(Kind::TextureCoordinate, def, Data::TexCoords(points));
        self.finish_with_metadata(node, element, "TextureCoordinate", depth)
    }

    /// A shape element of vertices in faces of `index_count`.
    fn add_shape(&mut self, node: &XmlNode, kind: Kind, def: Vec<u8>, vertices: Vec<Vector3>, index_count: usize, depth: usize) -> Result<(), ImportError> {
        let element = self.create(kind, def, Data::Shape { vertices, index_count });
        let name = node_name(node);
        self.finish_with_metadata(node, element, &name, depth)
    }

    /// `readArc2D` and `readCircle2D`: the arc as line segments.
    fn read_arc_2d(&mut self, node: &XmlNode, kind: Kind, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut end_angle = HALF_PI;
        let mut radius = 1.0f32;
        let mut start_angle = 0.0f32;
        if kind == Kind::Arc2D {
            read_float(node, "endAngle", &mut end_angle);
            read_float(node, "radius", &mut radius);
            read_float(node, "startAngle", &mut start_angle);
        } else {
            read_float(node, "radius", &mut radius);
        }
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        let (start, end) = if kind == Kind::Arc2D { (start_angle, end_angle) } else { (0.0, 0.0) };
        let mut points = Vec::new();
        geometry::arc_2d(start, end, radius, ARC_SEGMENTS, &mut points)?;
        let vertices = geometry::point_to_line(&points)?;
        self.add_shape(node, kind, def, vertices, 2, depth)
    }

    /// `readArcClose2D`: the arc closed through the centre ("PIE") or
    /// across ("CHORD") unless it is a full turn, as one polygon.
    fn read_arc_close_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let closure_type = node.attribute("closureType").unwrap_or(b"PIE").to_vec();
        let mut end_angle = HALF_PI;
        let mut radius = 1.0f32;
        let mut start_angle = 0.0f32;
        read_float(node, "endAngle", &mut end_angle);
        read_float(node, "radius", &mut radius);
        read_float(node, "startAngle", &mut start_angle);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::ArcClose2D);
        }
        let mut vertices = Vec::new();
        geometry::arc_2d(start_angle, end_angle, radius, ARC_SEGMENTS, &mut vertices)?;
        let full_turn = (end_angle - start_angle).abs() >= TWO_PI || end_angle == start_angle;
        if !full_turn {
            if closure_type == b"PIE" || closure_type == b"\"PIE\"" {
                vertices.push([0.0, 0.0, 0.0]);
            } else if closure_type != b"CHORD" && closure_type != b"\"CHORD\"" {
                return fail("Attribute \"closureType\" in node <ArcClose2D> has incorrect value.");
            }
            vertices.push(vertices[0]);
        }
        let index_count = vertices.len();
        self.add_shape(node, Kind::ArcClose2D, def, vertices, index_count, depth)
    }

    /// `readDisk2D`: a filled disk, a circle, or a ring (whose quads are
    /// read past the ends of the two circles' point lists).
    fn read_disk_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut inner_radius = 0.0f32;
        let mut outer_radius = 1.0f32;
        read_float(node, "innerRadius", &mut inner_radius);
        read_float(node, "outerRadius", &mut outer_radius);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Disk2D);
        }
        if inner_radius > outer_radius {
            return fail("Attribute \"innerRadius\" in node <Disk2D> has incorrect value.");
        }
        let mut outer = Vec::new();
        geometry::arc_2d(0.0, 0.0, outer_radius, ARC_SEGMENTS, &mut outer)?;
        let (vertices, index_count) = if inner_radius == 0.0 {
            let count = outer.len();
            (outer, count)
        } else if inner_radius == outer_radius {
            (geometry::point_to_line(&outer)?, 2)
        } else {
            let mut inner = Vec::new();
            geometry::arc_2d(0.0, 0.0, inner_radius, ARC_SEGMENTS, &mut inner)?;
            return undefined("a Disk2D ring's quads read past its circles' points");
        };
        self.add_shape(node, Kind::Disk2D, def, vertices, index_count, depth)
    }

    /// `readPolyline2D`: line segments through the points.
    fn read_polyline_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let segments = vector2_list(node, "lineSegments")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Polyline2D);
        }
        let points: Vec<Vector3> = segments.iter().map(|&[x, y]| [x, y, 0.0]).collect();
        let vertices = geometry::point_to_line(&points)?;
        self.add_shape(node, Kind::Polyline2D, def, vertices, 2, depth)
    }

    /// `readPolypoint2D`.
    fn read_polypoint_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let points = vector2_list(node, "point")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Polypoint2D);
        }
        let vertices = points.iter().map(|&[x, y]| [x, y, 0.0]).collect();
        self.add_shape(node, Kind::Polypoint2D, def, vertices, 1, depth)
    }

    /// `readRectangle2D`.
    fn read_rectangle_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut size = [2.0f32, 2.0];
        read_vector2(node, "size", &mut size)?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Rectangle2D);
        }
        let x1 = -size[0] / 2.0;
        let x2 = size[0] / 2.0;
        let y1 = -size[1] / 2.0;
        let y2 = size[1] / 2.0;
        let vertices = vec![[x2, y1, 0.0], [x2, y2, 0.0], [x1, y2, 0.0], [x1, y1, 0.0]];
        self.add_shape(node, Kind::Rectangle2D, def, vertices, 4, depth)
    }

    /// `readTriangleSet2D`.
    fn read_triangle_set_2d(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let points = vector2_list(node, "vertices")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::TriangleSet2D);
        }
        if points.len() % 3 != 0 {
            return fail("TriangleSet2D. Not enough points for defining triangle.");
        }
        let vertices = points.iter().map(|&[x, y]| [x, y, 0.0]).collect();
        self.add_shape(node, Kind::TriangleSet2D, def, vertices, 3, depth)
    }

    /// `readBox`.
    fn read_box(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut size = [2.0f32, 2.0, 2.0];
        read_vector3(node, "size", &mut size)?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Box);
        }
        let vertices = geometry::box_quads(size);
        self.add_shape(node, Kind::Box, def, vertices, 4, depth)
    }

    /// `readCone`: its side (open without a bottom), or just the bottom
    /// circle.
    fn read_cone(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut side = true;
        let mut bottom = true;
        let mut height = 2.0f32;
        let mut bottom_radius = 1.0f32;
        read_bool(node, "side", &mut side);
        read_bool(node, "bottom", &mut bottom);
        read_float(node, "height", &mut height);
        read_float(node, "bottomRadius", &mut bottom_radius);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Cone);
        }
        let mut vertices = Vec::new();
        if side {
            shapes::cone(height, 0.0, bottom_radius, ROUND_SEGMENTS, !bottom, &mut vertices)?;
        } else if bottom {
            shapes::circle(bottom_radius, ROUND_SEGMENTS, &mut vertices)?;
            let base = -(height / 2.0);
            for vertex in &mut vertices {
                vertex[1] = base;
            }
        }
        self.add_shape(node, Kind::Cone, def, vertices, 3, depth)
    }

    /// `readCylinder`: the open side, then the top and bottom circles.
    fn read_cylinder(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut radius = 1.0f32;
        let mut bottom = true;
        let mut top = true;
        let mut side = true;
        let mut height = 2.0f32;
        read_float(node, "radius", &mut radius);
        read_bool(node, "bottom", &mut bottom);
        read_bool(node, "top", &mut top);
        read_bool(node, "side", &mut side);
        read_float(node, "height", &mut height);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Cylinder);
        }
        let mut vertices = Vec::new();
        if side {
            shapes::cone(height, radius, radius, ROUND_SEGMENTS, true, &mut vertices)?;
        }
        let half_height = height / 2.0;
        let mut circle = Vec::new();
        if top || bottom {
            shapes::circle(radius, ROUND_SEGMENTS, &mut circle)?;
        }
        if top {
            for point in &circle {
                vertices.push([point[0], half_height, point[2]]);
            }
        }
        if bottom {
            for point in &circle {
                vertices.push([point[0], -half_height, point[2]]);
            }
        }
        self.add_shape(node, Kind::Cylinder, def, vertices, 3, depth)
    }

    /// `readSphere`.
    fn read_sphere(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut radius = 1.0f32;
        read_float(node, "radius", &mut radius);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Sphere);
        }
        let mut points = Vec::new();
        shapes::sphere(SPHERE_TESSELLATION, &mut points)?;
        let vertices = points.iter().map(|&point| scale(point, radius)).collect();
        self.add_shape(node, Kind::Sphere, def, vertices, 3, depth)
    }

    /// `readElevationGrid`: a grid of heights as quads (or, one row or
    /// column wide, as lines).
    fn read_elevation_grid(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ccw = true;
        let mut color_per_vertex = true;
        let mut normal_per_vertex = true;
        let mut heights = Vec::new();
        let mut x_dimension = 0i32;
        let mut x_spacing = 1.0f32;
        let mut z_dimension = 0i32;
        let mut z_spacing = 1.0f32;
        read_bool(node, "ccw", &mut ccw);
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        read_bool(node, "normalPerVertex", &mut normal_per_vertex);
        float_array(node, "height", &mut heights);
        read_int(node, "xDimension", &mut x_dimension);
        read_float(node, "xSpacing", &mut x_spacing);
        read_int(node, "zDimension", &mut z_dimension);
        read_float(node, "zSpacing", &mut z_spacing);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::ElevationGrid);
        }
        if x_spacing == 0.0 || z_spacing == 0.0 {
            return fail("Spacing in <ElevationGrid> must be grater than zero.");
        }
        if x_dimension <= 0 || z_dimension <= 0 {
            return fail("Dimension in <ElevationGrid> must be grater than zero.");
        }
        let grid_size = x_dimension as u64 * z_dimension as u64;
        if (heights.len() as u64) < grid_size {
            return undefined("ElevationGrid heights read past their list");
        }
        let mut vertices = Vec::with_capacity(grid_size as usize);
        let mut height = heights.iter();
        for zi in 0..z_dimension {
            for xi in 0..x_dimension {
                let y = height.next().copied().unwrap_or_default();
                vertices.push([x_spacing * xi as f32, y, z_spacing * zi as f32]);
            }
        }
        let mut coord_index = Vec::new();
        let mut capacity = VectorCapacity::new(INDEX_SIZE);
        if x_dimension < 2 || z_dimension < 2 {
            capacity.push_to((vertices.len() - 1) * 3)?;
            for index in 0..vertices.len() - 1 {
                coord_index.extend_from_slice(&[index as i32, index as i32 + 1, -1]);
            }
        } else {
            let quads = (x_dimension as usize - 1) * (z_dimension as usize - 1);
            capacity.push_to(quads * 5)?;
            for row in 0..z_dimension - 1 {
                for column in 0..x_dimension - 1 {
                    let corners = [
                        (row + 1) * x_dimension + column,
                        (row + 1) * x_dimension + column + 1,
                        row * x_dimension + column + 1,
                        row * x_dimension + column,
                    ];
                    if ccw {
                        coord_index.extend_from_slice(&corners);
                    } else {
                        coord_index.extend_from_slice(&[corners[3], corners[2], corners[1], corners[0]]);
                    }
                    coord_index.push(-1);
                }
            }
        }
        let data = Data::Grid { vertices, coord_index, color_per_vertex, normal_per_vertex };
        let element = self.create(Kind::ElevationGrid, def, data);
        self.finish_with_children(node, element, "ElevationGrid", ChildSet::Grid, depth)
    }

    /// `readExtrusion`: defaults and padding, then the surface.
    fn read_extrusion(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut begin_cap = true;
        let mut ccw = true;
        let mut end_cap = true;
        read_bool(node, "beginCap", &mut begin_cap);
        read_bool(node, "ccw", &mut ccw);
        let mut cross_section = vector2_list(node, "crossSection")?;
        read_bool(node, "endCap", &mut end_cap);
        let mut orientation = Vec::new();
        float_array(node, "orientation", &mut orientation);
        let mut scale_values = vector2_list(node, "scale")?;
        let mut spine = vector3_list(node, "spine")?;
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::Extrusion);
        }
        match spine.len() {
            0 => spine = vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            1 => return fail("ParseNode_Geometry3D_Extrusion. Spine must have at least two points."),
            _ => {}
        }
        if cross_section.is_empty() {
            cross_section = vec![[1.0, 1.0], [1.0, -1.0], [-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
        }
        let rotations = orientation.len() / 4;
        if rotations < spine.len() {
            let added: [f32; 4] = if rotations == 1 { [orientation[0], orientation[1], orientation[2], orientation[3]] } else { [0.0, 0.0, 1.0, 0.0] };
            for _ in 0..spine.len() - rotations {
                orientation.extend_from_slice(&added);
            }
        }
        if orientation.len() % 4 != 0 {
            return fail("Attribute \"orientation\" in <Extrusion> must has multiple four quantity of numbers.");
        }
        if scale_values.len() < spine.len() {
            let added = if scale_values.len() == 1 { scale_values[0] } else { [1.0, 1.0] };
            let missing = spine.len() - scale_values.len();
            scale_values.extend(std::iter::repeat_n(added, missing));
        }
        let extrusion = Extrusion { begin_cap, end_cap, ccw, cross_section, orientation, scale: scale_values, spine };
        let (coord_index, vertices) = extrusion.build();
        let set = IndexedSet { coord_index, vertices, ..IndexedSet::default() };
        let element = self.create(Kind::Extrusion, def, Data::Indexed(set));
        self.finish_with_metadata(node, element, "Extrusion", depth)
    }

    /// `readIndexedFaceSet`: `coordIndex` must not be empty.
    fn read_indexed_face_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut color_per_vertex = true;
        let mut normal_per_vertex = true;
        let color_index = int_array(node, "colorIndex");
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        let coord_index = int_array(node, "coordIndex");
        let normal_index = int_array(node, "normalIndex");
        read_bool(node, "normalPerVertex", &mut normal_per_vertex);
        let tex_coord_index = int_array(node, "texCoordIndex");
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::IndexedFaceSet);
        }
        if coord_index.is_empty() {
            return fail("IndexedFaceSet must contain not empty \"coordIndex\" attribute.");
        }
        let set = IndexedSet { coord_index, color_index, normal_index, tex_coord_index, color_per_vertex, normal_per_vertex, vertices: Vec::new() };
        let element = self.create(Kind::IndexedFaceSet, def, Data::Indexed(set));
        self.finish_with_children(node, element, "IndexedFaceSet", ChildSet::Full, depth)
    }

    /// `readIndexedLineSet`.
    fn read_indexed_line_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut color_per_vertex = true;
        let color_index = int_array(node, "colorIndex");
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        let coord_index = int_array(node, "coordIndex");
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::IndexedLineSet);
        }
        if coord_index.len() < 2 || (coord_index.last() == Some(&-1) && coord_index.len() < 3) {
            return fail("IndexedLineSet must contain not empty \"coordIndex\" attribute.");
        }
        let set = IndexedSet { coord_index, color_index, color_per_vertex, ..IndexedSet::default() };
        let element = self.create(Kind::IndexedLineSet, def, Data::Indexed(set));
        self.finish_with_children(node, element, "IndexedLineSet", ChildSet::Lines, depth)
    }

    /// `readIndexedTriangleSet`, `readIndexedTriangleFanSet` and
    /// `readIndexedTriangleStripSet`: `index` made triangles.
    fn read_indexed_triangles(&mut self, node: &XmlNode, kind: Kind, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ccw = true;
        let mut color_per_vertex = true;
        let mut normal_per_vertex = true;
        read_bool(node, "ccw", &mut ccw);
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        let index = int_array(node, "index");
        read_bool(node, "normalPerVertex", &mut normal_per_vertex);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, kind);
        }
        let name = match kind {
            Kind::IndexedTriangleSet => "IndexedTriangleSet",
            Kind::IndexedTriangleFanSet => "IndexedTriangleFanSet",
            _ => "IndexedTriangleStripSet",
        };
        if index.is_empty() {
            return fail(format!("{name} must contain not empty \"index\" attribute."));
        }
        let coord_index = match kind {
            Kind::IndexedTriangleSet => triangle_list(&index, ccw),
            Kind::IndexedTriangleFanSet => triangle_fan(&index, ccw),
            _ => triangle_strip(&index, ccw),
        };
        let set = IndexedSet { coord_index, color_per_vertex, normal_per_vertex, ..IndexedSet::default() };
        let element = self.create(kind, def, Data::Indexed(set));
        self.finish_with_children(node, element, name, ChildSet::Full, depth)
    }

    /// `readLineSet`: `vertexCount` points per line, in order.
    fn read_line_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let vertex_count = int_array(node, "vertexCount");
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::LineSet);
        }
        if vertex_count.is_empty() {
            return fail("LineSet must contain not empty \"vertexCount\" attribute.");
        }
        let mut coord_index = Vec::new();
        let mut capacity = VectorCapacity::new(INDEX_SIZE);
        let mut next = 0i32;
        for &count in &vertex_count {
            if count < 2 {
                return fail("LineSet. vertexCount shall be greater than or equal to two.");
            }
            capacity.push_to(coord_index.len() + count as usize + 1)?;
            for _ in 0..count {
                coord_index.push(next);
                next = next.wrapping_add(1);
            }
            coord_index.push(-1);
        }
        let data = Data::Set { coord_index, color_per_vertex: true, normal_per_vertex: true };
        let element = self.create(Kind::LineSet, def, data);
        self.finish_with_children(node, element, "LineSet", ChildSet::Lines, depth)
    }

    /// `readPointSet`.
    fn read_point_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::PointSet);
        }
        let element = self.create(Kind::PointSet, def, Data::None);
        self.finish_with_children(node, element, "PointSet", ChildSet::Lines, depth)
    }

    /// `readTriangleFanSet`: fans of `fanCount` points, in order.
    fn read_triangle_fan_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut ccw = true;
        let mut color_per_vertex = true;
        let mut normal_per_vertex = true;
        read_bool(node, "ccw", &mut ccw);
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        let fan_count = int_array(node, "fanCount");
        read_bool(node, "normalPerVertex", &mut normal_per_vertex);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::TriangleFanSet);
        }
        if fan_count.is_empty() {
            return fail("TriangleFanSet must contain not empty \"fanCount\" attribute.");
        }
        let mut coord_index = Vec::new();
        let mut capacity = VectorCapacity::new(INDEX_SIZE);
        let mut first: usize = 0;
        let mut previous: usize = 1;
        for &count in &fan_count {
            if count < 3 {
                return fail("TriangleFanSet. fanCount shall be greater than or equal to three.");
            }
            capacity.push_to(coord_index.len() + (count as usize - 2) * 4)?;
            for _ in 2..count {
                if ccw {
                    coord_index.push(first as i32);
                    coord_index.push(previous as i32);
                    previous += 1;
                    coord_index.push(previous as i32);
                } else {
                    coord_index.push(first as i32);
                    coord_index.push((previous + 1) as i32);
                    coord_index.push(previous as i32);
                    previous += 1;
                }
                coord_index.push(-1);
            }
            previous += 1;
            first = previous;
            previous += 1;
        }
        let data = Data::Set { coord_index, color_per_vertex, normal_per_vertex };
        let element = self.create(Kind::TriangleFanSet, def, data);
        self.finish_with_children(node, element, "TriangleFanSet", ChildSet::Full, depth)
    }

    /// `readTriangleSet`.
    fn read_triangle_set(&mut self, node: &XmlNode, depth: usize) -> Result<(), ImportError> {
        let (def, usage) = def_use(node);
        let mut color_per_vertex = true;
        read_bool(node, "colorPerVertex", &mut color_per_vertex);
        if !usage.is_empty() {
            return self.use_element(node, &def, &usage, Kind::TriangleSet);
        }
        let element = self.create(Kind::TriangleSet, def, Data::TriangleSet { color_per_vertex });
        self.finish_with_children(node, element, "TriangleSet", ChildSet::Full, depth)
    }
}

/// `readIndexedTriangleSet`'s triangles: each three indices (a -1 one of
/// them, a short end dropped).
fn triangle_list(index: &[i32], ccw: bool) -> Vec<i32> {
    let mut coord_index = Vec::new();
    for triangle in index.as_chunks::<3>().0 {
        let ordered = if ccw { [triangle[0], triangle[1], triangle[2]] } else { [triangle[0], triangle[2], triangle[1]] };
        coord_index.extend_from_slice(&ordered);
        coord_index.push(-1);
    }
    coord_index
}

/// `readIndexedTriangleFanSet`'s triangles: fans split at negative
/// indices around their first.
fn triangle_fan(index: &[i32], ccw: bool) -> Vec<i32> {
    let mut coord_index = Vec::new();
    let mut counter = 0;
    let mut corners = [0i32; 3];
    for &value in index {
        corners[2] = value;
        if value < 0 {
            counter = 0;
            continue;
        }
        if counter >= 2 {
            let ordered = if ccw { [corners[0], corners[1], corners[2]] } else { [corners[0], corners[2], corners[1]] };
            coord_index.extend_from_slice(&ordered);
            coord_index.push(-1);
            corners[1] = corners[2];
        } else {
            corners[counter] = corners[2];
        }
        counter += 1;
    }
    coord_index
}

/// `readIndexedTriangleStripSet`'s triangles: strips split at negative
/// indices (every triangle wound the same way).
fn triangle_strip(index: &[i32], ccw: bool) -> Vec<i32> {
    let mut coord_index = Vec::new();
    let mut counter = 0usize;
    let mut corners = [0i32; 3];
    for &value in index {
        corners[2] = value;
        if value < 0 {
            counter = 0;
            continue;
        }
        if counter >= 2 {
            let ordered = if ccw { [corners[0], corners[1], corners[2]] } else { [corners[0], corners[2], corners[1]] };
            coord_index.extend_from_slice(&ordered);
            coord_index.push(-1);
        }
        corners[counter & 1] = corners[2];
        counter += 1;
    }
    coord_index
}

/// `"/../"` folded into the folder before it, as `readInline` does.
fn fold_parent_directories(path: &mut Vec<u8>) {
    const PARENT: &[u8] = b"/../";
    let find = |path: &[u8], from: usize| path.get(from..).and_then(|rest| rest.windows(PARENT.len()).position(|window| window == PARENT)).map(|offset| from + offset);
    let mut position = find(path, 0);
    while let Some(mut found) = position {
        if found > 0 {
            match path[..found].iter().rposition(|&byte| byte == b'/') {
                Some(slash) => {
                    path.drain(slash..found + 3);
                    found = slash;
                }
                None => {
                    path.drain(..found + 4);
                    found = 0;
                }
            }
        } else {
            found += 3;
        }
        position = find(path, found);
    }
}

/// Building the meshes from the graph (`Postprocess_*`).
impl Reader<'_> {
    /// `PostprocessHelper_CollectMetadata` over a node's children: metadata
    /// sets are searched through (a cycle never ends).
    fn walk_metadata(&mut self, element: usize, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return undefined("X3D metadata sets nested deeper than the stack (a USE cycle)");
        }
        let children = self.elements[element].children.clone();
        for child in children {
            self.step()?;
            if self.elements[child].kind == Kind::MetaSet {
                self.walk_metadata(child, depth + 1)?;
            }
        }
        Ok(())
    }

    /// `Postprocess_BuildNode`: groups, shapes and lights below a group
    /// (only its chosen child for a switch).
    fn build_node(&mut self, element: usize, depth: usize, meshes: &mut Vec<Mesh>) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return undefined("an X3D node graph deeper than the stack (a USE cycle)");
        }
        self.walk_metadata(element, 0)?;
        let children = self.elements[element].children.clone();
        let mut range = 0..children.len();
        if let Data::Group { choice: Some(choice), .. } = self.elements[element].data {
            range = if choice < 0 || choice as usize >= children.len() { 0..0 } else { choice as usize..choice as usize + 1 };
        }
        for &child in &children[range] {
            self.step()?;
            let kind = self.elements[child].kind;
            if kind == Kind::Group {
                self.build_node(child, depth + 1, meshes)?;
            } else if kind == Kind::Shape {
                self.build_shape(child, meshes)?;
            } else if !kind.is_light() && !kind.is_metadata() {
                return fail(format!("Postprocess_BuildNode. Unknown type: {}.", kind as u32));
            }
        }
        Ok(())
    }

    /// `Postprocess_BuildShape`: one mesh and one appearance at most.
    fn build_shape(&mut self, shape: usize, meshes: &mut Vec<Mesh>) -> Result<(), ImportError> {
        let mut has_mesh = false;
        let mut has_material = false;
        let children = self.elements[shape].children.clone();
        for child in children {
            let kind = self.elements[child].kind;
            if kind.is_mesh() {
                if has_mesh {
                    return fail("Postprocess_BuildMesh. *pMesh must be nullptr.");
                }
                if let Some(mesh) = self.build_mesh(child)? {
                    meshes.push(mesh);
                    has_mesh = true;
                }
            } else if kind == Kind::Appearance {
                if has_material {
                    return fail("Postprocess_BuildMaterial. *pMaterial must be nullptr.");
                }
                has_material = true;
            }
        }
        Ok(())
    }

    /// The mesh a set's `Coordinate` children make (the last of them).
    fn coordinate_mesh(&self, element: usize, make: impl Fn(&[Vector3]) -> Result<Option<Mesh>, ImportError>) -> Result<Option<Mesh>, ImportError> {
        let mut mesh = None;
        for &child in &self.elements[element].children {
            if let (Kind::Coordinate, Data::Points(points)) = (self.elements[child].kind, &self.elements[child].data) {
                mesh = make(points)?;
            }
        }
        Ok(mesh)
    }

    fn unknown_child<T>(&self, element: usize, child: usize) -> Result<T, ImportError> {
        let name = kind_name(self.elements[element].kind);
        fail(format!("Postprocess_BuildMesh. Unknown child of {name}: {}.", self.elements[child].kind as u32))
    }

    /// `Postprocess_BuildMesh`.
    fn build_mesh(&self, element: usize) -> Result<Option<Mesh>, ImportError> {
        let kind = self.elements[element].kind;
        let children = &self.elements[element].children;
        match &self.elements[element].data {
            Data::Shape { vertices, index_count } => shapes::make_mesh(vertices, *index_count as u32),
            Data::Grid { vertices, coord_index, color_per_vertex, normal_per_vertex } => {
                let mut mesh = geometry::make_mesh(coord_index, vertices)?;
                for &child in children {
                    match (self.elements[child].kind, &self.elements[child].data) {
                        (Kind::Color | Kind::ColorRgba, Data::Colors(colors)) => geometry::add_color(&mut mesh, colors, *color_per_vertex)?,
                        (Kind::Normal, Data::Points(normals)) => geometry::add_normal(&mut mesh, normals, *normal_per_vertex)?,
                        (Kind::TextureCoordinate, Data::TexCoords(coordinates)) => geometry::add_tex_coord(&mut mesh, coordinates)?,
                        _ => return self.unknown_child(element, child),
                    }
                }
                Ok(Some(mesh))
            }
            Data::Indexed(set) if kind == Kind::Extrusion => Ok(Some(geometry::make_mesh(&set.coord_index, &set.vertices)?)),
            Data::Indexed(set) => {
                let lines = kind == Kind::IndexedLineSet;
                let mut mesh = self.coordinate_mesh(element, |points| {
                    let built = if lines { geometry::make_line_mesh(&set.coord_index, points)? } else { geometry::make_mesh(&set.coord_index, points)? };
                    Ok(Some(built))
                })?;
                for &child in children {
                    let child_kind = self.elements[child].kind;
                    if child_kind == Kind::Coordinate {
                        continue;
                    }
                    let takes = matches!(child_kind, Kind::Color | Kind::ColorRgba) || (!lines && matches!(child_kind, Kind::Normal | Kind::TextureCoordinate));
                    if !takes {
                        return self.unknown_child(element, child);
                    }
                    let Some(mesh) = mesh.as_mut() else {
                        return undefined("an X3D set's mesh used before coordinates made it");
                    };
                    match &self.elements[child].data {
                        Data::Colors(colors) => geometry::add_indexed_color(mesh, &set.coord_index, &set.color_index, colors, set.color_per_vertex)?,
                        Data::Points(normals) => geometry::add_indexed_normal(mesh, &set.coord_index, &set.normal_index, normals, set.normal_per_vertex)?,
                        Data::TexCoords(coordinates) => geometry::add_indexed_tex_coord(mesh, &set.coord_index, &set.tex_coord_index, coordinates)?,
                        _ => return self.unknown_child(element, child),
                    }
                }
                Ok(mesh)
            }
            Data::Set { coord_index, color_per_vertex, normal_per_vertex } => {
                let fans = kind == Kind::TriangleFanSet;
                let mut mesh = self.coordinate_mesh(element, |points| Ok(Some(geometry::make_mesh(coord_index, points)?)))?;
                for &child in children {
                    // A fan set stops at a missing mesh; a line set uses it.
                    if fans && mesh.is_none() {
                        break;
                    }
                    let child_kind = self.elements[child].kind;
                    if child_kind == Kind::Coordinate {
                        continue;
                    }
                    let takes = matches!(child_kind, Kind::Color | Kind::ColorRgba) || (fans && matches!(child_kind, Kind::Normal | Kind::TextureCoordinate));
                    if !takes {
                        return self.unknown_child(element, child);
                    }
                    let Some(mesh) = mesh.as_mut() else {
                        return undefined("an X3D set's mesh used before coordinates made it");
                    };
                    match &self.elements[child].data {
                        Data::Colors(colors) => geometry::add_color(mesh, colors, if fans { *color_per_vertex } else { true })?,
                        Data::Points(normals) => geometry::add_indexed_normal(mesh, coord_index, &[], normals, *normal_per_vertex)?,
                        Data::TexCoords(coordinates) => geometry::add_indexed_tex_coord(mesh, coord_index, &[], coordinates)?,
                        _ => return self.unknown_child(element, child),
                    }
                }
                Ok(mesh)
            }
            Data::TriangleSet { color_per_vertex } => {
                let mut mesh = self.coordinate_mesh(element, |points| shapes::make_mesh(points, 3))?;
                for &child in children {
                    match (self.elements[child].kind, &self.elements[child].data) {
                        (Kind::Coordinate, _) => {}
                        (Kind::Color | Kind::ColorRgba, Data::Colors(colors)) => {
                            let Some(mesh) = mesh.as_mut() else {
                                return undefined("colours added to a TriangleSet without coordinates");
                            };
                            geometry::add_color(mesh, colors, *color_per_vertex)?;
                        }
                        (Kind::Normal | Kind::TextureCoordinate, _) => {
                            return undefined("a TriangleSet's index lists read from the wrong fields");
                        }
                        _ => return self.unknown_child(element, child),
                    }
                }
                Ok(mesh)
            }
            Data::None if kind == Kind::PointSet => {
                let mut mesh = self.coordinate_mesh(element, |points| shapes::make_mesh(points, 1))?;
                for &child in children {
                    match (self.elements[child].kind, &self.elements[child].data) {
                        (Kind::Coordinate, _) => {}
                        (Kind::Color | Kind::ColorRgba, Data::Colors(colors)) => {
                            let Some(mesh) = mesh.as_mut() else {
                                return undefined("colours added to a PointSet without points");
                            };
                            geometry::add_color(mesh, colors, true)?;
                        }
                        _ => return self.unknown_child(element, child),
                    }
                }
                Ok(mesh)
            }
            _ => fail(format!("Postprocess_BuildMesh. Unknown mesh type: {}.", kind as u32)),
        }
    }
}

/// A geometry element's node name, for messages.
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::ElevationGrid => "ElevationGrid",
        Kind::IndexedFaceSet => "IndexedFaceSet",
        Kind::IndexedLineSet => "IndexedLineSet",
        Kind::IndexedTriangleSet | Kind::IndexedTriangleFanSet | Kind::IndexedTriangleStripSet => {
            "IndexedTriangleSet or IndexedTriangleFanSet, or IndexedTriangleStripSet"
        }
        Kind::PointSet => "PointSet",
        Kind::LineSet => "LineSet",
        Kind::TriangleFanSet => "TrianlgeFanSet",
        Kind::TriangleSet => "TrianlgeSet",
        _ => "TriangleStripSet",
    }
}

/// `X3DImporter::InternReadFile`: the file's folder pushed for inlined
/// files, the file read, the folder popped; the graph's root found from
/// the first element listed (an empty list is read before its start),
/// and the meshes built.
pub(crate) fn read_graph(input: &FileInput) -> Result<LoadedScene, ImportError> {
    let files = &input.files;
    let path = input.path.as_bytes();
    let directory = match path.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
        Some(slash) => path[..=slash].to_vec(),
        None => Vec::new(),
    };
    files.push_directory(&directory);
    let mut reader = Reader { files, elements: Vec::new(), list: Vec::new(), current: None, open_files: Vec::new(), steps: 0 };
    if let Some(identity) = FileSystem::identify(path) {
        reader.open_files.push(identity);
    }
    reader.parse_document(input.bytes, 0)?;
    files.pop_directory();
    let Some(&first) = reader.list.first() else {
        return undefined("the first element of an empty X3D element list");
    };
    let mut root = first;
    while let Some(parent) = reader.elements[root].parent {
        root = parent;
    }
    let mut meshes = Vec::new();
    reader.build_node(root, 0, &mut meshes)?;
    Ok(LoadedScene { meshes, graph: Graph::Unknown, animations: Some(0) })
}
