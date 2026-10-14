//! The viewport's renderers (`RendererBase` and its `RawRenderer`,
//! `ClayRenderer` and `RealisticRenderer`) and the lighting they read
//! (`TerrainState.Viewport`, which the lighting panel edits). A render
//! thread bakes the terrain's colours on the CPU, lighting, shadows and
//! occlusion included, and the 3D view shows them unlit.

use std::sync::Arc;
use std::thread;

use bevy::prelude::Resource;
use crossbeam_channel::{unbounded, Receiver, Sender};
use nerthus_engine::json::f32_value;
use nerthus_engine::{Map, RenderIntent, Resolution};
use nerthus_nodes::shade::{AmbientOcclusionModel, LightRig};
use nerthus_nodes::{colour, histogram, maths};
use nerthus_render::TerrainColours;
use serde_json::{Map as JsonMap, Value};

/// `RenderMode`: how the viewport draws the preview (`Preview Renderer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// `None`, "Visualize Data (Render Off)".
    Data,
    Clay,
    Realistic,
}

impl RenderMode {
    /// `Viewport.RenderMode` as saved: a name, or Newtonsoft's number. Not
    /// saved means `None`; `Mesh` and `Experimental` leave the renderer as
    /// it was, as loading a project does.
    pub fn from_state(saved: Option<&Value>) -> Option<Self> {
        let Some(saved) = saved else {
            return Some(RenderMode::Data);
        };
        let name = match saved {
            Value::String(name) => name.as_str(),
            Value::Number(number) => match number.as_i64() {
                Some(0) => "None",
                Some(1) => "Clay",
                Some(2) => "Realistic",
                _ => "",
            },
            _ => "",
        };
        match name {
            "None" => Some(RenderMode::Data),
            "Clay" => Some(RenderMode::Clay),
            "Realistic" => Some(RenderMode::Realistic),
            _ => None,
        }
    }

    pub fn state_name(self) -> &'static str {
        match self {
            RenderMode::Data => "None",
            RenderMode::Clay => "Clay",
            RenderMode::Realistic => "Realistic",
        }
    }
}

/// `ProcessInput`: what the viewport does to a mask before colouring it
/// (`Mask Enhancement`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskEnhancement {
    #[default]
    None,
    Autolevel,
    Equalize,
}

/// `RendererQuality`: how much of its lighting the realistic renderer
/// takes the time for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderQuality {
    /// `FastestNoAOShadow`: no occlusion and no shadows (the Constructor's
    /// Faster Previews).
    Fastest,
    /// `FastAO`: local occlusion and quick shadows (the Constructor's
    /// previews, and the viewport while a gizmo or Draw's painter is held).
    Fast,
    /// Soft shadows and hybrid occlusion.
    #[default]
    Normal,
}

/// `MaskGradient`: the colours masks are shown in. It starts from
/// `Options.MaskGradient` and the lighting panel changes it for the
/// session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskGradient {
    #[default]
    Greyscale,
    Hot,
    Red,
    Blue,
    Spectrum,
}

/// `MaskGradient`'s members in order, with their names.
pub const MASK_GRADIENTS: [(MaskGradient, &str); 5] = [
    (MaskGradient::Greyscale, "Greyscale"),
    (MaskGradient::Hot, "Hot"),
    (MaskGradient::Red, "Red"),
    (MaskGradient::Blue, "Blue"),
    (MaskGradient::Spectrum, "Spectrum"),
];

/// The "Hot" gradient.
const HOT: [&str; 9] = ["#080808", "#1c147f", "#9e2399", "#b62592", "#d72d50", "#f28600", "#f8cf01", "#f7f4ac", "#f2f2f2"];

impl MaskGradient {
    /// `(MaskGradient)Options.MaskGradient`.
    pub fn from_index(index: i64) -> Self {
        let found = MASK_GRADIENTS.get(index.max(0) as usize).map(|(gradient, _)| *gradient);
        found.unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        let found = MASK_GRADIENTS.iter().find(|(gradient, _)| *gradient == self);
        found.map(|(_, name)| *name).unwrap_or("Greyscale")
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let found = MASK_GRADIENTS.iter().find(|(_, gradient_name)| *gradient_name == name);
        found.map(|(gradient, _)| *gradient)
    }

