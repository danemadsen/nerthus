//! The Transform family: Transform, Transform3D, Dilate, Distance,
//! Aperture and Construction.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeData, NodeError, RenderIntent};
use serde_json::{Value, json};

use crate::masking;
use crate::maths;
use crate::mesh_rasterizer::{self, RenderSettings};
use crate::morph::{self, DistanceRtMode, DistanceRtSettings};
use crate::morphology2::{self, ApertureKernel, ApertureSettings};
use crate::morphology::{self, DistanceOptions, Kernel};
use crate::transformer::{self, AnchorPosition, BlendMode, TransformQuality};

fn quality(name: &str) -> TransformQuality {
    match name {
        "Draft" => TransformQuality::Draft,
        "High" => TransformQuality::High,
        _ => TransformQuality::Medium,
    }
}

/// The Transform node's `EdgeFade`.
fn fade_edges(map: Map, edges: &str) -> Map {
    match edges {
        "Thin" => masking::mask_by_square_edge(&map, 0.15),
        "Wide" => {
            let faded = masking::mask_by_square_edge(&map, 0.9);
            masking::mask_by_square_edge(&faded, 0.1)
        }
        "Soft" => masking::vignette(&map, 0.5, 0.25),
        _ => map,
    }
}

/// Moves, scales and rotates `In` and pastes it over `Base` (or black).
pub struct Transform;

struct Placement {
    position: (f32, f32),
    scale: (f32, f32),
    rotation: f32,
    height: f32,
    mode: BlendMode,
    quality: TransformQuality,
    edges: String,
}

impl Placement {
    fn read(context: &BuildContext) -> Self {
        let position = ((context.f32("OffsetX") + 1.0) * 0.5, (context.f32("OffsetY") + 1.0) * 0.5);
        let scale_factor = context.f32("Scale");
        let scale = if context.bool("Uniform") {
            (scale_factor, scale_factor)
        } else {
            (scale_factor * context.f32("ScaleX"), scale_factor * context.f32("ScaleY"))
        };
        Self {
            position,
            scale,
            rotation: context.f32("Rotate"),
            height: context.f32("OffsetZ"),
            mode: BlendMode::from_name(context.enum_name("BlendMode")),
            quality: quality(context.enum_name("Quality")),
            edges: context.enum_name("Edges").to_string(),
        }
    }

    fn paste(&self, source: &Map, base: &mut Map) {
        let raised = source.multiply_scalar(self.height);
        let faded = fade_edges(raised, &self.edges);
        transformer::multi_transform(
            &faded,
            base,
            self.position,
            self.scale,
            self.rotation,
            1.0,
            self.mode,
            self.quality,
            None,
            true,
            AnchorPosition::Center,
        );
    }
}

impl NodeBuilder for Transform {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let base_name = context.input_names().get(1).map(|name| name.to_string());
        let base_input = base_name.and_then(|name| context.input(&name));
        let mut base = match base_input {
            Some(map) => (*map).clone(),
            None => Map::new(context.resolution, 1),
        };
        let placement = Placement::read(context);

        if input.channel_count() == 1 {
            placement.paste(&input, &mut base);
            return context.commit(base);
        }

        if base.channel_count() == 1 {
            base = Map::stack(&[&base, &base, &base]);
        }
        let mut channels = Vec::with_capacity(input.channel_count());
        for channel in 0..input.channel_count() {
            let source = Map::from_data(input.resolution(), input.channel(channel).to_vec());
            let mut target = Map::from_data(base.resolution(), base.channel(channel).to_vec());
            placement.paste(&source, &mut target);
            channels.push(target.into_data());
        }
        context.render_as(RenderIntent::Color);
        let combined = Map::from_channels(input.resolution(), channels);
        context.commit(combined)
    }
}

/// The rotation, scale, offsets and options shared by Transform3D and
/// Object (`MeshRasterizer.Parameters`).
pub fn mesh_render_settings(context: &BuildContext, antialiasing: bool) -> RenderSettings {
    let resolution = context.resolution;
    let offset = [context.f32("OffsetX"), context.f32("OffsetY"), context.f32("OffsetZ")];
    RenderSettings {
        width: resolution.x as i32,
        height: resolution.y as i32,
        height_ratio: context.terrain_height() / context.terrain_width(),
        pitch: context.f32("Pitch"),
        yaw: context.f32("Yaw"),
        roll: context.f32("Roll"),
        scale: context.f32("Scale"),
        offset,
        drop_to_floor: context.bool("DropToFloor"),
        antialiasing,
    }
}

