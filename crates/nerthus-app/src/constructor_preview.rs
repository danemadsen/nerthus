//! The Constructor's previews (`ConstructionPreview` with a
//! `RealisticRenderer`): the base terrain at the preview's size with each
//! terrain placed as Construction places it (sized by its Scale, shaped,
//! turned, raised, lifted, edged by its mask, then inserted at its
//! position with its blend), shaded. Colourized, each terrain's footprint
//! is drawn in its colour. A thread renders the newest preview asked for,
//! keeping each terrain as last prepared until its settings or its input
//! change (`RefreshTerrainCache`).

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{unbounded, Receiver, Sender};
use nerthus_engine::{Map, Resolution};
use nerthus_nodes::transformer::{self, AnchorPosition, BlendMode, EdgeBehaviour, ResizeMethod};
use nerthus_nodes::{aspect, colour, masking, maths};
use nerthus_render::TerrainColours;
use serde_json::Value;

use crate::viewport_render::{constructor_render, ViewportLighting};

/// One terrain's placement (`TerrainTransform`).
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewTransform {
    /// Its input is `Terrain{index + 1}`.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub base: f32,
    pub shaper: f32,
    pub rotate: f32,
    pub scale: f32,
    pub visible: bool,
    pub blend_mode: String,
    pub mask: String,
    pub mask_bias: f32,
    pub colour: [f32; 3],
}