    /// `B.ClutMap` as `PrivateSky.Build` sets it: the gradient's colour
    /// stops.
    pub fn stops(self) -> Vec<[f32; 3]> {
        match self {
            MaskGradient::Greyscale => vec![[0.0; 3], [1.0; 3]],
            MaskGradient::Hot => HOT.iter().map(|hex| hex_colour(hex)).collect(),
            MaskGradient::Red => vec![[0.75; 3], [1.0, 0.0, 0.0]],
            MaskGradient::Blue => vec![[0.75; 3], [0.0, 0.0, 1.0]],
            MaskGradient::Spectrum => vec![
                [1.0, 0.0, 0.0],
                [1.0, 0.5, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.29, 0.0, 0.51],
                [0.56, 0.0, 1.0],
            ],
        }
    }
}

/// `new Color("#rrggbb").ToVector3()`: each byte over 255.
fn hex_colour(hex: &str) -> [f32; 3] {
    let channel = |start: usize| {
        let digits = hex.get(start..start + 2).unwrap_or("00");
        let byte = u8::from_str_radix(digits, 16).unwrap_or(0);
        byte as f32 / 255.0
    };
    [channel(1), channel(3), channel(5)]
}

/// The project's viewport lighting (`TerrainState.Viewport`), which the
/// lighting panel (`PrivateSky`) edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportLighting {
    /// Degrees clockwise from north (the map's top).
    pub sun_azimuth: f32,
    /// `SunAltitude`: degrees above the horizon.
    pub sun_elevation: f32,
    pub sun_intensity: f32,
    pub ambient_intensity: f32,
    pub exposure: f32,
    pub shadows: bool,
    pub ambient_occlusion: bool,
    pub ground_brightness: f32,
    pub air_density: f32,
    pub haze: f32,
    pub ozone: f32,
    pub fog_density: f32,
}

impl Default for ViewportLighting {
    /// A new `Viewport`.
    fn default() -> Self {
        Self {
            sun_azimuth: 45.0,
            sun_elevation: 33.0,
            sun_intensity: 1.0,
            ambient_intensity: 1.0,
            exposure: 1.0,
            shadows: true,
            ambient_occlusion: true,
            ground_brightness: 0.8,
            air_density: 1.0,
            haze: 1.0,
            ozone: 1.0,
            fog_density: 0.2,
        }
    }
}

/// `Viewport`'s members in declaration order, as Newtonsoft writes them.
const VIEWPORT_MEMBERS: [&str; 19] = [
    "CameraPosition",
    "Camera",
    "RenderMode",
    "AutolevelMasks",
    "SunAltitude",
    "SunAzimuth",
    "SunIntensity",
    "AmbientOcclusion",
    "Shadows",
    "AirDensity",
    "AmbientIntensity",
    "Exposure",
    "FogDensity",
    "GroundBrightness",
    "Haze",
    "Ozone",
    "OverlayOpacity",
    "OverlayImage",
    "CustomOverlayFile",
];

/// The lighting's `float` members with the value Newtonsoft leaves out and
/// fills in (`[DefaultValue]`).
const LIGHTING_FLOATS: [(&str, f32); 10] = [
    ("SunAltitude", 33.0),
    ("SunAzimuth", 45.0),
    ("SunIntensity", 1.0),
    ("AirDensity", 1.0),
    ("AmbientIntensity", 1.0),
    ("Exposure", 1.0),
    ("FogDensity", 0.2),
    ("GroundBrightness", 0.8),
    ("Haze", 1.0),
    ("Ozone", 1.0),
];

impl ViewportLighting {
    /// The lighting panel's own defaults (`Reset Sky`, `ResetToDefaults`):
    /// a new project's, but with the sun at 327° and 25°.
    pub fn panel_defaults() -> Self {
        Self {
            sun_azimuth: 327.0,
            sun_elevation: 25.0,
            ..Self::default()
        }
    }

    fn float(&self, name: &str) -> f32 {
        match name {
            "SunAltitude" => self.sun_elevation,
            "SunAzimuth" => self.sun_azimuth,
            "SunIntensity" => self.sun_intensity,
            "AirDensity" => self.air_density,
            "AmbientIntensity" => self.ambient_intensity,
            "Exposure" => self.exposure,
            "FogDensity" => self.fog_density,
            "GroundBrightness" => self.ground_brightness,
            "Haze" => self.haze,
            _ => self.ozone,
        }
    }

