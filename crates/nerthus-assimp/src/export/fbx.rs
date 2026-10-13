//! `FBXExporter::ExportBinary` (FBX 7.5) for the scenes the mesh exports
//! make.
//!
//! An FBX file is a tree of nodes: each a header (the absolute offset of
//! its end, its property count and their byte length), a name, typed
//! properties, then child nodes closed by a null record. Assimp writes the
//! header extension (stamped with the local time), Blender's generic file
//! id and creation time, global settings, one document, the object types
//! with their property templates, the objects — a geometry per mesh, a
//! Lambert material per material, a "Mesh" model for each of the root
//! node's meshes — and the connections between them.
//!
//! Geometry keeps its vertices as they are (the export enforces no
//! JoinIdenticalVertices, so `bJoinIdenticalVertices` is false), polygons
//! end with −1 − their last index, normals are per corner and UVs are
//! numbered by first use.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{Datelike, Timelike};

use super::{export_time, ExportError, ExportMaterial, ExportScene, VERSION_MAJOR, VERSION_MINOR, VERSION_REVISION};
use crate::scene::Mesh;

/// FBX 7.5, times 1000.
const EXPORT_VERSION: u32 = 7500;
/// The first 23 bytes of a binary file.
const BINARY_HEADER: &[u8; 23] = b"Kaydara FBX Binary\x20\x20\x00\x1a\x00";
/// The file id and creation time Blender writes, which Assimp copies (the
/// file has hashes that depend on them).
const GENERIC_FILE_ID: &[u8; 16] = b"\x28\xb3\x2a\xeb\xb6\x24\xcc\xc2\xbf\xc8\xb0\x2a\xa9\x2b\xfc\xf1";
const GENERIC_CREATION_TIME: &str = "1970-01-01 10:00:00:000";
const GENERIC_FOOT_ID: &[u8; 16] = b"\xfa\xbc\xab\x09\xd0\xc8\xd4\x66\xb1\x76\xfb\x83\x1c\xf7\x26\x7e";
const FOOT_MAGIC: &[u8; 16] = b"\xf8\x5a\x8c\x6a\xde\xf5\xd9\x7e\xec\xe9\x0c\xe3\x75\x8f\x29\x0b";
/// `FBX::SEPARATOR`, between an object's name and its class.
const SEPARATOR: &str = "\x00\x01";
/// One second in FBX time (`FBX::SECOND`).
const SECOND: i64 = 46186158000;
/// What closes a node's children, and the top-level list (64-bit files).
const NULL_RECORD: [u8; 25] = [0; 25];
/// `generate_uid`'s counter before its first uid.
const LAST_UID: i64 = 999999;
/// `TransformInheritance_RSrs`, the inherit type of every model.
const INHERIT_RSRS: i32 = 1;

/// `FBXExportProperty`: one typed value.
enum Property {
    /// 'C'.
    Bool(bool),
    /// 'I'.
    Int(i32),
    /// 'L'.
    Long(i64),
    /// 'D'.
    Double(f64),
    /// 'S'.
    String(Vec<u8>),
    /// 'R'.
    Raw(Vec<u8>),
    /// 'i'.
    Ints(Vec<i32>),
}

impl Property {
    fn string(text: &str) -> Property {
        Property::String(text.as_bytes().to_vec())
    }

