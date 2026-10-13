//! FBXDocument.cpp and the object types geometry conversion reaches: the
//! header, property templates, global settings, the lazily constructed
//! objects (`LazyObject::Get`: a malformed object header fails whoever
//! asked, a failing constructor only yields no object), and connections.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::parser::{Element, File, Scope, required_element, required_scope};
use crate::scene::{ImportError, fail};

fn dom_error<T>(message: &str) -> Result<T, ImportError> {
    fail(format!("FBX-DOM: {message}"))
}

/// A typed property value (`ReadTypedProperty`); only floats are read
/// back, the other types matter for their parse errors and for not being
/// floats.
#[derive(Clone, Debug)]
pub enum Property {
    Float(f32),
    Other,
}

/// `ReadTypedProperty`: `None` for unknown types or fewer than two tokens.
fn read_typed_property(file: &File, element: &Element) -> Result<Option<Property>, ImportError> {
    let tokens = &element.tokens;
    if tokens.len() < 2 {
        return Ok(None);
    }
    let kind = file.string(tokens[1])?;
    let need = |count: usize| -> Result<(), ImportError> {
        if tokens.len() < count {
            return fail("Not enough tokens for property");
        }
        Ok(())
    };
    let property = match kind.as_slice() {
        b"KString" => {
            need(5)?;
            file.string(tokens[4])?;
            Property::Other
        }
        b"bool" | b"Bool" | b"int" | b"Int" | b"enum" | b"Enum" | b"Integer" => {
            need(5)?;
            file.int(tokens[4])?;
            Property::Other
        }
        b"ULongLong" => {
            need(5)?;
            file.id(tokens[4])?;
            Property::Other
        }
        b"KTime" => {
            need(5)?;
            file.int64(tokens[4])?;
            Property::Other
        }
        b"Vector3D" | b"ColorRGB" | b"Vector" | b"Color" | b"Lcl Translation" | b"Lcl Rotation" | b"Lcl Scaling" => {
            need(7)?;
            for &token in &tokens[4..7] {
                file.float(token)?;
            }
            Property::Other
        }
        b"double" | b"Number" | b"float" | b"Float" | b"FieldOfView" | b"UnitScaleFactor" => {
            need(5)?;
            Property::Float(file.float(tokens[4])?)
        }
        b"ColorAndAlpha" => {
            need(8)?;
            for &token in &tokens[4..8] {
                file.float(token)?;
            }
            Property::Other
        }
        _ => return Ok(None),
    };
    Ok(Some(property))
}

/// `PropertyTable`: `P` elements by name (the first of a name wins),
/// parsed on first use, falling back to the template.
pub struct PropertyTable<'a> {
    file: &'a File,
    lazy: BTreeMap<Vec<u8>, &'a Element>,
    parsed: RefCell<HashMap<Vec<u8>, Option<Property>>>,
    template: Option<Rc<PropertyTable<'a>>>,
}

impl<'a> PropertyTable<'a> {
    fn empty(file: &'a File) -> Self {
        PropertyTable { file, lazy: BTreeMap::new(), parsed: RefCell::new(HashMap::new()), template: None }
    }

    /// `PropertyTable(element, template)`.
    fn new(file: &'a File, element: &'a Element, template: Option<Rc<PropertyTable<'a>>>) -> Result<Self, ImportError> {
        let scope = required_scope(element)?;
        let mut lazy = BTreeMap::new();
        for (key, property) in scope.all() {
            if key != b"P" {
                continue;
            }
            // `PeekPropertyName`.
            if property.tokens.len() < 4 {
                continue;
            }
            let name = file.string(property.tokens[0])?;
            if name.is_empty() {
                continue;
            }
            lazy.entry(name).or_insert(property);
        }
        Ok(PropertyTable { file, lazy, parsed: RefCell::new(HashMap::new()), template })
    }

    /// `PropertyTable::Get`.
    pub fn get(&self, name: &str) -> Result<Option<Property>, ImportError> {
        if let Some(value) = self.parsed.borrow().get(name.as_bytes()) {
            return Ok(value.clone());
        }
        if let Some(element) = self.lazy.get(name.as_bytes()) {
            let value = read_typed_property(self.file, element)?;
            self.parsed.borrow_mut().insert(name.as_bytes().to_vec(), value.clone());
            return Ok(value);
        }
        match &self.template {
            Some(template) => template.get(name),
            None => Ok(None),
        }
    }