    fn float_mut(&mut self, name: &str) -> &mut f32 {
        match name {
            "SunAltitude" => &mut self.sun_elevation,
            "SunAzimuth" => &mut self.sun_azimuth,
            "SunIntensity" => &mut self.sun_intensity,
            "AirDensity" => &mut self.air_density,
            "AmbientIntensity" => &mut self.ambient_intensity,
            "Exposure" => &mut self.exposure,
            "FogDensity" => &mut self.fog_density,
            "GroundBrightness" => &mut self.ground_brightness,
            "Haze" => &mut self.haze,
            _ => &mut self.ozone,
        }
    }

    /// The lighting a saved `Viewport` holds. Members left out take their
    /// `[DefaultValue]`, or false for the two switches (Newtonsoft's
    /// `IgnoreAndPopulate`).
    pub fn from_state(viewport: Option<&JsonMap<String, Value>>) -> Self {
        let mut lighting = Self::default();
        for (name, default) in LIGHTING_FLOATS {
            let saved = viewport.and_then(|viewport| viewport.get(name)).and_then(Value::as_f64);
            *lighting.float_mut(name) = saved.map(|value| value as f32).unwrap_or(default);
        }
        let switch = |name: &str| viewport.and_then(|viewport| viewport.get(name)).and_then(Value::as_bool).unwrap_or(false);
        lighting.shadows = switch("Shadows");
        lighting.ambient_occlusion = switch("AmbientOcclusion");
        lighting
    }

    /// Writes the lighting into a saved `Viewport` (`ViewportArea.UpdateSky`),
    /// leaving out what is at its default.
    pub fn write_to(&self, viewport: &mut JsonMap<String, Value>) {
        for (name, default) in LIGHTING_FLOATS {
            let value = self.float(name);
            if value == default {
                viewport.remove(name);
            } else {
                viewport.insert(name.to_string(), f32_value(value));
            }
        }
        for (name, value) in [("Shadows", self.shadows), ("AmbientOcclusion", self.ambient_occlusion)] {
            if value {
                viewport.insert(name.to_string(), Value::Bool(true));
            } else {
                viewport.remove(name);
            }
        }
        order_viewport_members(viewport);
    }
}

/// Writes `Viewport.RenderMode` (left out at `None`, the type's default).
pub fn write_render_mode(viewport: &mut JsonMap<String, Value>, mode: RenderMode) {
    if mode == RenderMode::Data {
        viewport.remove("RenderMode");
    } else {
        viewport.insert("RenderMode".to_string(), Value::from(mode.state_name()));
    }
    order_viewport_members(viewport);
}

/// Writes `Viewport.AutolevelMasks`: whether masks are enhanced at all.
pub fn write_autolevel_masks(viewport: &mut JsonMap<String, Value>, enhancement: MaskEnhancement) {
    if enhancement == MaskEnhancement::None {
        viewport.remove("AutolevelMasks");
    } else {
        viewport.insert("AutolevelMasks".to_string(), Value::Bool(true));
    }
    order_viewport_members(viewport);
}

/// Puts the members in the order Newtonsoft writes them: `$id`, the
/// declared members, then anything else as it was.
pub fn order_viewport_members(viewport: &mut JsonMap<String, Value>) {
    let mut ordered = JsonMap::new();
    if let Some(id) = viewport.remove("$id") {
        ordered.insert("$id".to_string(), id);
    }
    for name in VIEWPORT_MEMBERS {
        if let Some(value) = viewport.remove(name) {
            ordered.insert(name.to_string(), value);
        }
    }
    let rest = std::mem::take(viewport);
    ordered.extend(rest);
    *viewport = ordered;
}

/// What `ResolveColorfield` found to drape over the terrain.
#[derive(Clone)]
pub enum Colorfield {
    /// The previewed port's own map; a mask gets the mask treatment.
    Port(Arc<Map>),
    /// `Node.GetCustomColorMap`: the node's own colours, shown as they are.
    Custom(Arc<Map>),
}