    /// `FBXExportProperty::DumpBinary`.
    fn write(&self, bytes: &mut Vec<u8>) {
        match self {
            Property::Bool(value) => {
                bytes.push(b'C');
                bytes.push(u8::from(*value));
            }
            Property::Int(value) => {
                bytes.push(b'I');
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            Property::Long(value) => {
                bytes.push(b'L');
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            Property::Double(value) => {
                bytes.push(b'D');
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            Property::String(data) => {
                bytes.push(b'S');
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            Property::Raw(data) => {
                bytes.push(b'R');
                bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
                bytes.extend_from_slice(data);
            }
            Property::Ints(values) => {
                let header = array_header(b'i', values.len(), 4);
                bytes.extend_from_slice(&header);
                for value in values {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}

/// An array property's type, element count, encoding (0: not compressed)
/// and data size (a 32-bit product, as Assimp computes it).
fn array_header(kind: u8, count: usize, element_size: u32) -> [u8; 13] {
    let count = count as u32;
    let data_size = count.wrapping_mul(element_size);
    let mut header = [0u8; 13];
    header[0] = kind;
    header[1..5].copy_from_slice(&count.to_le_bytes());
    header[9..13].copy_from_slice(&data_size.to_le_bytes());
    header
}

/// `BeginBinary`, `DumpPropertiesBinary` and `EndPropertiesBinary`: the end
/// offset left for later, the property count and length (both left zero
/// without properties), the name and the properties.
fn write_header(bytes: &mut Vec<u8>, name: &str, properties: &[Property]) {
    let start = bytes.len();
    bytes.extend_from_slice(&[0; 24]);
    bytes.push(name.len() as u8);
    bytes.extend_from_slice(name.as_bytes());
    let property_start = bytes.len();
    for property in properties {
        property.write(bytes);
    }
    if !properties.is_empty() {
        let count = properties.len() as u64;
        let length = (bytes.len() - property_start) as u64;
        bytes[start + 8..start + 16].copy_from_slice(&count.to_le_bytes());
        bytes[start + 16..start + 24].copy_from_slice(&length.to_le_bytes());
    }
}

/// `FBX::Node`, for the nodes built whole before they are written.
struct Node {
    name: &'static str,
    properties: Vec<Property>,
    children: Vec<Node>,
    force_has_children: bool,
}

impl Node {
    fn new(name: &'static str, properties: Vec<Property>) -> Node {
        Node { name, properties, children: Vec::new(), force_has_children: false }
    }

    /// `AddChild(name, value)`: a child holding one property.
    fn add(&mut self, name: &'static str, property: Property) {
        let child = Node::new(name, vec![property]);
        self.children.push(child);
    }

    /// `DumpBinary`, with `base` the file offset `bytes` starts at.
    fn write(&self, bytes: &mut Vec<u8>, base: u64) {
        let start = bytes.len();
        write_header(bytes, self.name, &self.properties);
        for child in &self.children {
            child.write(bytes, base);
        }
        if self.force_has_children || !self.children.is_empty() {
            bytes.extend_from_slice(&NULL_RECORD);
        }
        let end = base + bytes.len() as u64;
        bytes[start..start + 8].copy_from_slice(&end.to_le_bytes());
    }
}

/// A "Properties70" node holding `entries`.
fn properties70(entries: Vec<Node>) -> Node {
    let mut node = Node::new("Properties70", Vec::new());
    node.children = entries;
    node
}

/// `AddP70`: a "P" node with the property's name, type, label and flags,
/// then its values.
fn p70(name: &str, kind: &str, label: &str, flags: &str, values: Vec<Property>) -> Node {
    let mut properties = vec![Property::string(name), Property::string(kind), Property::string(label), Property::string(flags)];
    properties.extend(values);
    Node::new("P", properties)
}

fn doubles(values: [f64; 3]) -> Vec<Property> {
    values.into_iter().map(Property::Double).collect()
}

fn p70_int(name: &str, value: i32) -> Node {
    p70(name, "int", "Integer", "", vec![Property::Int(value)])
}

fn p70_bool(name: &str, value: bool) -> Node {
    p70(name, "bool", "", "", vec![Property::Int(i32::from(value))])
}

fn p70_double(name: &str, value: f64) -> Node {
    p70(name, "double", "Number", "", vec![Property::Double(value)])
}

fn p70_number_a(name: &str, value: f64) -> Node {
    p70(name, "Number", "", "A", vec![Property::Double(value)])
}

fn p70_color(name: &str, color: [f64; 3]) -> Node {
    p70(name, "ColorRGB", "Color", "", doubles(color))
}

fn p70_color_a(name: &str, color: [f64; 3]) -> Node {
    p70(name, "Color", "", "A", doubles(color))
}

fn p70_vector(name: &str, vector: [f64; 3]) -> Node {
    p70(name, "Vector3D", "Vector", "", doubles(vector))
}

fn p70_string(name: &str, value: &str) -> Node {
    p70(name, "KString", "", "", vec![Property::string(value)])
}

fn p70_enum(name: &str, value: i32) -> Node {
    p70(name, "enum", "", "", vec![Property::Int(value)])
}

fn p70_time(name: &str, value: i64) -> Node {
    p70(name, "KTime", "Time", "", vec![Property::Long(value)])
}

/// An "ObjectType" definition: how many objects of the type there are and
/// the template their properties default to, if it has one.
fn object_type(name: &str, count: i32, template: Option<(&str, Vec<Node>)>) -> Node {
    let mut node = Node::new("ObjectType", vec![Property::string(name)]);
    node.add("Count", Property::Int(count));
    if let Some((template_name, entries)) = template {
        let mut property_template = Node::new("PropertyTemplate", vec![Property::string(template_name)]);
        property_template.children.push(properties70(entries));
        node.children.push(property_template);
    }
    node
}

/// The "FbxSkeleton" template of the node attributes.
fn skeleton_template() -> Vec<Node> {
    vec![
        p70_color("Color", [0.8, 0.8, 0.8]),
        p70_double("Size", 33.333333333333),
        p70("LimbLength", "double", "Number", "H", vec![Property::Double(1.0)]),
    ]
}

/// The "FbxNode" template of the models.
fn model_template() -> Vec<Node> {
    let zero = [0.0; 3];
    let one = [1.0; 3];
    vec![
        p70_enum("QuaternionInterpolate", 0),
        p70_vector("RotationOffset", zero),
        p70_vector("RotationPivot", zero),
        p70_vector("ScalingOffset", zero),
        p70_vector("ScalingPivot", zero),
        p70_bool("TranslationActive", false),
        p70_vector("TranslationMin", zero),
        p70_vector("TranslationMax", zero),
        p70_bool("TranslationMinX", false),
        p70_bool("TranslationMinY", false),
        p70_bool("TranslationMinZ", false),
        p70_bool("TranslationMaxX", false),
        p70_bool("TranslationMaxY", false),
        p70_bool("TranslationMaxZ", false),
        p70_enum("RotationOrder", 0),
        p70_bool("RotationSpaceForLimitOnly", false),
        p70_double("RotationStiffnessX", 0.0),
        p70_double("RotationStiffnessY", 0.0),
        p70_double("RotationStiffnessZ", 0.0),
        p70_double("AxisLen", 10.0),
        p70_vector("PreRotation", zero),
        p70_vector("PostRotation", zero),
        p70_bool("RotationActive", false),
        p70_vector("RotationMin", zero),
        p70_vector("RotationMax", zero),
        p70_bool("RotationMinX", false),
        p70_bool("RotationMinY", false),
        p70_bool("RotationMinZ", false),
        p70_bool("RotationMaxX", false),
        p70_bool("RotationMaxY", false),
        p70_bool("RotationMaxZ", false),
        p70_enum("InheritType", 0),
        p70_bool("ScalingActive", false),
        p70_vector("ScalingMin", zero),
        p70_vector("ScalingMax", one),
        p70_bool("ScalingMinX", false),
        p70_bool("ScalingMinY", false),
        p70_bool("ScalingMinZ", false),
        p70_bool("ScalingMaxX", false),
        p70_bool("ScalingMaxY", false),
        p70_bool("ScalingMaxZ", false),
        p70_vector("GeometricTranslation", zero),
        p70_vector("GeometricRotation", zero),
        p70_vector("GeometricScaling", one),
        p70_double("MinDampRangeX", 0.0),
        p70_double("MinDampRangeY", 0.0),
        p70_double("MinDampRangeZ", 0.0),
        p70_double("MaxDampRangeX", 0.0),
        p70_double("MaxDampRangeY", 0.0),
        p70_double("MaxDampRangeZ", 0.0),
        p70_double("MinDampStrengthX", 0.0),
        p70_double("MinDampStrengthY", 0.0),
        p70_double("MinDampStrengthZ", 0.0),
        p70_double("MaxDampStrengthX", 0.0),
        p70_double("MaxDampStrengthY", 0.0),
        p70_double("MaxDampStrengthZ", 0.0),
        p70_double("PreferedAngleX", 0.0),
        p70_double("PreferedAngleY", 0.0),
        p70_double("PreferedAngleZ", 0.0),
        p70("LookAtProperty", "object", "", "", Vec::new()),
        p70("UpVectorProperty", "object", "", "", Vec::new()),
        p70_bool("Show", true),
        p70_bool("NegativePercentShapeSupport", true),
        p70_int("DefaultAttributeIndex", -1),
        p70_bool("Freeze", false),
        p70_bool("LODBox", false),
        p70("Lcl Translation", "Lcl Translation", "", "A", doubles(zero)),
        p70("Lcl Rotation", "Lcl Rotation", "", "A", doubles(zero)),
        p70("Lcl Scaling", "Lcl Scaling", "", "A", doubles(one)),
        p70("Visibility", "Visibility", "", "A", vec![Property::Double(1.0)]),
        p70("Visibility Inheritance", "Visibility Inheritance", "", "", vec![Property::Int(1)]),
    ]
}

/// The "FbxMesh" template of the geometries.
fn mesh_template() -> Vec<Node> {
    let zero = [0.0; 3];
    vec![
        p70_color("Color", zero),
        p70_vector("BBoxMin", zero),
        p70_vector("BBoxMax", zero),
        p70_bool("Primary Visibility", true),
        p70_bool("Casts Shadows", true),
        p70_bool("Receive Shadows", true),
    ]
}

/// The "FbxSurfaceLambert" template of the materials (none of the exported
/// ones has a shininess, which would make it Phong).
fn lambert_template() -> Vec<Node> {
    let zero = [0.0; 3];
    vec![
        p70_string("ShadingModel", "Lambert"),
        p70_bool("MultiLayer", false),
        p70_color_a("EmissiveColor", zero),
        p70_number_a("EmissiveFactor", 1.0),
        p70_color_a("AmbientColor", [0.2, 0.2, 0.2]),
        p70_number_a("AmbientFactor", 1.0),
        p70_color_a("DiffuseColor", [0.8, 0.8, 0.8]),
        p70_number_a("DiffuseFactor", 1.0),
        p70_vector("Bump", zero),
        p70_vector("NormalMap", zero),
        p70_double("BumpFactor", 1.0),
        p70_color_a("TransparentColor", zero),
        p70_number_a("TransparencyFactor", 0.0),
        p70_color("DisplacementColor", zero),
        p70_double("DisplacementFactor", 1.0),
        p70_color("VectorDisplacementColor", zero),
        p70_double("VectorDisplacementFactor", 1.0),
    ]
}

/// A geometry layer's reference to one of its elements.
fn layer_element(kind: &str, index: i32) -> Node {
    let mut node = Node::new("LayerElement", Vec::new());
    node.add("Type", Property::string(kind));
    node.add("TypedIndex", Property::Int(index));
    node
}

/// The file, written through a buffer, with seeks back to fill in the end
/// offsets of nodes whose children are written as they are made.
struct Output {
    file: BufWriter<File>,
    position: u64,
}

impl Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.position += bytes.len() as u64;
        Ok(())
    }

    /// `Node::Dump` for a node built whole.
    fn node(&mut self, node: &Node) -> io::Result<()> {
        let mut bytes = Vec::new();
        node.write(&mut bytes, self.position);
        self.write(&bytes)
    }

    /// `WritePropertyNode`: a node holding one property.
    fn property(&mut self, name: &'static str, property: Property) -> io::Result<()> {
        let node = Node::new(name, vec![property]);
        self.node(&node)
    }

    /// `Begin` to `EndProperties` of a node whose children follow; returns
    /// where it starts, for `end`.
    fn begin(&mut self, name: &str, properties: &[Property]) -> io::Result<u64> {
        let start = self.position;
        let mut bytes = Vec::new();
        write_header(&mut bytes, name, properties);
        self.write(&bytes)?;
        Ok(start)
    }

    /// `Node::End`: the null record if there were children, then the end
    /// offset filled in at the start.
    fn end(&mut self, start: u64, has_children: bool) -> io::Result<()> {
        if has_children {
            self.write(&NULL_RECORD)?;
        }
        let end = self.position;
        self.file.seek(SeekFrom::Start(start))?;
        self.file.write_all(&end.to_le_bytes())?;
        self.file.seek(SeekFrom::Start(end))?;
        Ok(())
    }

    /// `WritePropertyNodeBinary`'s header for a node holding one array of
    /// `count` elements of `element_size` bytes, which the caller then
    /// writes.
    fn begin_array(&mut self, name: &str, kind: u8, count: usize, element_size: u32) -> io::Result<()> {
        let property_length = 13 + count as u64 * u64::from(element_size);
        let end = self.position + 25 + name.len() as u64 + property_length;
        let mut bytes = Vec::with_capacity(25 + name.len() + 13);
        bytes.extend_from_slice(&end.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&property_length.to_le_bytes());
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        let header = array_header(kind, count, element_size);
        bytes.extend_from_slice(&header);
        self.write(&bytes)
    }

    fn double(&mut self, value: f64) -> io::Result<()> {
        self.write(&value.to_le_bytes())
    }

    fn int(&mut self, value: i32) -> io::Result<()> {
        self.write(&value.to_le_bytes())
    }
}

/// The `FBXExporter` state: uids handed out in writing order from
/// 1000000, the objects' uids and the connections, written last.
struct Exporter<'a> {
    scene: &'a ExportScene,
    output: Output,
    last_uid: i64,
    mesh_uids: Vec<i64>,
    material_uids: Vec<i64>,
    connections: Vec<Node>,
}

impl Exporter<'_> {
    fn generate_uid(&mut self) -> i64 {
        self.last_uid += 1;
        self.last_uid
    }

    /// An object-object connection of `child` to `parent`.
    fn connect(&mut self, child: i64, parent: i64) {
        let properties = vec![Property::string("OO"), Property::Long(child), Property::Long(parent)];
        let connection = Node::new("C", properties);
        self.connections.push(connection);
    }

    fn export(&mut self) -> io::Result<()> {
        self.output.write(BINARY_HEADER)?;
        self.output.write(&EXPORT_VERSION.to_le_bytes())?;

        self.write_header_extension()?;
        self.write_global_settings()?;
        self.write_documents()?;
        self.write_references()?;
        self.write_definitions()?;
        self.write_objects()?;
        self.write_connections()?;

        self.write_footer()?;
        self.output.file.flush()
    }

    fn write_header_extension(&mut self) -> io::Result<()> {
        let time = export_time();
        let mut timestamp = Node::new("CreationTimeStamp", Vec::new());
        timestamp.add("Version", Property::Int(1000));
        timestamp.add("Year", Property::Int(time.year()));
        timestamp.add("Month", Property::Int(time.month() as i32));
        timestamp.add("Day", Property::Int(time.day() as i32));
        timestamp.add("Hour", Property::Int(time.hour() as i32));
        timestamp.add("Minute", Property::Int(time.minute() as i32));
        timestamp.add("Second", Property::Int(time.second() as i32));
        timestamp.add("Millisecond", Property::Int(0));

        let creator = format!("Open Asset Import Library (Assimp) {VERSION_MAJOR}.{VERSION_MINOR}.{VERSION_REVISION}");
        let mut extension = Node::new("FBXHeaderExtension", Vec::new());
        extension.add("FBXHeaderVersion", Property::Int(1003));
        extension.add("FBXVersion", Property::Int(EXPORT_VERSION as i32));
        extension.add("EncryptionType", Property::Int(0));
        extension.children.push(timestamp);
        extension.add("Creator", Property::string(&creator));
        self.output.node(&extension)?;

        // Binary files repeat these at the top level.
        self.output.property("FileId", Property::Raw(GENERIC_FILE_ID.to_vec()))?;
        self.output.property("CreationTime", Property::string(GENERIC_CREATION_TIME))?;
        self.output.property("Creator", Property::string(&creator))
    }

    /// The scene has no metadata, so every setting is the default.
    fn write_global_settings(&mut self) -> io::Result<()> {
        let entries = vec![
            p70_int("UpAxis", 1),
            p70_int("UpAxisSign", 1),
            p70_int("FrontAxis", 2),
            p70_int("FrontAxisSign", 1),
            p70_int("CoordAxis", 0),
            p70_int("CoordAxisSign", 1),
            p70_int("OriginalUpAxis", 1),
            p70_int("OriginalUpAxisSign", 1),
            p70_double("UnitScaleFactor", 1.0),
            p70_double("OriginalUnitScaleFactor", 1.0),
            p70_color("AmbientColor", [0.0; 3]),
            p70_string("DefaultCamera", "Producer Perspective"),
            p70_enum("TimeMode", 11),
            p70_enum("TimeProtocol", 2),
            p70_enum("SnapOnFrameMode", 0),
            p70_time("TimeSpanStart", 0),
            p70_time("TimeSpanStop", SECOND),
            p70_double("CustomFrameRate", -1.0),
            p70("TimeMarker", "Compound", "", "", Vec::new()),
            p70_int("CurrentTimeMarker", -1),
        ];
        let mut settings = Node::new("GlobalSettings", Vec::new());
        settings.add("Version", Property::Int(1000));
        settings.children.push(properties70(entries));
        self.output.node(&settings)
    }

    fn write_documents(&mut self) -> io::Result<()> {
        let uid = self.generate_uid();
        let entries = vec![p70("SourceObject", "object", "", "", Vec::new()), p70_string("ActiveAnimStackName", "")];
        let properties = vec![Property::Long(uid), Property::string(""), Property::string("Scene")];
        let mut document = Node::new("Document", properties);
        document.children.push(properties70(entries));
        // The root node's uid is always 0.
        document.add("RootNode", Property::Long(0));

        let mut documents = Node::new("Documents", Vec::new());
        documents.add("Count", Property::Int(1));
        documents.children.push(document);
        self.output.node(&documents)
    }

    fn write_references(&mut self) -> io::Result<()> {
        let mut references = Node::new("References", Vec::new());
        references.force_has_children = true;
        self.output.node(&references)
    }

    /// The object types and their counts: the scene has no animations,
    /// textures, bones or blend shapes, and its root node holds the meshes
    /// (it is no model itself).
    fn write_definitions(&mut self) -> io::Result<()> {
        let scene = self.scene;
        let mut object_types = vec![object_type("GlobalSettings", 1, None)];
        let mut total_count = 1;

        // Always one node attribute (Assimp does not count them).
        let skeleton = skeleton_template();
        object_types.push(object_type("NodeAttribute", 1, Some(("FbxSkeleton", skeleton))));
        total_count += 1;

        let model_count = scene.root_meshes.len() as i32;
        if model_count != 0 {
            let template = model_template();
            object_types.push(object_type("Model", model_count, Some(("FbxNode", template))));
            total_count += model_count;
        }

        let geometry_count = scene.meshes.len() as i32;
        if geometry_count != 0 {
            let template = mesh_template();
            object_types.push(object_type("Geometry", geometry_count, Some(("FbxMesh", template))));
            total_count += geometry_count;
        }

        let material_count = scene.materials.len() as i32;
        if material_count != 0 {
            let template = lambert_template();
            object_types.push(object_type("Material", material_count, Some(("FbxSurfaceLambert", template))));
            total_count += material_count;
        }

        let mut definitions = Node::new("Definitions", Vec::new());
        definitions.add("Version", Property::Int(100));
        definitions.add("Count", Property::Int(total_count));
        definitions.children.extend(object_types);
        self.output.node(&definitions)
    }

    fn write_objects(&mut self) -> io::Result<()> {
        let scene = self.scene;
        let objects = self.output.begin("Objects", &[])?;
        for mesh in &scene.meshes {
            self.write_geometry(mesh)?;
        }
        for material in &scene.materials {
            self.write_material(material)?;
        }
        self.write_model_nodes()?;
        self.output.end(objects, true)
    }

    fn write_geometry(&mut self, mesh: &Mesh) -> io::Result<()> {
        let uid = self.generate_uid();
        self.mesh_uids.push(uid);
        let class = format!("{SEPARATOR}Geometry");
        let properties = [Property::Long(uid), Property::string(&class), Property::string("Mesh")];
        let geometry = self.output.begin("Geometry", &properties)?;

        let vertices = mesh.vertices.as_deref().unwrap_or(&[]);
        let faces = mesh.faces.as_deref().unwrap_or(&[]);
        let corner_count: usize = faces.iter().map(Vec::len).sum();

        self.output.begin_array("Vertices", b'd', vertices.len() * 3, 8)?;
        for vertex in vertices {
            for &value in vertex {
                self.output.double(f64::from(value))?;
            }
        }

        // Each polygon's corners, the last as −1 − index.
        self.output.begin_array("PolygonVertexIndex", b'i', corner_count, 4)?;
        for face in faces {
            if let Some((&last, others)) = face.split_last() {
                for &index in others {
                    self.output.int(index as i32)?;
                }
                self.output.int(-1 - last as i32)?;
            }
        }

        self.output.property("GeometryVersion", Property::Int(124))?;

        if mesh.has_normals() {
            let normals = mesh.normals.as_deref().unwrap_or(&[]);
            let layer = self.output.begin("LayerElementNormal", &[Property::Int(0)])?;
            self.output.property("Version", Property::Int(101))?;
            self.output.property("Name", Property::string(""))?;
            self.output.property("MappingInformationType", Property::string("ByPolygonVertex"))?;
            self.output.property("ReferenceInformationType", Property::string("Direct"))?;
            self.output.begin_array("Normals", b'd', corner_count * 3, 8)?;
            for face in faces {
                for &index in face {
                    for &value in &normals[index as usize] {
                        self.output.double(f64::from(value))?;
                    }
                }
            }
            self.output.end(layer, true)?;
        }

        let color_channels = mesh.colors.iter().take_while(|colors| colors.is_some()).count();
        for channel in 0..color_channels {
            let colors = mesh.colors[channel].as_deref().unwrap_or(&[]);
            let layer = self.output.begin("LayerElementColor", &[Property::Int(channel as i32)])?;
            let name = format!("COLOR_{channel}");
            self.output.property("Version", Property::Int(101))?;
            self.output.property("Name", Property::string(&name))?;
            self.output.property("MappingInformationType", Property::string("ByPolygonVertex"))?;
            self.output.property("ReferenceInformationType", Property::string("Direct"))?;
            self.output.begin_array("Colors", b'd', corner_count * 4, 8)?;
            for face in faces {
                for &index in face {
                    for &value in &colors[index as usize] {
                        self.output.double(f64::from(value))?;
                    }
                }
            }
            self.output.end(layer, true)?;
        }

        let uv_channels = mesh.texture_coords.iter().take_while(|coordinates| coordinates.is_some()).count();
        for channel in 0..uv_channels {
            self.write_uv_layer(mesh, channel, faces, corner_count)?;
        }

        // Assimp's material layer is always "AllSame" material 0; the
        // material is linked by the connections.
        let mut material_layer = Node::new("LayerElementMaterial", vec![Property::Int(0)]);
        material_layer.add("Version", Property::Int(101));
        material_layer.add("Name", Property::string(""));
        material_layer.add("MappingInformationType", Property::string("AllSame"));
        material_layer.add("ReferenceInformationType", Property::string("IndexToDirect"));
        material_layer.add("Materials", Property::Ints(vec![0]));
        self.output.node(&material_layer)?;

        // Layer 0 names normals and UVs whether the mesh has them or not.
        let mut layer = Node::new("Layer", vec![Property::Int(0)]);
        layer.add("Version", Property::Int(100));
        layer.children.push(layer_element("LayerElementNormal", 0));
        for channel in 0..color_channels {
            layer.children.push(layer_element("LayerElementColor", channel as i32));
        }
        layer.children.push(layer_element("LayerElementMaterial", 0));
        layer.children.push(layer_element("LayerElementUV", 0));
        self.output.node(&layer)?;

        for channel in 1..uv_channels {
            let mut extra_layer = Node::new("Layer", vec![Property::Int(channel as i32)]);
            extra_layer.add("Version", Property::Int(100));
            extra_layer.children.push(layer_element("LayerElementUV", channel as i32));
            self.output.node(&extra_layer)?;
        }

        self.output.end(geometry, true)
    }

    /// A UV channel by polygon corner: the distinct coordinates in order of
    /// first use (equal as `aiVector3D::operator<` sees them, so −0 is 0)
    /// with each corner's index into them.
    fn write_uv_layer(&mut self, mesh: &Mesh, channel: usize, faces: &[Vec<u32>], corner_count: usize) -> io::Result<()> {
        let coordinates = mesh.texture_coords[channel].as_deref().unwrap_or(&[]);
        let components = (mesh.uv_components[channel] as usize).min(3);
        let layer = self.output.begin("LayerElementUV", &[Property::Int(channel as i32)])?;
        self.output.property("Version", Property::Int(101))?;
        self.output.property("Name", Property::string(""))?;
        self.output.property("MappingInformationType", Property::string("ByPolygonVertex"))?;
        self.output.property("ReferenceInformationType", Property::string("IndexToDirect"))?;

        let mut index_by_uv: HashMap<[u32; 3], i32> = HashMap::new();
        let mut uv_data: Vec<f64> = Vec::new();
        let mut uv_indices: Vec<i32> = Vec::with_capacity(corner_count);
        for face in faces {
            for &index in face {
                let uv = coordinates[index as usize];
                let key = uv.map(|value| (value + 0.0).to_bits());
                let next_index = index_by_uv.len() as i32;
                let uv_index = *index_by_uv.entry(key).or_insert_with(|| {
                    for &value in &uv[..components] {
                        uv_data.push(f64::from(value));
                    }
                    next_index
                });
                uv_indices.push(uv_index);
            }
        }

        self.output.begin_array("UV", b'd', uv_data.len(), 8)?;
        for &value in &uv_data {
            self.output.double(value)?;
        }
        self.output.begin_array("UVIndex", b'i', uv_indices.len(), 4)?;
        for &uv_index in &uv_indices {
            self.output.int(uv_index)?;
        }
        self.output.end(layer, true)
    }

    /// A material with only a name and a diffuse colour: Lambert, the
    /// diffuse colour, then the legacy properties with their defaults.
    fn write_material(&mut self, material: &ExportMaterial) -> io::Result<()> {
        let uid = self.generate_uid();
        self.material_uids.push(uid);
        let [red, green, blue, _] = material.diffuse.map(f64::from);
        let ambient = f64::from(0.2f32);
        let entries = vec![
            p70_color_a("DiffuseColor", [red, green, blue]),
            p70_vector("Emissive", [0.0; 3]),
            p70_vector("Ambient", [ambient; 3]),
            p70_vector("Diffuse", [red, green, blue]),
            p70_double("Opacity", 1.0),
        ];
        let name = format!("{}{SEPARATOR}Material", material.name);
        let properties = vec![Property::Long(uid), Property::string(&name), Property::string("")];
        let mut node = Node::new("Material", properties);
        node.add("Version", Property::Int(102));
        node.add("ShadingModel", Property::string("lambert"));
        node.add("MultiLayer", Property::Int(0));
        node.children.push(properties70(entries));
        self.output.node(&node)
    }

    /// `WriteModelNodes` from the root node, which is no model itself: a
    /// "Mesh" model per mesh it holds, named after the mesh and connected
    /// to the root (uid 0), its geometry and its material.
    fn write_model_nodes(&mut self) -> io::Result<()> {
        let scene = self.scene;
        for &mesh_index in &scene.root_meshes {
            let mesh = &scene.meshes[mesh_index as usize];
            let uid = self.generate_uid();
            let mesh_uid = self.mesh_uids[mesh_index as usize];
            let material_uid = self.material_uids[mesh.material_index as usize];
            self.connect(uid, 0);
            self.connect(mesh_uid, uid);
            self.connect(material_uid, uid);
            self.write_model_node(&mesh.name, uid)?;
        }
        Ok(())
    }

    /// `WriteModelNode` for a node with an identity transform (no "Lcl"
    /// properties) and no metadata.
    fn write_model_node(&mut self, name: &str, uid: i64) -> io::Result<()> {
        let entries = vec![p70_bool("RotationActive", true), p70_int("DefaultAttributeIndex", 0), p70_enum("InheritType", INHERIT_RSRS)];
        let name = format!("{name}{SEPARATOR}Model");
        let properties = vec![Property::Long(uid), Property::string(&name), Property::string("Mesh")];
        let mut model = Node::new("Model", properties);
        model.add("Version", Property::Int(232));
        model.children.push(properties70(entries));
        model.add("Shading", Property::Bool(true));
        model.add("Culling", Property::string("CullingOff"));
        self.output.node(&model)
    }

    fn write_connections(&mut self) -> io::Result<()> {
        let mut connections = Node::new("Connections", Vec::new());
        connections.children = std::mem::take(&mut self.connections);
        self.output.node(&connections)
    }

    fn write_footer(&mut self) -> io::Result<()> {
        self.output.write(&NULL_RECORD)?;
        self.output.write(GENERIC_FOOT_ID)?;
        // Zeros to a multiple of 16 bytes: a whole 16 when already there.
        let padding = 16 - (self.output.position % 16) as usize;
        let zeros = [0u8; 16];
        self.output.write(&zeros[..padding])?;
        self.output.write(&[0; 4])?;
        self.output.write(&EXPORT_VERSION.to_le_bytes())?;
        self.output.write(&[0; 120])?;
        self.output.write(FOOT_MAGIC)
    }
}

/// `ExportSceneFBX`: `scene` as a binary FBX file at `path`.
pub fn write_binary(scene: &ExportScene, path: &Path) -> Result<(), ExportError> {
    let file = File::create(path).map_err(|_| ExportError(format!("could not open output .fbx file: {}", path.display())))?;
    let output = Output { file: BufWriter::new(file), position: 0 };
    let mut exporter = Exporter { scene, output, last_uid: LAST_UID, mesh_uids: Vec::new(), material_uids: Vec::new(), connections: Vec::new() };
    exporter.export().map_err(|error| ExportError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Mesh, PRIMITIVE_TRIANGLE};

    /// A node's name, properties' raw bytes and children, read back.
    struct ReadNode {
        name: String,
        children: Vec<ReadNode>,
    }

    fn u64_at(data: &[u8], position: usize) -> u64 {
        u64::from_le_bytes(data[position..position + 8].try_into().unwrap())
    }

    /// Reads the node list at `position` up to its null record, checking
    /// every node's end offset and property length on the way.
    fn read_nodes(data: &[u8], mut position: usize, end: usize) -> Vec<ReadNode> {
        let mut nodes = Vec::new();
        while position < end {
            let node_end = u64_at(data, position) as usize;
            if node_end == 0 {
                assert_eq!(&data[position..position + 25], &[0u8; 25], "null record");
                break;
            }
            let property_length = u64_at(data, position + 16) as usize;
            let name_length = data[position + 24] as usize;
            let name = String::from_utf8(data[position + 25..position + 25 + name_length].to_vec()).unwrap();
            let children_start = position + 25 + name_length + property_length;
            let children = if children_start < node_end { read_nodes(data, children_start, node_end) } else { Vec::new() };
            nodes.push(ReadNode { name, children });
            position = node_end;
        }
        nodes
    }

    #[test]
    fn binary_layout_is_consistent() {
        let mesh = Mesh {
            name: "Mesh".to_string(),
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count: 3,
            vertices: Some(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
            normals: Some(vec![[0.0, 1.0, 0.0]; 3]),
            faces: Some(vec![vec![0, 1, 2]]),
            ..Mesh::default()
        };
        let scene = ExportScene {
            root_name: "RootNode".to_string(),
            root_meshes: vec![0],
            meshes: vec![mesh],
            materials: vec![ExportMaterial { name: "DefaultMaterial".to_string(), diffuse: [1.0; 4] }],
        };
        let path = std::env::temp_dir().join(format!("nerthus-fbx-test-{}.fbx", std::process::id()));
        write_binary(&scene, &path).unwrap();
        let data = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(&data[..23], BINARY_HEADER);
        assert_eq!(u32::from_le_bytes(data[23..27].try_into().unwrap()), EXPORT_VERSION);
        assert_eq!(&data[data.len() - 16..], FOOT_MAGIC);
        let nodes = read_nodes(&data, 27, data.len());
        let names: Vec<&str> = nodes.iter().map(|node| node.name.as_str()).collect();
        let expected = [
            "FBXHeaderExtension", "FileId", "CreationTime", "Creator", "GlobalSettings", "Documents", "References", "Definitions", "Objects", "Connections",
        ];
        assert_eq!(names, expected);
        let objects: Vec<&str> = nodes[8].children.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(objects, ["Geometry", "Material", "Model"]);
        let geometry: Vec<&str> = nodes[8].children[0].children.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(geometry, ["Vertices", "PolygonVertexIndex", "GeometryVersion", "LayerElementNormal", "LayerElementMaterial", "Layer"]);
        // Model to root, geometry to model, material to model.
        assert_eq!(nodes[9].children.len(), 3);
    }
}