    /// `GetUnparsedProperties`: every own property not read yet.
    pub fn parse_unparsed(&self) -> Result<(), ImportError> {
        for (name, element) in &self.lazy {
            if self.parsed.borrow().contains_key(name) {
                continue;
            }
            read_typed_property(self.file, element)?;
        }
        Ok(())
    }

}

/// `GetPropertyTable`: the element's `Properties70` with the template, the
/// template alone, or an empty table.
fn property_table<'a>(document: &Document<'a>, template_name: &str, scope: &'a Scope) -> Result<Rc<PropertyTable<'a>>, ImportError> {
    let template = if template_name.is_empty() { None } else { document.templates.get(template_name.as_bytes()).cloned() };
    let Some(properties) = scope.get("Properties70").filter(|element| element.compound.is_some()) else {
        return Ok(template.unwrap_or_else(|| Rc::new(PropertyTable::empty(document.file))));
    };
    Ok(Rc::new(PropertyTable::new(document.file, properties, template)?))
}

pub struct MeshGeometry {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<u32>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub binormals: Vec<[f32; 3]>,
    pub uvs: [Vec<[f32; 2]>; 8],
    pub colors: [Vec<[f32; 4]>; 8],
    pub materials: Vec<i32>,
    pub blend_shapes: Vec<u64>,
}

pub struct LineGeometry {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<i32>,
}

pub struct ShapeGeometry {
    pub indices: Vec<u32>,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

pub struct Model<'a> {
    pub props: Rc<PropertyTable<'a>>,
    pub geometry: Vec<u64>,
    pub materials: Vec<u64>,
    pub attributes: Vec<u64>,
}

/// The objects geometry conversion distinguishes; the rest construct
/// without being read.
pub enum Object<'a> {
    Model(Model<'a>),
    Mesh(Box<MeshGeometry>),
    Line(LineGeometry),
    Shape(ShapeGeometry),
    BlendShape(Vec<u64>),
    BlendShapeChannel(Vec<u64>),
    Material(Rc<PropertyTable<'a>>),
    Light(Rc<PropertyTable<'a>>),
    Camera(Rc<PropertyTable<'a>>),
    OtherNodeAttribute,
    Texture,
    Other,
}

impl Object<'_> {
    pub fn is_geometry(&self) -> bool {
        matches!(self, Object::Mesh(_) | Object::Line(_) | Object::Shape(_))
    }

    pub fn is_node_attribute(&self) -> bool {
        matches!(self, Object::Light(_) | Object::Camera(_) | Object::OtherNodeAttribute)
    }
}

const BEING_CONSTRUCTED: u8 = 1;
const FAILED_TO_CONSTRUCT: u8 = 2;

pub struct LazyObject<'a> {
    pub element: &'a Element,
    flags: Cell<u8>,
    object: RefCell<Option<Rc<Object<'a>>>>,
}

pub struct Connection {
    pub order: u64,
    pub source: u64,
    pub property: Vec<u8>,
}

pub struct Document<'a> {
    pub file: &'a File,
    templates: HashMap<Vec<u8>, Rc<PropertyTable<'a>>>,
    pub global_settings: Rc<PropertyTable<'a>>,
    objects: HashMap<u64, LazyObject<'a>>,
    animation_stacks: Vec<u64>,
    connections: Vec<Connection>,
    by_destination: HashMap<u64, Vec<usize>>,
    by_source: HashMap<u64, Vec<usize>>,
}

impl<'a> Document<'a> {
    /// `Document::Document`: header, templates, global settings, objects
    /// and connections.
    pub fn new(file: &'a File, root: &'a Scope) -> Result<Self, ImportError> {
        let mut document = Document {
            file,
            templates: HashMap::new(),
            global_settings: Rc::new(PropertyTable::empty(file)),
            objects: HashMap::new(),
            animation_stacks: Vec::new(),
            connections: Vec::new(),
            by_destination: HashMap::new(),
            by_source: HashMap::new(),
        };
        document.read_header(root)?;
        document.read_templates(root)?;
        document.read_global_settings(root)?;
        document.read_objects(root)?;
        document.read_connections(root)?;
        Ok(document)
    }