/// What one render needs: `RendererBase.GetRender(node, port)`'s inputs,
/// resolved by the editor.
#[derive(Clone)]
pub struct RenderJob {
    pub generation: u64,
    /// `ResolveHeightfield`: what shapes the terrain.
    pub heightfield: Arc<Map>,
    /// `ResolveColorfield` before its mask treatment: the previewed map
    /// when it is a mask, colours or raw data, or the node's own colours.
    pub colorfield: Option<Colorfield>,
    pub intent: RenderIntent,
    pub mode: RenderMode,
    /// How the realistic renderer lights it.
    pub quality: RenderQuality,
    pub mask_enhancement: MaskEnhancement,
    pub mask_gradient: MaskGradient,
    pub lighting: ViewportLighting,
    /// The terrain's width and height in metres (how steep it is).
    pub terrain: (f32, f32),
    /// The overlay laid over the render (`B.ShowOverlay`), if any.
    pub overlay: Option<crate::overlays::OverlayJob>,
}

impl RenderJob {
    /// Whether the realistic renderer lights it at less than its `Normal`
    /// quality.
    pub fn lit_quickly(&self) -> bool {
        self.mode == RenderMode::Realistic && self.quality != RenderQuality::Normal
    }
}

/// A finished render: the terrain and what to drape over it (nothing when
/// there is nothing to send).
pub struct RenderedView {
    pub generation: u64,
    pub heights: Arc<Map>,
    pub colours: Option<Arc<TerrainColours>>,
}

/// `RendererBase.GetRender(ref heightfield, colorfield, intent)`, with
/// `ResolveColorfield`'s mask treatment: the colours the viewport shows.
pub fn render(job: &RenderJob) -> Option<TerrainColours> {
    let colorfield = job.colorfield.as_ref().map(|colorfield| resolve_colorfield(colorfield, job));
    // Masks and raw data bypass the renderer.
    if matches!(job.intent, RenderIntent::Mask | RenderIntent::Raw) {
        let colours = colorfield?;
        let shown = if colours.channel_count() == 1 { grey(&colours) } else { colours };
        return Some(pack_bytes(&shown));
    }
    let colorfield = colorfield.map(|colours| if colours.channel_count() == 1 { grey(&colours) } else { colours });
    let rendered = match job.mode {
        RenderMode::Data => raw_render(&job.heightfield, colorfield),
        RenderMode::Clay => clay_render(&job.heightfield, colorfield, job.terrain),
        RenderMode::Realistic => realistic_render(&job.heightfield, colorfield.as_ref(), &job.lighting, job.terrain, job.quality),
    };
    let overlaid = match &job.overlay {
        Some(overlay) => crate::overlays::apply(rendered, overlay),
        None => rendered,
    };
    Some(pack_bytes(&overlaid))
}

/// `ResolveColorfield`'s treatment of masks: autolevelled or equalized as
/// chosen, then coloured by the mask gradient (`Clut`). A node's own
/// colours replace all that.
fn resolve_colorfield(colorfield: &Colorfield, job: &RenderJob) -> Map {
    let map = match colorfield {
        Colorfield::Custom(colours) => return colours.as_ref().clone(),
        Colorfield::Port(map) => map.as_ref(),
    };
    if job.intent != RenderIntent::Mask {
        return map.clone();
    }
    let enhanced = match job.mask_enhancement {
        MaskEnhancement::None => map.clone(),
        MaskEnhancement::Autolevel => maths::autolevel(map.clone()),
        MaskEnhancement::Equalize => histogram::equalize(map.clone()),
    };
    let stops = job.mask_gradient.stops();
    colour::colorize(&enhanced, &stops)
}

/// A map's first channel as grey RGB.
fn grey(map: &Map) -> Map {
    let plane = map.channel(0).to_vec();
    let channels = vec![plane.clone(), plane.clone(), plane];
    Map::from_channels(map.resolution(), channels)
}

/// `RawRenderer`: the colours as they are, or the heights in grey.
fn raw_render(heightfield: &Map, colorfield: Option<Map>) -> Map {
    colorfield.unwrap_or_else(|| grey(heightfield))
}

