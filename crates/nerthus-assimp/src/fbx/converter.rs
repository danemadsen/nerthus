//! FBXConverter.cpp, as far as it decides the meshes: models reached from
//! the root through "Model" connections (in file order, a model's meshes
//! before its children's), each geometry converted once (split by
//! material when its polygons use several), line geometry as segments,
//! and the property reads and blend shape checks that can fail the import.

use std::collections::HashMap;
use std::rc::Rc;

use super::document::{Document, LineGeometry, MeshGeometry, Model, Object, Property};
use crate::scene::{ImportError, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, fail};
use crate::vector::cross;

/// The model properties `GenerateTransformationNodeChain` reads.
const TRANSFORM_PROPERTIES: [&str; 12] = [
    "PreRotation",
    "PostRotation",
    "RotationPivot",
    "RotationOffset",
    "ScalingOffset",
    "ScalingPivot",
    "Lcl Translation",
    "Lcl Scaling",
    "Lcl Rotation",
    "GeometricScaling",
    "GeometricRotation",
    "GeometricTranslation",
];

/// The material properties `SetShadingPropertiesCommon` reads.
const MATERIAL_PROPERTIES: [&str; 26] = [
    "DiffuseColor",
    "DiffuseFactor",
    "EmissiveColor",
    "EmissiveFactor",
    "AmbientColor",
    "AmbientFactor",
    "SpecularColor",
    "SpecularFactor",
    "ShininessExponent",
    "TransparentColor",
    "TransparencyFactor",
    "Opacity",
    "ReflectionColor",
    "ReflectionFactor",
    "BumpFactor",
    "DisplacementFactor",
    "Maya|base_color",
    "Maya|emissive",
    "Maya|emissive_intensity",
    "Maya|metallic",
    "Maya|roughness",
    "Maya|use_ao_map",
    "Maya|use_color_map",
    "Maya|use_emissive_map",
    "Maya|use_metallic_map",
    "Maya|use_roughness_map",
];

const LIGHT_PROPERTIES: [&str; 7] = ["LightType", "Color", "Intensity", "DecayType", "DecayStart", "InnerAngle", "OuterAngle"];
const CAMERA_PROPERTIES: [&str; 7] = ["AspectWidth", "AspectHeight", "FilmWidth", "FocalLength", "FieldOfView", "NearPlane", "FarPlane"];
const GLOBAL_PROPERTIES: [&str; 15] = [
    "UpAxis",
    "UpAxisSign",
    "FrontAxis",
    "FrontAxisSign",
    "CoordAxis",
    "CoordAxisSign",
    "OriginalUpAxis",
    "OriginalUpAxisSign",
    "UnitScaleFactor",
    "OriginalUnitScaleFactor",
    "AmbientColor",
    "TimeMode",
    "TimeSpanStart",
    "TimeSpanStop",
    "CustomFrameRate",
];

struct Converter<'d, 'a> {
    document: &'d Document<'a>,
    meshes: Vec<Mesh>,
    converted_geometry: HashMap<u64, ()>,
    converted_materials: HashMap<u64, ()>,
}

/// The primitive type bit of an `n`-corner face.
fn primitive_type(corners: u32) -> u32 {
    match corners {
        1 => PRIMITIVE_POINT,
        2 => PRIMITIVE_LINE,
        3 => PRIMITIVE_TRIANGLE,
        _ => PRIMITIVE_POLYGON,
    }
}