/// Transform3D: the input as a mesh (one vertex per pixel), rotated,
/// scaled and moved, rendered back to a heightmap. Its Antialiasing
/// setting never reaches the renderer (`Transform3D.Build`).
pub struct Transform3D;

impl NodeBuilder for Transform3D {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let settings = mesh_render_settings(context, false);
        let terrain_width = context.terrain_width();
        let terrain_height = context.terrain_height();
        let mesh = mesh_rasterizer::heightmap_to_mesh(input.data(), input.width(), input.height(), terrain_width, terrain_height);
        let rendered = mesh_rasterizer::render_mesh(&mesh, &settings);
        context.commit(Map::from_data(context.resolution, rendered))
    }
}

/// Flat dilation (`Morphology.Dilate`); `Invert` erodes instead.
pub struct Dilate;

impl NodeBuilder for Dilate {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let invert = context.bool("Invert");
        let mut source = (*input).clone();
        if invert {
            maths::invert(&mut source);
        }
        let kernel = Kernel::from_name(context.enum_name("Kernel"));
        let mut dilated = morphology::dilate(&source, kernel, context.f32("Size"), context.i32("Iterations"), context.f32("Shape"));
        if invert {
            maths::invert(&mut dilated);
        }
        context.commit(dilated)
    }
}

/// Distance from the input's high areas (`Morphology.DistanceTransform`;
/// the RT method is `Morph.DistanceRT`).
pub struct Distance;

impl NodeBuilder for Distance {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        if context.enum_name("Method") != "Classic" {
            let settings = DistanceRtSettings {
                mode: DistanceRtMode::from_name(context.enum_name("Mode")),
                directions: context.i32("Directions"),
                skew: context.f32("Skew"),
                angle: context.f32("Angle"),
                angular_jitter: context.f32("AngularJitter"),
                falloff: context.f32("Falloff"),
                falloff_jitter: context.f32("FalloffJitter"),
                invert_input: context.bool("InvertInput"),
                invert_output: context.bool("InvertOutput"),
                seed: context.i32("Seed"),
            };
            let terrain = (context.terrain_width(), context.terrain_height());
            let distances = morph::distance_rt(&input, &settings, terrain);
            return context.commit(distances);
        }
        let size = context.resolution.x;
        let invert_input = context.bool("InvertInput");
        let options = DistanceOptions {
            falloff: size as f32 * context.f32("Falloff"),
            threshold: context.f32("Threshold"),
            invert_input: !invert_input,
            invert_output: context.bool("InvertOutput"),
            clamp: true,
        };
        let source = input.data();
        let mut distances = morphology::distance_transform(source, size, options);
        if context.bool("MultiplyByInput") {
            for (value, &original) in distances.iter_mut().zip(source) {
                let factor = if invert_input { 1.0 - original } else { original };
                *value *= factor;
            }
        }
        let map = Map::from_data(maths::square_resolution(distances.len()), distances);
        context.commit(map)
    }
}

/// Shaped dilation or erosion (`Morphology2.Aperture`).
pub struct Aperture;

impl NodeBuilder for Aperture {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let settings = ApertureSettings {
            size: context.resolution.x as f32 * context.f32("Size"),
            kernel: ApertureKernel::from_name(context.enum_name("Shape")),
            erosion: context.enum_name("Mode") == "Erosion",
            vertices: context.i32("VerticesCount"),
            antialiased: context.bool("Antialiased"),
            direction: context.i32("Direction") as f32,
            corner_angle: context.i32("CornerAngle") as f32,
        };
        let shaped = morphology2::aperture(&input, &settings);
        context.commit(shaped)
    }
}

/// One placed terrain of a Construction (`TerrainTransform`).
struct TerrainTransform {
    x: f32,
    y: f32,
    z: f32,
    base: f32,
    shaper: f32,
    rotate: f32,
    scale: f32,
    is_visible: bool,
    /// `TerrainBlendMode`: Max, Min, Add or Subtract.
    blend_mode: String,
    /// `TerrainBlendMask`: None, Square, Round or Soft.
    mask: String,
    mask_bias: f32,
    index: usize,
}