impl PreviewTransform {
    /// A saved transform; members left out keep the class's defaults.
    pub fn from_value(value: &Value) -> Self {
        let number = |name: &str, default: f64| value.get(name).and_then(Value::as_f64).unwrap_or(default) as f32;
        let text = |name: &str, default: &str| value.get(name).and_then(Value::as_str).unwrap_or(default).to_string();
        let colour = value.get("Color");
        let channel = |name: &str| colour.and_then(|colour| colour.get(name)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        Self {
            index: value.get("Index").and_then(Value::as_u64).unwrap_or(0) as usize,
            x: number("X", 0.0),
            y: number("Y", 0.0),
            z: number("Z", 1.0),
            base: number("Base", 0.0),
            shaper: number("Shaper", 0.0),
            rotate: number("Rotate", 0.0),
            scale: number("Scale", 1.0),
            visible: value.get("IsVisible").and_then(Value::as_bool).unwrap_or(true),
            blend_mode: text("BlendMode", "Max"),
            mask: text("Mask", "Square"),
            mask_bias: number("MaskBias", 0.25),
            colour: [channel("R"), channel("G"), channel("B")],
        }
    }
}

/// A preview to render.
#[derive(Clone)]
pub struct PreviewJob {
    pub base: Arc<Map>,
    /// The terrains by index (`Terrain1` first); none for ports without
    /// a map.
    pub terrains: Vec<Option<Arc<Map>>>,
    pub transforms: Vec<PreviewTransform>,
    pub resolution: usize,
    pub colorize: bool,
    pub faster: bool,
    /// Refresh Previews: every terrain prepared again (`InvalidateAll`).
    pub refresh: bool,
    pub lighting: ViewportLighting,
    pub terrain: (f32, f32),
}

/// A rendered preview, or why there is none.
pub struct PreviewImage {
    pub colours: Result<TerrainColours, String>,
}

/// What a prepared terrain depends on.
#[derive(Clone, PartialEq)]
struct PreparedKey {
    input: usize,
    resolution: usize,
    scale: u32,
    shaper: u32,
    rotate: u32,
    z: u32,
    base: u32,
    mask: String,
    mask_bias: u32,
}

impl PreparedKey {
    fn of(transform: &PreviewTransform, input: &Arc<Map>, resolution: usize) -> Self {
        Self {
            input: Arc::as_ptr(input) as usize,
            resolution,
            scale: transform.scale.to_bits(),
            shaper: transform.shaper.to_bits(),
            rotate: transform.rotate.to_bits(),
            z: transform.z.to_bits(),
            base: transform.base.to_bits(),
            mask: transform.mask.clone(),
            mask_bias: transform.mask_bias.to_bits(),
        }
    }
}

/// `RefreshTerrainCache` for one terrain: sized to the preview times its
/// Scale (8 at least), shaped, turned, raised, lifted and edged.
fn prepare(transform: &PreviewTransform, input: &Map, resolution: usize) -> Result<Map, String> {
    if input.channel_count() > 1 {
        return Err("This node requires heightfield input, not colors.".to_string());
    }
    let side = ((resolution as f32 * transform.scale) as usize).max(8);
    let size = Resolution::square(side);
    let mut map = if input.resolution() != size { transformer::resize_with(input, size, ResizeMethod::LinearFast) } else { input.clone() };
    if transform.shaper != 0.0 {
        maths::strengthen(&mut map, transform.shaper);
    }
    if transform.rotate.abs() > 0.001 {
        map = transformer::rotate(&map, transform.rotate, false, EdgeBehaviour::Edge, true);
    }
    if transform.z != 1.0 {
        let z = transform.z;
        map.apply(|value| value * z);
    }
    if transform.base > 0.0 {
        let lift = transform.base;
        map.apply(|value| value + lift);
    }
    let masked = match transform.mask.as_str() {
        "Square" | "Soft" => masking::mask_by_square_edge(&map, transform.mask_bias),
        "Round" => masking::vignette(&map, transform.mask_bias, 0.25),
        _ => map,
    };
    Ok(masked)
}

/// `GetBlendMode`.
fn blend_mode(name: &str) -> BlendMode {
    match name {
        "Max" => BlendMode::Max,
        "Min" => BlendMode::Min,
        "Add" => BlendMode::Add,
        "Subtract" => BlendMode::Subtract,
        _ => BlendMode::Blend,
    }
}

/// `BuildPreview`: the terrain the transforms make, and its colours when
/// colourized (each terrain where it changed the terrain, numbered from
/// 1 over the count, run through the terrains' colours).
fn composite(job: &PreviewJob, prepared: &mut HashMap<usize, (PreparedKey, Arc<Map>)>) -> Result<(Map, Option<Map>), String> {
    let size = Resolution::square(job.resolution);
    let mut terrain = transformer::resize_with(&job.base, size, ResizeMethod::LinearFast);
    let mut footprints = job.colorize.then(|| Map::new(size, 1));
    let mut colours = Vec::new();
    let count = job.transforms.len().max(1) as f32;
    let mut number = 1;
    for transform in job.transforms.iter().filter(|transform| transform.visible) {
        let Some(input) = job.terrains.get(transform.index).and_then(Option::as_ref) else {
            prepared.remove(&transform.index);
            continue;
        };
        let key = PreparedKey::of(transform, input, job.resolution);
        let current = prepared.get(&transform.index).filter(|(saved, _)| *saved == key).map(|(_, map)| map.clone());
        let map = match current {
            Some(map) => map,
            None => {
                let map = Arc::new(prepare(transform, input, job.resolution)?);
                prepared.insert(transform.index, (key, map.clone()));
                map
            }
        };
        let before = footprints.is_some().then(|| terrain.clone());
        transformer::insert(&map, &mut terrain, (transform.x, transform.y), blend_mode(&transform.blend_mode), None, AnchorPosition::Center);
        if let (Some(before), Some(footprints)) = (before, footprints.as_mut()) {
            let mut changed = before;
            for (old, new) in changed.data_mut().iter_mut().zip(terrain.data()) {
                *old = (*old - new).abs();
            }
            let touched = aspect::threshold(&changed, 0.01);
            let weight = number as f32 / count;
            for (footprint, mark) in footprints.data_mut().iter_mut().zip(touched.data()) {
                *footprint = footprint.max(mark * weight);
            }
            colours.push(transform.colour);
        }
        number += 1;
    }
    let tinted = match footprints {
        Some(footprints) if !colours.is_empty() => Some(colour::colorize(&footprints, &colours)),
        _ => None,
    };
    Ok((terrain, tinted))
}

fn render(job: &PreviewJob, prepared: &mut HashMap<usize, (PreparedKey, Arc<Map>)>) -> Result<TerrainColours, String> {
    let (terrain, tinted) = composite(job, prepared)?;
    Ok(constructor_render(&terrain, tinted.as_ref(), &job.lighting, job.terrain, job.faster))
}

/// The thread previews are rendered on. The web build has no threads:
/// there the jobs wait until `poll` renders the newest.
pub struct ConstructorRenderer {
    jobs: Sender<PreviewJob>,
    images: Receiver<PreviewImage>,
    #[cfg(target_arch = "wasm32")]
    waiting: (Receiver<PreviewJob>, Sender<PreviewImage>, std::sync::Mutex<HashMap<usize, (PreparedKey, Arc<Map>)>>),
}

impl ConstructorRenderer {
    pub fn start() -> Self {
        let (job_sender, job_receiver) = unbounded::<PreviewJob>();
        let (image_sender, image_receiver) = unbounded::<PreviewImage>();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let spawned = thread::Builder::new().name("nerthus-constructor".to_string()).spawn(move || {
                let mut prepared = HashMap::new();
                while let Ok(job) = job_receiver.recv() {
                    if !render_newest(job, &job_receiver, &image_sender, &mut prepared) {
                        return;
                    }
                }
            });
            if let Err(error) = spawned {
                log::error!("cannot start the Constructor's preview thread: {error}");
            }
        }
        Self {
            jobs: job_sender,
            images: image_receiver,
            #[cfg(target_arch = "wasm32")]
            waiting: (job_receiver, image_sender, Default::default()),
        }
    }