/// `ClayRenderer`: occlusion alone, from a sky straight above, given more
/// contrast and stretched; over the colours when there are some.
fn clay_render(heightfield: &Map, colorfield: Option<Map>, terrain: (f32, f32)) -> Map {
    let rig = LightRig {
        automatic_colors: true,
        shadow_quality: 1,
        hybrid_ao_octaves: 1,
        hybrid_ao_quality: 2,
        sun_angular_diameter: 0.2,
        direct_lighting: false,
        shadows: false,
        ambient_lighting: true,
        ambient_occlusion: true,
        ao_model: AmbientOcclusionModel::Hybrid,
        ..LightRig::default()
    };
    let sky = rig.sky(33.0, 90.0);
    let maps = rig.shade(&sky, heightfield, None, terrain);
    let mut occlusion = maps.occlusion;
    maths::strengthen(&mut occlusion, -0.75);
    let shading = maths::autolevel(occlusion);
    match colorfield {
        Some(colours) => colours.multiply(&shading),
        None => grey(&shading),
    }
}

/// `RealisticRenderer`: the lighting rig from the viewport's lighting, sun
/// and sky at the quality asked for. `Normal` and `Fast` turn shadows and
/// occlusion on whatever the lighting says.
fn realistic_render(heightfield: &Map, colorfield: Option<&Map>, lighting: &ViewportLighting, terrain: (f32, f32), quality: RenderQuality) -> Map {
    let rig = realistic_rig(lighting, quality);
    let sky = rig.sky(lighting.sun_azimuth, lighting.sun_elevation);
    let maps = rig.shade(&sky, heightfield, colorfield, terrain);
    maps.color
}

/// The Constructor's `RealisticRenderer`s: `RefreshLighting(resolution)`
/// keeps the project's lighting but puts the sun at -33° and 25°; they
/// render at `Fast` quality or, `faster`, `Fastest`.
pub fn constructor_render(heightfield: &Map, colorfield: Option<&Map>, lighting: &ViewportLighting, terrain: (f32, f32), faster: bool) -> TerrainColours {
    let quality = if faster { RenderQuality::Fastest } else { RenderQuality::Fast };
    let rig = realistic_rig(lighting, quality);
    let sky = rig.sky(-33.0, 25.0);
    let maps = rig.shade(&sky, heightfield, colorfield, terrain);
    pack_bytes(&maps.color)
}

/// `B.ClearViewport`'s colours: a flat map lit by the clearing renderer
/// (hybrid occlusion, soft shadows, ground brightness 0.8, the project's
/// sun), `resolution` square. Flat, every texel lights the same, so a small
/// map is lit and its middle colour fills the rest.
pub fn cleared_colours(lighting: &ViewportLighting, resolution: usize) -> TerrainColours {
    let rig = LightRig {
        ao_model: AmbientOcclusionModel::Hybrid,
        shadow_quality: -4,
        hybrid_ao_octaves: 4,
        hybrid_ao_quality: 4,
        automatic_colors: true,
        sun_angular_diameter: 0.0,
        default_color: 0.8,
        shadows: true,
        ambient_lighting: true,
        ambient_occlusion: true,
        sun_intensity: 1.0,
        ambient_intensity: 1.0,
        ..LightRig::default()
    };
    // `CreateRenderer`: no azimuth is -33; no altitude puts 25 in the
    // azimuth instead.
    let mut azimuth = lighting.sun_azimuth;
    let altitude = lighting.sun_elevation;
    if azimuth == 0.0 {
        azimuth = -33.0;
    }
    if altitude == 0.0 {
        azimuth = 25.0;
    }
    let sky = rig.sky(azimuth, altitude);
    let lit_side = 32;
    let flat = Map::new(Resolution::square(lit_side), 1);
    let maps = rig.shade(&sky, &flat, None, (5000.0, 2500.0));
    let lit = pack_bytes(&maps.color);
    let middle = (lit_side / 2 * lit_side + lit_side / 2) * 3;
    let colour = [lit.rgb[middle], lit.rgb[middle + 1], lit.rgb[middle + 2]];
    let side = resolution.max(1);
    let rgb = colour.repeat(side * side);
    TerrainColours { width: side, height: side, rgb }
}

/// `RefreshLighting`, then the quality's switches: soft shadows and hybrid
/// occlusion at `Normal`, quick shadows and local occlusion at `Fast`,
/// neither at `Fastest`.
fn realistic_rig(lighting: &ViewportLighting, quality: RenderQuality) -> LightRig {
    let mut rig = normal_rig(lighting);
    match quality {
        RenderQuality::Fastest => {
            rig.ambient_occlusion = false;
            rig.shadows = false;
        }
        RenderQuality::Fast => {
            rig.ao_model = AmbientOcclusionModel::Local;
            rig.shadow_quality = -1;
        }
        RenderQuality::Normal => {}
    }
    rig
}