impl TerrainTransform {
    /// Reads a saved transform; members it leaves out keep the class
    /// defaults.
    fn from_value(value: &Value) -> Self {
        let number = |name: &str, default: f64| value.get(name).and_then(Value::as_f64).unwrap_or(default) as f32;
        let text = |name: &str, default: &str| value.get(name).and_then(Value::as_str).unwrap_or(default).to_string();
        Self {
            x: number("X", 0.0),
            y: number("Y", 0.0),
            z: number("Z", 1.0),
            base: number("Base", 0.0),
            shaper: number("Shaper", 0.0),
            rotate: number("Rotate", 0.0),
            scale: number("Scale", 1.0),
            is_visible: value.get("IsVisible").and_then(Value::as_bool).unwrap_or(true),
            blend_mode: text("BlendMode", "Max"),
            mask: text("Mask", "Square"),
            mask_bias: number("MaskBias", 0.25),
            index: value.get("Index").and_then(Value::as_u64).unwrap_or(0) as usize,
        }
    }

    /// A new transform as `AddNewPort` appends it.
    fn default_value(index: usize) -> Value {
        json!({
            "X": 0.0, "Y": 0.0, "Z": 1.0, "Base": 0.0, "Shaper": 0.0, "Rotate": 0.0, "Scale": 1.0,
            "Color": {"R": 0.0, "G": 0.0, "B": 0.0},
            "IsVisible": true, "BlendMode": "Max", "Mask": "Square", "MaskBias": 0.25, "Index": index,
        })
    }
}

/// Construction: places other terrains on a base terrain, each moved,
/// scaled, rotated, raised and blended through an edge mask.
pub struct Construction;

impl Construction {
    /// `TransformTerrainFinal`: pastes the terrain feeding the transform's
    /// port into `base`; a port without a map leaves it as it is.
    fn place(context: &BuildContext, transform: &TerrainTransform, base: &mut Map) -> Result<(), NodeError> {
        let names = context.input_names();
        let Some(port) = names.get(transform.index + 1) else {
            return Err(NodeError::Failed("Index was outside the bounds of the array.".to_string()));
        };
        let Some(terrain) = context.input(port) else {
            return Ok(());
        };
        let mut terrain = (*terrain).clone();
        if transform.shaper != 0.0 {
            maths::strengthen(&mut terrain, transform.shaper);
        }
        if transform.z != 1.0 {
            let z = transform.z;
            terrain.apply(|value| value * z);
        }
        if transform.base > 0.0 {
            let lift = transform.base;
            terrain.apply(|value| value + lift);
        }
        if terrain.channel_count() > 1 {
            return Err(NodeError::Failed("This node requires heightfield input, not colors.".to_string()));
        }

        let ones = Map::filled(context.resolution, 1.0);
        let mask = match transform.mask.as_str() {
            "Square" | "Soft" => Some(masking::mask_by_square_edge(&ones, transform.mask_bias)),
            "Round" => Some(masking::vignette(&ones, transform.mask_bias, 0.25)),
            _ => None,
        };
        let mode = BlendMode::from_name(&transform.blend_mode);
        let quality = quality(context.enum_name("Quality"));
        let position = (transform.x, transform.y);
        let scale = (transform.scale, transform.scale);
        transformer::multi_transform(&terrain, base, position, scale, transform.rotate, 1.0, mode, quality, mask.as_ref(), true, AnchorPosition::Center);
        Ok(())
    }
}

impl NodeBuilder for Construction {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let Some(base) = context.input("In") else {
            return Err(NodeError::Failed("Construction requires a base terrain.".to_string()));
        };
        let mut base = (*base).clone();
        let transforms: Vec<TerrainTransform> = context
            .value("Transforms")
            .as_array()
            .map(|items| items.iter().filter(|item| item.is_object()).map(TerrainTransform::from_value).collect())
            .unwrap_or_default();
        for transform in transforms.iter().filter(|transform| transform.is_visible) {
            Self::place(context, transform, &mut base)?;
        }
        context.commit(base)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
        if count >= 15 {
            return None;
        }
        let number = count + 1;
        let port = format!("Terrain{number}");
        node.add_port(&port, "In");
        node.values.insert("PortCount".to_string(), Value::from(number));
        let transforms = node.values.entry("Transforms".to_string()).or_insert_with(|| json!([]));
        if let Some(items) = transforms.as_array_mut() {
            items.push(TerrainTransform::default_value(number as usize - 1));
        }
        Some(port)
    }
}