    fn read_header(&self, root: &'a Scope) -> Result<(), ImportError> {
        let Some(header) = root.get("FBXHeaderExtension").and_then(|element| element.compound.as_deref()) else {
            return dom_error("no FBXHeaderExtension dictionary found");
        };
        let version_element = required_element(header, "FBXVersion")?;
        let version = self.file.int(self.file.required_token(version_element, 0)?)?;
        if version < 7100 {
            return dom_error("unsupported, old format version, supported are only FBX 2011, FBX 2012 and FBX 2013");
        }
        if let Some(creator) = header.get("Creator") {
            self.file.string(self.file.required_token(creator, 0)?)?;
        }
        if let Some(stamp) = header.get("CreationTimeStamp").and_then(|element| element.compound.as_deref()) {
            for field in ["Year", "Month", "Day", "Hour", "Minute", "Second", "Millisecond"] {
                let element = required_element(stamp, field)?;
                self.file.int(self.file.required_token(element, 0)?)?;
            }
        }
        Ok(())
    }

    fn read_templates(&mut self, root: &'a Scope) -> Result<(), ImportError> {
        let Some(definitions) = root.get("Definitions").and_then(|element| element.compound.as_deref()) else {
            return Ok(());
        };
        for object_type in definitions.collection("ObjectType") {
            let Some(scope) = object_type.compound.as_deref() else {
                continue;
            };
            let Some(&name_token) = object_type.tokens.first() else {
                continue;
            };
            let object_name = self.file.string(name_token)?;
            for template in scope.collection("PropertyTemplate") {
                let Some(inner) = template.compound.as_deref() else {
                    continue;
                };
                let Some(&template_token) = template.tokens.first() else {
                    continue;
                };
                let template_name = self.file.string(template_token)?;
                if let Some(properties) = inner.get("Properties70") {
                    let table = PropertyTable::new(self.file, properties, None)?;
                    let mut key = object_name.clone();
                    key.push(b'.');
                    key.extend_from_slice(&template_name);
                    self.templates.insert(key, Rc::new(table));
                }
            }
        }
        Ok(())
    }

    fn read_global_settings(&mut self, root: &'a Scope) -> Result<(), ImportError> {
        let Some(settings) = root.get("GlobalSettings").and_then(|element| element.compound.as_deref()) else {
            return Ok(());
        };
        self.global_settings = property_table(self, "", settings)?;
        Ok(())
    }

    fn read_objects(&mut self, root: &'a Scope) -> Result<(), ImportError> {
        let Some(objects) = root.get("Objects").filter(|element| element.compound.is_some()) else {
            return dom_error("no Objects dictionary found");
        };
        self.objects.insert(0, LazyObject { element: objects, flags: Cell::new(0), object: RefCell::new(None) });
        for (key, element) in objects.compound.as_deref().unwrap().all() {
            let Some(&first) = element.tokens.first() else {
                return dom_error("expected ID after object key");
            };
            let id = self.file.id(first)?;
            if id == 0 {
                return dom_error("encountered object with implicitly defined id 0");
            }
            self.objects.insert(id, LazyObject { element, flags: Cell::new(0), object: RefCell::new(None) });
            if key == b"AnimationStack" {
                self.animation_stacks.push(id);
            }
        }
        Ok(())
    }

    fn read_connections(&mut self, root: &'a Scope) -> Result<(), ImportError> {
        let Some(connections) = root.get("Connections").and_then(|element| element.compound.as_deref()) else {
            return dom_error("no Connections dictionary found");
        };
        let mut order = 0u64;
        for element in connections.collection("C") {
            let kind = self.file.string(self.file.required_token(element, 0)?)?;
            if kind == b"PP" {
                continue;
            }
            let source = self.file.id(self.file.required_token(element, 1)?)?;
            let destination = self.file.id(self.file.required_token(element, 2)?)?;
            let property = if kind == b"OP" { self.file.string(self.file.required_token(element, 3)?)? } else { Vec::new() };
            if !self.objects.contains_key(&source) || !self.objects.contains_key(&destination) {
                continue;
            }
            let index = self.connections.len();
            self.connections.push(Connection { order, source, property });
            order += 1;
            self.by_destination.entry(destination).or_default().push(index);
            self.by_source.entry(source).or_default().push(index);
        }
        Ok(())
    }

    /// The `AnimationStack` objects in file order (`ConvertAnimations` gets
    /// each first; a malformed header fails the import).
    pub fn animation_stacks(&self) -> Vec<u64> {
        self.animation_stacks.clone()
    }

    /// The object's key token text (its element type).
    pub fn object_key(&self, id: u64) -> &[u8] {
        let element = self.objects[&id].element;
        self.file.bytes(element.key)
    }