/// `RefreshLighting` at `Normal`: the viewport's lighting with soft
/// shadows and hybrid occlusion.
fn normal_rig(lighting: &ViewportLighting) -> LightRig {
    LightRig {
        ao_model: AmbientOcclusionModel::Hybrid,
        sun_intensity: lighting.sun_intensity,
        ambient_intensity: lighting.ambient_intensity,
        exposure: lighting.exposure,
        ambient_occlusion: true,
        shadows: true,
        default_color: lighting.ground_brightness,
        air_density: lighting.air_density * 0.01,
        aerosols: lighting.haze * 0.05,
        ozone: lighting.ozone * 0.0025,
        shadow_quality: -4,
        hybrid_ao_octaves: 4,
        hybrid_ao_quality: 4,
        automatic_colors: true,
        sun_angular_diameter: 0.0,
        direct_lighting: true,
        ambient_lighting: true,
        ..LightRig::default()
    }
}

/// A colour map as RGB bytes: each value
/// times 255, capped at 255 and truncated; values below zero wrap round
/// as the vector narrowing does.
fn pack_bytes(colours: &Map) -> TerrainColours {
    let width = colours.width();
    let height = colours.height();
    let red = colours.rgb_channel(0);
    let green = colours.rgb_channel(1);
    let blue = colours.rgb_channel(2);
    let mut rgb = Vec::with_capacity(width * height * 3);
    for index in 0..width * height {
        for plane in [red, green, blue] {
            rgb.push(to_byte(plane[index]));
        }
    }
    TerrainColours { width, height, rgb }
}

/// One channel's byte: `Vector.Min(v * 255, 255)` (255 for NaN, as
/// `minps` gives), `ConvertToInt32` (`int.MinValue` below range, where
/// Rust's cast saturates to the same), then the low byte.
fn to_byte(value: f32) -> u8 {
    let scaled = (value * 255.0).min(255.0);
    let whole = scaled as i32;
    whole as u8
}

/// The render thread: it renders the newest job, dropping any it never
/// got to. The web build has no threads: there the jobs wait until `poll`
/// renders the newest.
#[derive(Resource)]
pub struct ViewportRenderer {
    jobs: Sender<RenderJob>,
    views: Receiver<RenderedView>,
    #[cfg(target_arch = "wasm32")]
    waiting: (Receiver<RenderJob>, Sender<RenderedView>),
}