    pub fn request(&self, job: PreviewJob) {
        let _ = self.jobs.send(job);
    }

    /// The newest preview finished since the last call.
    pub fn poll(&self) -> Option<PreviewImage> {
        #[cfg(target_arch = "wasm32")]
        if let (Ok(job), Ok(mut prepared)) = (self.waiting.0.try_recv(), self.waiting.2.lock()) {
            render_newest(job, &self.waiting.0, &self.waiting.1, &mut prepared);
        }
        self.images.try_iter().last()
    }
}

/// Renders `job`, or the newest of the jobs that came after it; false once
/// nothing takes the images.
fn render_newest(mut job: PreviewJob, jobs: &Receiver<PreviewJob>, images: &Sender<PreviewImage>, prepared: &mut HashMap<usize, (PreparedKey, Arc<Map>)>) -> bool {
    while let Ok(newer) = jobs.try_recv() {
        job = newer;
    }
    if job.refresh {
        prepared.clear();
    }
    let colours = render(&job, prepared);
    let image = PreviewImage { colours };
    images.send(image).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(terrains: Vec<Option<Arc<Map>>>, transforms: Vec<PreviewTransform>) -> PreviewJob {
        PreviewJob {
            base: Arc::new(Map::new(Resolution::square(16), 1)),
            terrains,
            transforms,
            resolution: 16,
            colorize: true,
            faster: true,
            refresh: false,
            lighting: ViewportLighting::default(),
            terrain: (5000.0, 2500.0),
        }
    }

    #[test]
    fn terrains_are_placed_and_their_footprints_coloured() {
        let saved = json!({"X": 0.5, "Y": 0.5, "Scale": 0.5, "Mask": "None", "Index": 0, "Color": {"R": 1.0, "G": 0.0, "B": 0.0}});
        let transform = PreviewTransform::from_value(&saved);
        assert_eq!((transform.z, transform.blend_mode.as_str(), transform.colour), (1.0, "Max", [1.0, 0.0, 0.0]));
        let hill = Arc::new(Map::filled(Resolution::square(16), 1.0));
        let mut prepared = HashMap::new();
        let (terrain, tinted) = composite(&job(vec![Some(hill)], vec![transform]), &mut prepared).unwrap();
        // An 8 square in the middle.
        assert_eq!(terrain.get(8, 8), 1.0);
        assert_eq!(terrain.get(1, 1), 0.0);
        let tinted = tinted.expect("colourized");
        assert_eq!((tinted.channel(0)[8 * 16 + 8], tinted.channel(1)[8 * 16 + 8]), (1.0, 0.0));
        assert_eq!(prepared.len(), 1);
    }

    #[test]
    fn colour_inputs_are_refused() {
        let colours = Arc::new(Map::new(Resolution::square(16), 3));
        let transform = PreviewTransform::from_value(&json!({"Index": 0}));
        let mut prepared = HashMap::new();
        let result = composite(&job(vec![Some(colours)], vec![transform]), &mut prepared);
        assert_eq!(result.err().as_deref(), Some("This node requires heightfield input, not colors."));
    }
}