    /// `GetConnectionsByDestinationSequenced(dest, classnames)`: the
    /// connections into `destination` whose source's element key is one of
    /// `classes` (all when empty), in file order.
    pub fn connections_into(&self, destination: u64, classes: &[&str]) -> Vec<&Connection> {
        let Some(indices) = self.by_destination.get(&destination) else {
            return Vec::new();
        };
        let mut selected: Vec<&Connection> = indices
            .iter()
            .map(|&index| &self.connections[index])
            .filter(|connection| classes.is_empty() || classes.iter().any(|class| self.object_key(connection.source) == class.as_bytes()))
            .collect();
        selected.sort_by_key(|connection| connection.order);
        selected
    }

    /// `LazyObject::Get`: the header (three tokens: id, name, class; its
    /// errors propagate), then the constructor (whose errors only make the
    /// object absent).
    pub fn get(&self, id: u64) -> Result<Option<Rc<Object<'a>>>, ImportError> {
        let Some(lazy) = self.objects.get(&id) else {
            return Ok(None);
        };
        if lazy.flags.get() & (BEING_CONSTRUCTED | FAILED_TO_CONSTRUCT) != 0 {
            return Ok(None);
        }
        if let Some(object) = lazy.object.borrow().as_ref() {
            return Ok(Some(object.clone()));
        }
        let element = lazy.element;
        if element.tokens.len() < 3 {
            return dom_error("expected at least 3 tokens: id, name and class tag");
        }
        self.file.string(element.tokens[1])?;
        let class = self.file.string(element.tokens[2])?;