impl Converter<'_, '_> {
    fn object(&self, id: u64) -> Result<Option<Rc<Object<'_>>>, ImportError> {
        self.document.get(id)
    }

    /// `ConvertNodes`.
    fn convert_nodes(&mut self, id: u64) -> Result<(), ImportError> {
        for connection in self.document.connections_into(id, &["Model"]) {
            if !connection.property.is_empty() {
                continue;
            }
            let source = connection.source;
            let Some(object) = self.document.get(source)? else {
                continue;
            };
            let Object::Model(model) = &*object else {
                continue;
            };
            // GenerateTransformationNodeChain, then SetupNodeMetadata.
            model.props.get("RotationOrder")?;
            for name in TRANSFORM_PROPERTIES {
                model.props.get(name)?;
            }
            model.props.parse_unparsed()?;
            model.props.get("UDP3DSMAX")?;

            self.convert_model(model)?;
            self.convert_nodes(source)?;
            self.check_node_attributes(model)?;
        }
        Ok(())
    }

    /// `ConvertLights`/`ConvertCameras`: their property reads.
    fn check_node_attributes(&self, model: &Model) -> Result<(), ImportError> {
        for &attribute in &model.attributes {
            let Some(object) = self.document.get(attribute)? else {
                continue;
            };
            match &*object {
                Object::Light(props) => {
                    for name in LIGHT_PROPERTIES {
                        props.get(name)?;
                    }
                }
                Object::Camera(props) => {
                    for name in CAMERA_PROPERTIES {
                        props.get(name)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `ConvertModel`.
    fn convert_model(&mut self, model: &Model) -> Result<(), ImportError> {
        for &geometry in &model.geometry {
            let Some(object) = self.document.get(geometry)? else {
                continue;
            };
            match &*object {
                Object::Mesh(mesh) => self.convert_mesh(geometry, mesh, model)?,
                Object::Line(line) => self.convert_line(geometry, line),
                _ => {}
            }
        }
        Ok(())
    }

    /// `ConvertMesh`: once per geometry; several materials split it.
    fn convert_mesh(&mut self, id: u64, geometry: &MeshGeometry, model: &Model) -> Result<(), ImportError> {
        if self.converted_geometry.contains_key(&id) {
            return Ok(());
        }
        if geometry.vertices.is_empty() || geometry.faces.is_empty() {
            return Ok(());
        }
        let materials = &geometry.materials;
        if let Some(&first) = materials.first()
            && materials.iter().any(|&material| material != first) {
                return self.convert_multi_material(id, geometry, model);
            }
        self.converted_geometry.insert(id, ());
        let mut mesh = Mesh {
            vertex_count: geometry.vertices.len(),
            vertices: Some(geometry.vertices.clone()),
            ..Mesh::default()
        };
        let mut faces = Vec::with_capacity(geometry.faces.len());
        let mut cursor = 0u32;
        for &corners in &geometry.faces {
            mesh.primitive_types |= primitive_type(corners);
            faces.push((cursor..cursor + corners).collect());
            cursor += corners;
        }
        mesh.faces = Some(faces);
        if !geometry.normals.is_empty() {
            mesh.normals = Some(geometry.normals.clone());
        }
        if let Some((tangents, bitangents)) = tangent_frames(geometry) {
            mesh.tangents = Some(tangents);
            mesh.bitangents = Some(bitangents);
        }
        for set in 0..8 {
            if geometry.uvs[set].is_empty() {
                break;
            }
            mesh.texture_coords[set] = Some(geometry.uvs[set].iter().map(|uv| [uv[0], uv[1], 0.0]).collect());
            mesh.uv_components[set] = 2;
        }
        for set in 0..8 {
            if geometry.colors[set].is_empty() {
                break;
            }
            mesh.colors[set] = Some(geometry.colors[set].clone());
        }
        self.meshes.push(mesh);
        if let Some(&material) = materials.first() {
            self.convert_material_for_mesh(model, material)?;
        }
        self.check_blend_shapes(geometry)
    }

    /// `ConvertMeshMultiMaterial`: one mesh per material index, in order of
    /// first use, each with its polygons' corners.
    fn convert_multi_material(&mut self, id: u64, geometry: &MeshGeometry, model: &Model) -> Result<(), ImportError> {
        self.converted_geometry.insert(id, ());
        let mut seen = Vec::new();
        for &material in &geometry.materials {
            if seen.contains(&material) {
                continue;
            }
            seen.push(material);
            let mut mesh = Mesh::default();
            let tangent_frame = tangent_frames(geometry);
            let uv_sets = (0..8).take_while(|&set| !geometry.uvs[set].is_empty()).count();
            let color_sets = (0..8).take_while(|&set| !geometry.colors[set].is_empty()).count();
            let mut vertices = Vec::new();
            let mut normals = Vec::new();
            let mut tangents = Vec::new();
            let mut bitangents = Vec::new();
            let mut uvs: Vec<Vec<[f32; 3]>> = vec![Vec::new(); uv_sets];
            let mut colors: Vec<Vec<[f32; 4]>> = vec![Vec::new(); color_sets];
            let mut faces = Vec::new();
            let mut source = 0usize;
            for (&face_material, &corners) in geometry.materials.iter().zip(&geometry.faces) {
                if face_material != material {
                    source += corners as usize;
                    continue;
                }
                mesh.primitive_types |= primitive_type(corners);
                let start = vertices.len() as u32;
                faces.push((start..start + corners).collect());
                for corner in source..source + corners as usize {
                    vertices.push(geometry.vertices[corner]);
                    if !geometry.normals.is_empty() {
                        normals.push(geometry.normals[corner]);
                    }
                    if let Some((all_tangents, all_bitangents)) = &tangent_frame {
                        tangents.push(all_tangents[corner]);
                        bitangents.push(all_bitangents[corner]);
                    }
                    for (set_uvs, source_uvs) in uvs.iter_mut().zip(&geometry.uvs) {
                        let uv = source_uvs[corner];
                        set_uvs.push([uv[0], uv[1], 0.0]);
                    }
                    for (set_colors, source_colors) in colors.iter_mut().zip(&geometry.colors) {
                        set_colors.push(source_colors[corner]);
                    }
                }
                source += corners as usize;
            }
            mesh.vertex_count = vertices.len();
            mesh.vertices = Some(vertices);
            mesh.faces = Some(faces);
            if !geometry.normals.is_empty() {
                mesh.normals = Some(normals);
            }
            if tangent_frame.is_some() {
                mesh.tangents = Some(tangents);
                mesh.bitangents = Some(bitangents);
            }
            for (set, coordinates) in uvs.into_iter().enumerate() {
                mesh.texture_coords[set] = Some(coordinates);
                mesh.uv_components[set] = 2;
            }
            for (set, values) in colors.into_iter().enumerate() {
                mesh.colors[set] = Some(values);
            }
            self.meshes.push(mesh);
            self.convert_material_for_mesh(model, material)?;
            self.check_blend_shapes(geometry)?;
        }
        Ok(())
    }

    /// `ConvertLine`: a segment from each non-negative index to the next
    /// (an end marker `-(i + 1)` naming vertex `i`, the last wrapping to
    /// the first).
    fn convert_line(&mut self, id: u64, line: &LineGeometry) {
        if line.vertices.is_empty() || line.indices.is_empty() {
            return;
        }
        self.converted_geometry.insert(id, ());
        let mut mesh = Mesh { primitive_types: PRIMITIVE_LINE, ..Mesh::default() };
        mesh.vertex_count = line.vertices.len();
        mesh.vertices = Some(line.vertices.clone());
        let count = line.indices.len();
        let mut faces = Vec::new();
        for index in 0..count {
            let start = line.indices[index];
            if start < 0 {
                continue;
            }
            let next = line.indices[if index + 1 == count { 0 } else { index + 1 }];
            let end = if next < 0 { (next as i64 + 1).wrapping_neg() } else { next as i64 };
            faces.push(vec![start as u32, end as u32]);
        }
        mesh.faces = Some(faces);
        self.meshes.push(mesh);
    }

    /// `ConvertMaterialForMesh` → `ConvertMaterial`: the property reads.
    fn convert_material_for_mesh(&mut self, model: &Model, index: i32) -> Result<(), ImportError> {
        if index < 0 || index as usize >= model.materials.len() {
            return Ok(());
        }
        let material = model.materials[index as usize];
        if self.converted_materials.contains_key(&material) {
            return Ok(());
        }
        self.converted_materials.insert(material, ());
        let Some(object) = self.document.get(material)? else {
            return Ok(());
        };
        let Object::Material(props) = &*object else {
            return Ok(());
        };
        for name in MATERIAL_PROPERTIES {
            props.get(name)?;
        }
        props.parse_unparsed()
    }

    /// The blend shape reads that throw (`std::vector::at`): a shape with
    /// more indices than vertices (or than normals, when it has some).
    fn check_blend_shapes(&self, geometry: &MeshGeometry) -> Result<(), ImportError> {
        for &shape in &geometry.blend_shapes {
            let Some(object) = self.object(shape)? else {
                continue;
            };
            let Object::BlendShape(channels) = &*object else {
                continue;
            };
            for &channel in channels {
                let Some(channel_object) = self.object(channel)? else {
                    continue;
                };
                let Object::BlendShapeChannel(shapes) = &*channel_object else {
                    continue;
                };
                for &shape_id in shapes {
                    let Some(shape_object) = self.object(shape_id)? else {
                        continue;
                    };
                    let Object::Shape(shape) = &*shape_object else {
                        continue;
                    };
                    let too_few_vertices = shape.indices.len() > shape.vertices.len();
                    let too_few_normals = !shape.normals.is_empty() && shape.indices.len() > shape.normals.len();
                    if too_few_vertices || too_few_normals {
                        return fail("invalid vector subscript");
                    }
                }
            }
        }
        Ok(())
    }
}

/// Per-vertex tangents and bitangents.
type TangentFrames = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// The mesh's tangents and bitangents: binormals as read, else
/// `normal ^ tangent` when there are normals, else none at all.
fn tangent_frames(geometry: &MeshGeometry) -> Option<TangentFrames> {
    if geometry.tangents.is_empty() {
        return None;
    }
    if !geometry.binormals.is_empty() {
        return Some((geometry.tangents.clone(), geometry.binormals.clone()));
    }
    if geometry.normals.is_empty() {
        return None;
    }
    let mut binormals = vec![[0.0f32; 3]; geometry.normals.len()];
    for (index, tangent) in geometry.tangents.iter().enumerate() {
        if index < binormals.len() {
            binormals[index] = cross(geometry.normals[index], *tangent);
        }
    }
    Some((geometry.tangents.clone(), binormals))
}

/// `ConvertToAssimpScene` for the meshes, then the importer's
/// `UnitScaleFactor` check.
pub fn convert(document: &Document) -> Result<Vec<Mesh>, ImportError> {
    for stack in document.animation_stacks() {
        document.get(stack)?;
    }
    let mut converter = Converter { document, meshes: Vec::new(), converted_geometry: HashMap::new(), converted_materials: HashMap::new() };
    converter.convert_nodes(0)?;
    for name in GLOBAL_PROPERTIES {
        document.global_settings.get(name)?;
    }
    let scale = match document.global_settings.get("UnitScaleFactor")? {
        Some(Property::Float(value)) => value,
        _ => 1.0,
    };
    if scale == 0.0 {
        return fail("The UnitScaleFactor must be non-zero");
    }
    Ok(converter.meshes)
}