impl ViewportRenderer {
    pub fn start() -> Self {
        let (job_sender, job_receiver) = unbounded::<RenderJob>();
        let (view_sender, view_receiver) = unbounded::<RenderedView>();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let spawned = thread::Builder::new().name("nerthus-viewport-render".to_string()).spawn(move || {
                run(job_receiver, view_sender);
            });
            if let Err(error) = spawned {
                log::error!("cannot start the viewport render thread: {error}");
            }
        }
        Self {
            jobs: job_sender,
            views: view_receiver,
            #[cfg(target_arch = "wasm32")]
            waiting: (job_receiver, view_sender),
        }
    }

    pub fn request(&self, job: RenderJob) {
        let _ = self.jobs.send(job);
    }

    /// The newest render finished since the last call.
    pub fn poll(&self) -> Option<RenderedView> {
        #[cfg(target_arch = "wasm32")]
        if let Ok(job) = self.waiting.0.try_recv() {
            render_newest(job, &self.waiting.0, &self.waiting.1);
        }
        self.views.try_iter().last()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run(jobs: Receiver<RenderJob>, views: Sender<RenderedView>) {
    while let Ok(job) = jobs.recv() {
        if !render_newest(job, &jobs, &views) {
            return;
        }
    }
}

/// Renders `job`, or the newest of the jobs that came after it; false once
/// nothing takes the views.
fn render_newest(mut job: RenderJob, jobs: &Receiver<RenderJob>, views: &Sender<RenderedView>) -> bool {
    while let Ok(newer) = jobs.try_recv() {
        job = newer;
    }
    let colours = render(&job);
    let view = RenderedView {
        generation: job.generation,
        heights: job.heightfield.clone(),
        colours: colours.map(Arc::new),
    };
    views.send(view).is_ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn job(heightfield: Map, colorfield: Option<Map>, intent: RenderIntent, mode: RenderMode) -> RenderJob {
        RenderJob {
            generation: 1,
            heightfield: Arc::new(heightfield),
            colorfield: colorfield.map(|map| Colorfield::Port(Arc::new(map))),
            intent,
            mode,
            quality: RenderQuality::Normal,
            mask_enhancement: MaskEnhancement::None,
            mask_gradient: MaskGradient::Greyscale,
            lighting: ViewportLighting::default(),
            terrain: (5000.0, 2500.0),
            overlay: None,
        }
    }

    #[test]
    fn bytes_truncate_cap_and_wrap() {
        assert_eq!(to_byte(0.5), 127);
        assert_eq!(to_byte(1.0), 255);
        assert_eq!(to_byte(7.0), 255);
        assert_eq!(to_byte(-0.5), 129);
        assert_eq!(to_byte(f32::NAN), 255);
        assert_eq!(to_byte(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn data_mode_shows_heights_in_grey() {
        let heights = Map::from_data(Resolution::square(2), vec![0.0, 0.25, 0.5, 1.0]);
        let colours = render(&job(heights, None, RenderIntent::Heightfield, RenderMode::Data)).unwrap();
        assert_eq!(colours.rgb, vec![0, 0, 0, 63, 63, 63, 127, 127, 127, 255, 255, 255]);
    }

    #[test]
    fn masks_bypass_the_renderer_through_the_gradient() {
        let flat = Map::new(Resolution::square(2), 1);
        let mask = Map::from_data(Resolution::square(2), vec![0.0, 0.5, 0.5, 1.0]);
        let mut mask_job = job(flat, Some(mask), RenderIntent::Mask, RenderMode::Realistic);
        mask_job.mask_gradient = MaskGradient::Red;
        let colours = render(&mask_job).unwrap();
        assert_eq!(&colours.rgb[0..3], &[191, 191, 191]);
        assert_eq!(&colours.rgb[3..6], &[223, 95, 95]);
        assert_eq!(&colours.rgb[9..12], &[255, 0, 0]);

        mask_job.mask_enhancement = MaskEnhancement::Autolevel;
        let low = Map::from_data(Resolution::square(2), vec![0.2, 0.3, 0.3, 0.4]);
        mask_job.colorfield = Some(Colorfield::Port(Arc::new(low)));
        let leveled = render(&mask_job).unwrap();
        assert_eq!(leveled.rgb, colours.rgb);
    }

    #[test]
    fn custom_colours_skip_the_mask_treatment() {
        let flat = Map::new(Resolution::square(2), 1);
        let colours = Map::from_data(Resolution::square(2), vec![0.2, 0.3, 0.3, 0.4]);
        let mut mask_job = job(flat, None, RenderIntent::Mask, RenderMode::Realistic);
        mask_job.mask_gradient = MaskGradient::Red;
        mask_job.mask_enhancement = MaskEnhancement::Autolevel;
        mask_job.colorfield = Some(Colorfield::Custom(Arc::new(colours)));
        let shown = render(&mask_job).unwrap();
        assert_eq!(&shown.rgb[0..6], &[51, 51, 51, 76, 76, 76]);
    }

    #[test]
    fn quick_lighting_trades_occlusion_and_shadows() {
        let lighting = ViewportLighting { shadows: false, ambient_occlusion: false, ..ViewportLighting::default() };
        let normal = realistic_rig(&lighting, RenderQuality::Normal);
        assert_eq!((normal.ao_model, normal.shadow_quality), (AmbientOcclusionModel::Hybrid, -4));
        assert!(normal.ambient_occlusion && normal.shadows);
        let fast = realistic_rig(&lighting, RenderQuality::Fast);
        assert_eq!((fast.ao_model, fast.shadow_quality), (AmbientOcclusionModel::Local, -1));
        assert!(fast.ambient_occlusion && fast.shadows);
        let fastest = realistic_rig(&lighting, RenderQuality::Fastest);
        assert!(!fastest.ambient_occlusion && !fastest.shadows);

        let mut quick = job(Map::new(Resolution::square(2), 1), None, RenderIntent::Heightfield, RenderMode::Realistic);
        quick.quality = RenderQuality::Fast;
        assert!(quick.lit_quickly());
        quick.mode = RenderMode::Clay;
        assert!(!quick.lit_quickly());
    }

    #[test]
    fn a_mask_without_a_map_sends_nothing() {
        let flat = Map::new(Resolution::square(2), 1);
        assert!(render(&job(flat, None, RenderIntent::Mask, RenderMode::Clay)).is_none());
    }

    #[test]
    fn hot_gradient_reads_its_hex_colours() {
        let stops = MaskGradient::Hot.stops();
        assert_eq!(stops.len(), 9);
        assert_eq!(stops[1], [0x1c as f32 / 255.0, 0x14 as f32 / 255.0, 0x7f as f32 / 255.0]);
        assert_eq!(MaskGradient::from_index(2), MaskGradient::Red);
        assert_eq!(MaskGradient::from_index(9), MaskGradient::Greyscale);
    }

    #[test]
    fn lighting_reads_and_writes_like_newtonsoft() {
        let saved = json!({"$id": "7", "Camera": {}, "RenderMode": "Realistic", "SunAltitude": 17.588236, "AmbientOcclusion": true});
        let viewport = saved.as_object().unwrap();
        let lighting = ViewportLighting::from_state(Some(viewport));
        assert_eq!(lighting.sun_elevation, 17.588236);
        assert_eq!(lighting.sun_azimuth, 45.0);
        assert!(lighting.ambient_occlusion);
        assert!(!lighting.shadows);
        assert_eq!(RenderMode::from_state(viewport.get("RenderMode")), Some(RenderMode::Realistic));
        assert_eq!(RenderMode::from_state(None), Some(RenderMode::Data));
        assert_eq!(RenderMode::from_state(Some(&json!("Mesh"))), None);

        let mut written = viewport.clone();
        let changed = ViewportLighting {
            sun_azimuth: 327.0,
            shadows: true,
            ..lighting
        };
        changed.write_to(&mut written);
        write_render_mode(&mut written, RenderMode::Clay);
        let keys: Vec<&str> = written.keys().map(String::as_str).collect();
        assert_eq!(keys, ["$id", "Camera", "RenderMode", "SunAltitude", "SunAzimuth", "AmbientOcclusion", "Shadows"]);
        assert_eq!(written["SunAzimuth"], json!(327.0));
        assert_eq!(written["SunAltitude"], json!(17.588236));
        assert_eq!(written["RenderMode"], json!("Clay"));

        ViewportLighting::default().write_to(&mut written);
        write_render_mode(&mut written, RenderMode::Data);
        let keys: Vec<&str> = written.keys().map(String::as_str).collect();
        assert_eq!(keys, ["$id", "Camera", "AmbientOcclusion", "Shadows"]);
    }

    #[test]
    fn a_cleared_viewport_is_one_lit_colour() {
        let lighting = ViewportLighting::default();
        let cleared = cleared_colours(&lighting, 8);
        assert_eq!((cleared.width, cleared.height), (8, 8));
        assert_eq!(cleared.rgb.len(), 8 * 8 * 3);
        let first = &cleared.rgb[..3];
        assert!(cleared.rgb.chunks(3).all(|pixel| pixel == first));
        // Lit, not black: ground brightness 0.8 under the sun and sky.
        assert!(first.iter().any(|channel| *channel > 64), "{first:?}");
        // Flat ground lights the same whatever the size it is lit at.
        let small = Map::new(Resolution::square(32), 1);
        let large = Map::new(Resolution::square(64), 1);
        let rig = LightRig { ao_model: AmbientOcclusionModel::Hybrid, default_color: 0.8, ..LightRig::default() };
        let sky = rig.sky(-33.0, 25.0);
        let small_lit = pack_bytes(&rig.shade(&sky, &small, None, (5000.0, 2500.0)).color);
        let large_lit = pack_bytes(&rig.shade(&sky, &large, None, (5000.0, 2500.0)).color);
        assert_eq!(small_lit.rgb[..3], large_lit.rgb[..3]);
        assert!(large_lit.rgb.chunks(3).all(|pixel| pixel == &large_lit.rgb[..3]));
    }
}