        lazy.flags.set(lazy.flags.get() | BEING_CONSTRUCTED);
        let key = self.file.bytes(element.key).to_vec();
        let constructed = self.construct(id, &key, &class, element);
        lazy.flags.set(lazy.flags.get() & !BEING_CONSTRUCTED);
        match constructed {
            Ok(Some(object)) => {
                let object = Rc::new(object);
                *lazy.object.borrow_mut() = Some(object.clone());
                Ok(Some(object))
            }
            Ok(None) => Ok(None),
            Err(_) => {
                lazy.flags.set(lazy.flags.get() | FAILED_TO_CONSTRUCT);
                Ok(None)
            }
        }
    }

    /// The constructors (`strncmp` against the key's own length, so a key
    /// that is a prefix of a type name also matches it).
    fn construct(&self, id: u64, key: &[u8], class: &[u8], element: &'a Element) -> Result<Option<Object<'a>>, ImportError> {
        let prefix_of = |name: &str| -> bool {
            // strncmp(obtype, name, length): equal over the key's length,
            // a NUL in `name` within it ends the comparison as a mismatch.
            let name = name.as_bytes();
            for (index, &byte) in key.iter().enumerate() {
                let other = name.get(index).copied().unwrap_or(0);
                if byte != other {
                    return false;
                }
                if other == 0 {
                    return true;
                }
            }
            true
        };
        if prefix_of("Geometry") {
            return match class {
                b"Mesh" => Ok(Some(Object::Mesh(Box::new(self.mesh_geometry(id, element)?)))),
                b"Shape" => Ok(Some(Object::Shape(self.shape_geometry(id, element)?))),
                b"Line" => Ok(Some(Object::Line(self.line_geometry(id, element)?))),
                _ => Ok(None),
            };
        }
        if prefix_of("NodeAttribute") {
            return match class {
                b"Camera" => Ok(Some(Object::Camera(self.node_attribute(element, class)?))),
                b"Light" => Ok(Some(Object::Light(self.node_attribute(element, class)?))),
                b"CameraSwitcher" | b"Null" | b"LimbNode" => {
                    self.node_attribute(element, class)?;
                    Ok(Some(Object::OtherNodeAttribute))
                }
                _ => Ok(None),
            };
        }
        if prefix_of("Deformer") {
            return match class {
                b"BlendShape" => Ok(Some(Object::BlendShape(self.blend_shape(id, element)?))),
                b"BlendShapeChannel" => Ok(Some(Object::BlendShapeChannel(self.blend_shape_channel(id, element)?))),
                b"Cluster" | b"Skin" => Ok(Some(Object::Other)),
                _ => Ok(None),
            };
        }
        if prefix_of("Model") {
            if class == b"IKEffector" || class == b"FKEffector" {
                return Ok(None);
            }
            return Ok(Some(Object::Model(self.model(id, element)?)));
        }
        if prefix_of("Material") {
            return Ok(Some(Object::Material(self.material(id, element)?)));
        }
        if prefix_of("Texture") || prefix_of("LayeredTexture") {
            return Ok(Some(Object::Texture));
        }
        if prefix_of("Video") || prefix_of("AnimationStack") || prefix_of("AnimationLayer") || prefix_of("AnimationCurve") || prefix_of("AnimationCurveNode") {
            return Ok(Some(Object::Other));
        }
        Ok(None)
    }

    /// `Geometry::Geometry`: its deformer links (a malformed deformer
    /// header fails the geometry); blend shapes are kept.
    fn geometry_deformers(&self, id: u64) -> Result<Vec<u64>, ImportError> {
        let mut blend_shapes = Vec::new();
        for connection in self.connections_into(id, &["Deformer"]) {
            if !connection.property.is_empty() {
                continue;
            }
            let Some(object) = self.get(connection.source)? else {
                continue;
            };
            if matches!(*object, Object::BlendShape(_)) && !blend_shapes.contains(&connection.source) {
                blend_shapes.push(connection.source);
            }
        }
        Ok(blend_shapes)
    }

    fn mesh_geometry(&self, id: u64, element: &'a Element) -> Result<MeshGeometry, ImportError> {
        let blend_shapes = self.geometry_deformers(id)?;
        let Some(scope) = element.compound.as_deref() else {
            return dom_error("failed to read Geometry object (class: Mesh), no data scope found");
        };
        let vertices_element = required_element(scope, "Vertices")?;
        let indices_element = required_element(scope, "PolygonVertexIndex")?;
        let source_vertices = self.file.vectors3(vertices_element)?;
        let polygon_indices = self.file.ints(indices_element)?;

        let vertex_count = source_vertices.len();
        let mut vertices = Vec::with_capacity(polygon_indices.len());
        let mut faces = Vec::new();
        let mut mapping_counts = vec![0u32; vertex_count];
        let mut count = 0u32;
        for &index in &polygon_indices {
            let absolute = if index < 0 { (index as i64).wrapping_neg() - 1 } else { index as i64 };
            if absolute as u64 >= vertex_count as u64 {
                return dom_error("polygon vertex index out of range");
            }
            vertices.push(source_vertices[absolute as usize]);
            count += 1;
            mapping_counts[absolute as usize] += 1;
            if index < 0 {
                faces.push(count);
                count = 0;
            }
        }
        let mut mapping_offsets = vec![0u32; vertex_count];
        let mut cursor = 0u32;
        for vertex in 0..vertex_count {
            mapping_offsets[vertex] = cursor;
            cursor += mapping_counts[vertex];
            mapping_counts[vertex] = 0;
        }
        let mut mappings = vec![0u32; polygon_indices.len()];
        for (cursor, &index) in (0u32..).zip(&polygon_indices) {
            let absolute = if index < 0 { (index as i64).wrapping_neg() - 1 } else { index as i64 } as usize;
            mappings[(mapping_offsets[absolute] + mapping_counts[absolute]) as usize] = cursor;
            mapping_counts[absolute] += 1;
        }

        let mut geometry = MeshGeometry {
            vertices,
            faces,
            normals: Vec::new(),
            tangents: Vec::new(),
            binormals: Vec::new(),
            uvs: Default::default(),
            colors: Default::default(),
            materials: Vec::new(),
            blend_shapes,
        };
        let layout = Layout { offsets: &mapping_offsets, counts: &mapping_counts, mappings: &mappings };
        for layer in scope.collection("Layer") {
            let Some(&index_token) = layer.tokens.first() else {
                return dom_error("expected a layer index");
            };
            self.file.int(index_token)?;
            let layer_scope = required_scope(layer)?;
            for layer_element in layer_scope.collection("LayerElement") {
                let element_scope = required_scope(layer_element)?;
                self.read_layer_element(&mut geometry, scope, element_scope, &layout)?;
            }
        }
        Ok(geometry)
    }

    /// `ReadLayerElement`: the `Type`/`TypedIndex` reference resolved
    /// against the geometry's own elements.
    fn read_layer_element(&self, geometry: &mut MeshGeometry, top: &'a Scope, layer_element: &'a Scope, layout: &Layout) -> Result<(), ImportError> {
        let kind_element = required_element(layer_element, "Type")?;
        let index_element = required_element(layer_element, "TypedIndex")?;
        let kind = self.file.string(self.file.required_token(kind_element, 0)?)?;
        let typed_index = self.file.int(self.file.required_token(index_element, 0)?)?;
        for candidate in top.collection_bytes(&kind) {
            let index = self.file.int(self.file.required_token(candidate, 0)?)?;
            if index == typed_index {
                let source = required_scope(candidate)?;
                return self.read_vertex_data(geometry, &kind, typed_index, source, layout);
            }
        }
        Ok(())
    }

    fn read_vertex_data(&self, geometry: &mut MeshGeometry, kind: &[u8], index: i32, source: &'a Scope, layout: &Layout) -> Result<(), ImportError> {
        let mapping = self.file.string(self.file.required_token(required_element(source, "MappingInformationType")?, 0)?)?;
        let reference = self.file.string(self.file.required_token(required_element(source, "ReferenceInformationType")?, 0)?)?;
        let vertex_count = geometry.vertices.len();
        let resolve = |data: &str, index_data: &str, width: usize| -> Result<Option<Vec<Vec<f32>>>, ImportError> {
            resolve_vertex_data(self.file, source, &mapping, &reference, data, index_data, width, vertex_count, layout)
        };
        match kind {
            b"LayerElementUV" => {
                // (A negative index would write before the array.)
                if !(0..8).contains(&index) {
                    return Ok(());
                }
                if let Some(name) = source.get("Name") {
                    self.file.string(self.file.required_token(name, 0)?)?;
                }
                if let Some(values) = resolve("UV", "UVIndex", 2)? {
                    geometry.uvs[index as usize] = values.iter().map(|value| [value[0], value[1]]).collect();
                }
            }
            b"LayerElementMaterial" => {
                if !geometry.materials.is_empty() {
                    return Ok(());
                }
                let materials = self.read_materials(geometry, source, &mapping, &reference)?;
                if materials.iter().all(|&material| material < 0) {
                    return Ok(());
                }
                geometry.materials = materials;
            }
            b"LayerElementNormal" => {
                if !geometry.normals.is_empty() {
                    return Ok(());
                }
                if let Some(values) = resolve("Normals", "NormalsIndex", 3)? {
                    geometry.normals = values.iter().map(|value| [value[0], value[1], value[2]]).collect();
                }
            }
            b"LayerElementTangent" => {
                if !geometry.tangents.is_empty() {
                    return Ok(());
                }
                let plural = source.count("Tangents") > 0;
                let (data, index_data) = if plural { ("Tangents", "TangentsIndex") } else { ("Tangent", "TangentIndex") };
                if let Some(values) = resolve(data, index_data, 3)? {
                    geometry.tangents = values.iter().map(|value| [value[0], value[1], value[2]]).collect();
                }
            }
            b"LayerElementBinormal" => {
                if !geometry.binormals.is_empty() {
                    return Ok(());
                }
                let plural = source.count("Binormals") > 0;
                let (data, index_data) = if plural { ("Binormals", "BinormalsIndex") } else { ("Binormal", "BinormalIndex") };
                if let Some(values) = resolve(data, index_data, 3)? {
                    geometry.binormals = values.iter().map(|value| [value[0], value[1], value[2]]).collect();
                }
            }
            b"LayerElementColor" => {
                if !(0..8).contains(&index) {
                    return Ok(());
                }
                if let Some(values) = resolve("Colors", "ColorIndex", 4)? {
                    geometry.colors[index as usize] = values.iter().map(|value| [value[0], value[1], value[2], value[3]]).collect();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `ReadVertexDataMaterials`.
    fn read_materials(&self, geometry: &MeshGeometry, source: &'a Scope, mapping: &[u8], reference: &[u8]) -> Result<Vec<i32>, ImportError> {
        let face_count = geometry.faces.len();
        let mut materials = Vec::new();
        if face_count == 0 {
            return Ok(materials);
        }
        if let Some(element) = source.get("Materials") {
            materials = self.file.ints(element)?;
        }
        if mapping == b"AllSame" {
            if materials.is_empty() {
                return Ok(materials);
            }
            if materials.len() > 1 {
                materials.clear();
            }
            materials.resize(geometry.vertices.len(), 0);
            let first = materials[0];
            materials.fill(first);
        } else if mapping == b"ByPolygon" && reference == b"IndexToDirect" {
            materials.resize(face_count, 0);
        }
        // Other mappings keep the array as read ("not implemented").
        Ok(materials)
    }

    fn shape_geometry(&self, id: u64, element: &'a Element) -> Result<ShapeGeometry, ImportError> {
        self.geometry_deformers(id)?;
        let Some(scope) = element.compound.as_deref() else {
            return dom_error("failed to read Geometry object (class: Shape), no data scope found");
        };
        let indices_element = required_element(scope, "Indexes")?;
        let vertices_element = required_element(scope, "Vertices")?;
        let indices = self.file.uints(indices_element)?;
        let vertices = self.file.vectors3(vertices_element)?;
        let mut normals = Vec::new();
        if let Some(element) = scope.get("Normals") {
            normals = self.file.vectors3(element)?;
        }
        Ok(ShapeGeometry { indices, vertices, normals })
    }

    fn line_geometry(&self, id: u64, element: &'a Element) -> Result<LineGeometry, ImportError> {
        self.geometry_deformers(id)?;
        let Some(scope) = element.compound.as_deref() else {
            return dom_error("failed to read Geometry object (class: Line), no data scope found");
        };
        let points = required_element(scope, "Points")?;
        let indices = required_element(scope, "PointsIndex")?;
        let vertices = self.file.vectors3(points)?;
        let indices = self.file.ints(indices)?;
        Ok(LineGeometry { vertices, indices })
    }

    /// `Deformer::Deformer`: the class name and property table.
    fn deformer(&self, element: &'a Element) -> Result<(), ImportError> {
        let scope = required_scope(element)?;
        let class = self.file.string(self.file.required_token(element, 2)?)?;
        let template = format!("Deformer.Fbx{}", String::from_utf8_lossy(&class));
        property_table(self, &template, scope)?;
        Ok(())
    }

    fn blend_shape(&self, id: u64, element: &'a Element) -> Result<Vec<u64>, ImportError> {
        self.deformer(element)?;
        let mut channels = Vec::new();
        for connection in self.connections_into(id, &["Deformer"]) {
            if !connection.property.is_empty() {
                continue;
            }
            let Some(object) = self.get(connection.source)? else {
                continue;
            };
            if matches!(*object, Object::BlendShapeChannel(_)) && !channels.contains(&connection.source) {
                channels.push(connection.source);
            }
        }
        Ok(channels)
    }

    fn blend_shape_channel(&self, id: u64, element: &'a Element) -> Result<Vec<u64>, ImportError> {
        self.deformer(element)?;
        let scope = required_scope(element)?;
        if let Some(percent) = scope.get("DeformPercent") {
            self.file.float(self.file.required_token(percent, 0)?)?;
        }
        if let Some(weights) = scope.get("FullWeights") {
            self.file.floats(weights)?;
        }
        let mut shapes = Vec::new();
        for connection in self.connections_into(id, &["Geometry"]) {
            if !connection.property.is_empty() {
                continue;
            }
            let Some(object) = self.get(connection.source)? else {
                continue;
            };
            if matches!(*object, Object::Shape(_)) && !shapes.contains(&connection.source) {
                shapes.push(connection.source);
            }
        }
        Ok(shapes)
    }

    /// `NodeAttribute::NodeAttribute`.
    fn node_attribute(&self, element: &'a Element, class: &[u8]) -> Result<Rc<PropertyTable<'a>>, ImportError> {
        let scope = required_scope(element)?;
        self.file.string(self.file.required_token(element, 2)?)?;
        let template = format!("NodeAttribute.Fbx{}", String::from_utf8_lossy(class));
        property_table(self, &template, scope)
    }

    /// `Model::Model` with `ResolveLinks`.
    fn model(&self, id: u64, element: &'a Element) -> Result<Model<'a>, ImportError> {
        let scope = required_scope(element)?;
        if let Some(shading) = scope.get("Shading") {
            self.file.required_token(shading, 0)?;
        }
        if let Some(culling) = scope.get("Culling") {
            self.file.string(self.file.required_token(culling, 0)?)?;
        }
        let props = property_table(self, "Model.FbxNode", scope)?;
        let mut model = Model { props, geometry: Vec::new(), materials: Vec::new(), attributes: Vec::new() };
        for connection in self.connections_into(id, &["Geometry", "Material", "NodeAttribute"]) {
            if !connection.property.is_empty() {
                continue;
            }
            let Some(object) = self.get(connection.source)? else {
                continue;
            };
            if matches!(*object, Object::Material(_)) {
                model.materials.push(connection.source);
            } else if object.is_geometry() {
                model.geometry.push(connection.source);
            } else if object.is_node_attribute() {
                model.attributes.push(connection.source);
            }
        }
        Ok(model)
    }

    /// `Material::Material` (texture links read for their headers).
    fn material(&self, id: u64, element: &'a Element) -> Result<Rc<PropertyTable<'a>>, ImportError> {
        let scope = required_scope(element)?;
        if let Some(layer) = scope.get("MultiLayer") {
            self.file.int(self.file.required_token(layer, 0)?)?;
        }
        let mut shading = b"phong".to_vec();
        if let Some(model) = scope.get("ShadingModel") {
            shading = self.file.string(self.file.required_token(model, 0)?)?;
        }
        shading.make_ascii_lowercase();
        let template = match shading.as_slice() {
            b"phong" => "Material.FbxSurfacePhong",
            b"lambert" => "Material.FbxSurfaceLambert",
            _ => "",
        };
        let props = property_table(self, template, scope)?;
        for connection in self.connections_into(id, &[]) {
            if connection.property.is_empty() {
                continue;
            }
            self.get(connection.source)?;
        }
        Ok(props)
    }
}

/// The polygon-vertex layout of a mesh (`m_mapping_offsets`,
/// `m_mappings`; the counts live on the geometry).
struct Layout<'a> {
    offsets: &'a [u32],
    counts: &'a [u32],
    mappings: &'a [u32],
}

/// `ResolveVertexDataArray` for `width`-float tuples: `None` where Assimp
/// leaves the channel empty.
#[allow(clippy::too_many_arguments)]
fn resolve_vertex_data(file: &File, source: &Scope, mapping: &[u8], reference: &[u8], data_name: &str, index_name: &str, width: usize, vertex_count: usize, layout: &Layout) -> Result<Option<Vec<Vec<f32>>>, ImportError> {
    let mut direct = reference == b"Direct";
    let mut index_to_direct = reference == b"IndexToDirect";
    let has_data = source.get(data_name).is_some();
    let has_index = source.get(index_name).is_some();
    if index_to_direct && !has_index {
        direct = true;
        index_to_direct = false;
    }
    let counts = layout.counts;
    let empty = vec![0.0f32; width];
    if mapping == b"ByVertice" && direct {
        if !has_data {
            return Ok(None);
        }
        let values = file.float_tuples(required_element(source, data_name)?, width)?;
        if values.len() != layout.offsets.len() {
            return Ok(None);
        }
        let mut output = vec![empty; vertex_count];
        for (vertex, value) in values.iter().enumerate() {
            let start = layout.offsets[vertex];
            for slot in start..start + counts[vertex] {
                output[layout.mappings[slot as usize] as usize] = value.clone();
            }
        }
        return Ok(Some(output));
    }
    if mapping == b"ByVertice" && index_to_direct {
        if !has_data || !has_index {
            return Ok(None);
        }
        let values = file.float_tuples(required_element(source, data_name)?, width)?;
        let indices = file.ints(required_element(source, index_name)?)?;
        if indices.len() != layout.offsets.len() {
            return Ok(None);
        }
        let mut output = vec![empty; vertex_count];
        for (vertex, &index) in indices.iter().enumerate() {
            let start = layout.offsets[vertex];
            for slot in start..start + counts[vertex] {
                if index as i64 as u64 >= values.len() as u64 {
                    return dom_error("index out of range");
                }
                output[layout.mappings[slot as usize] as usize] = values[index as usize].clone();
            }
        }
        return Ok(Some(output));
    }
    if mapping == b"ByPolygonVertex" && direct {
        if !has_data {
            return Ok(None);
        }
        let values = file.float_tuples(required_element(source, data_name)?, width)?;
        if values.len() != vertex_count {
            return Ok(None);
        }
        return Ok(Some(values));
    }
    if mapping == b"ByPolygonVertex" && index_to_direct {
        if !has_data || !has_index {
            return Ok(None);
        }
        let values = file.float_tuples(required_element(source, data_name)?, width)?;
        let mut indices = file.ints(required_element(source, index_name)?)?;
        if indices.len() > vertex_count {
            indices.truncate(vertex_count);
        }
        if indices.len() != vertex_count {
            return Ok(None);
        }
        let mut output = Vec::with_capacity(vertex_count);
        for &index in &indices {
            if index == -1 {
                output.push(empty.clone());
                continue;
            }
            if index as i64 as u64 >= values.len() as u64 {
                return dom_error("index out of range");
            }
            output.push(values[index as usize].clone());
        }
        return Ok(Some(output));
    }
    Ok(None)
}
