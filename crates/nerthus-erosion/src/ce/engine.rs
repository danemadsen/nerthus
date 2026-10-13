//! The erosion driver: `ErosionWrapper.ErodeTerrain` set-up,
//! `Erosion::Erosion`, `Erosion::erode_terrain` and the per-level loop of
//! the CE engine.
//!
//! Droplets may run on several threads unless "Deterministic" is ticked;
//! this port always runs the deterministic single-threaded schedule.

use super::droplet::run_droplet;
use super::thermal::{apply_sediment, copy_sediment, fine_detail, thermal_pass};
use super::kernel::{kernel_sum, FloatMap};
use super::layer::{removal_power, Layer, U16Map};

/// The engine's linear congruential generator step.
#[inline]
pub(crate) fn lcg(state: u32) -> u32 {
    state.wrapping_mul(0x5391_313d).wrapping_add(0x22fc_2945)
}

/// `2^-32`, turning a generator state into [0, 1).
pub(crate) const RANDOM_SCALE: f32 = 2.328_306_4e-10;

/// The arguments of `ErosionWrapper.SetVars` (the overload with sediment
/// removal) that reach the engine.
///
/// Left out: rivers mode and its settings (no node enables it), the
/// write-only `invert_mask`, and the rock angle, which thermal model 1 never
/// reads. Thermal erosion always runs model 1, the only one the nodes
/// select.
#[derive(Clone, Debug)]
pub struct ErosionSettings {
    pub seed: i32,
    /// Terrain width in world units.
    pub size: f64,
    /// Feature scale in world units.
    pub scale: f64,
    pub duration: f64,
    pub rock_softness: f64,
    pub fluvial_erosion_strength: f64,
    pub downcutting: f64,
    pub base_level: f64,
    pub retardation_of_downcutting: f64,
    pub enable_thermal: bool,
    pub thermal_erosion_strength: f64,
    pub talus_angle: f64,
    pub thermal_size: f64,
    pub flow_volume: f64,
    pub mask_by_shader: bool,
    pub masking_mode: i32,
    pub mask_as_thermal_erosion_strength: bool,
    pub sediment_removal: f64,
    pub laminar_flow: bool,
    pub randomness: f64,
    pub thermal_smoothness: f64,
    pub talus_settling: f64,
    pub thermal_detail: bool,
    pub debris_size: f64,
    /// Vertical range the [0, 1] heights are scaled to while eroding.
    pub height_range: f64,
}

impl Default for ErosionSettings {
    fn default() -> Self {
        Self {
            seed: 0,
            size: 5000.0,
            scale: 2000.0,
            duration: 0.04,
            rock_softness: 0.65,
            fluvial_erosion_strength: 0.5,
            downcutting: 0.1,
            base_level: 0.0,
            retardation_of_downcutting: 0.5,
            enable_thermal: false,
            thermal_erosion_strength: 0.0,
            talus_angle: 30.0,
            thermal_size: 0.0,
            flow_volume: 0.0,
            mask_by_shader: false,
            masking_mode: 4,
            mask_as_thermal_erosion_strength: false,
            sediment_removal: 0.0,
            laminar_flow: false,
            randomness: 0.0,
            thermal_smoothness: 0.0,
            talus_settling: 0.0,
            thermal_detail: false,
            debris_size: 0.0,
            height_range: 2600.0,
        }
    }
}

/// The wrapper's outputs, rescaled to [0, 1] heights.
#[derive(Clone, Debug)]
pub struct ErosionOutput {
    pub height: Vec<f32>,
    pub deposit: Vec<f32>,
    pub wear: Vec<f32>,
    /// Droplet visits, `count * 0.0025`.
    pub flow: Vec<f32>,
}

/// Engine state (`Erosion`).
pub(crate) struct Erosion {
    pub iteration: i32,
    pub random_state: u32,
    /// 32 random unit vectors.
    pub directions: [[f32; 2]; 32],
    pub thermal_scale: f32,
    /// The next level run is the first one.
    pub first_level: bool,
    /// +912: 1 scales erosion strength, 2 rock softness, 3 droplet spawn
    /// chance (precipitation), 4 no mask.
    pub masking_mode: i32,
    /// +920
    pub size: f64,
    /// +928
    pub resolution: i32,
    /// +944
    pub iterations: i32,
    /// +952
    pub rock_softness: f64,
    /// +976: log2 of the feature scale in cells.
    pub scale_levels: f64,
    /// +984: half the fluvial strength.
    pub strength: f64,
    /// The last of a thermal iteration's ten sub-steps.
    pub last_substep: bool,
    /// Talus settling sub-steps near the end of the run.
    pub settling: bool,
    /// The finishing thermal passes after the main loop.
    pub finishing: bool,
    /// +992
    pub thermal_strength: f64,
    /// +1000
    pub thermal_enabled: bool,
    /// +1008
    pub thermal_levels: f64,
    /// +1024: tan(talus angle).
    pub talus_slope: f64,
    /// +1032: `talus_settling * 128` sub-steps.
    pub talus_settling: i32,
    /// +1048
    pub thermal_size: f64,
    /// +1040
    pub downcutting: f64,
    /// +1064: `base_level^2 + 0.003`.
    pub base_level: f64,
    /// +1072
    pub flow_volume: f64,
    /// +1080
    pub randomness: f64,
    /// +1088
    pub laminar_flow: bool,
    /// +1144
    pub seed: i32,
    /// +1152: the mask scales thermal weathering.
    pub mask_as_thermal: bool,
    /// +1160: anisotropy.
    pub thermal_smoothness: f64,
    /// +1168: retardation of downcutting.
    pub inhibition: f64,
    /// +1176 (always 0 from the wrapper).
    pub debris: f64,
    /// +1184
    pub thermal_detail: bool,
    /// +1192: fine-detail debris size in world units.
    pub debris_size: f64,
    /// +1200
    pub height_range: f64,
    /// +1208
    pub sediment_removal: bool,
    /// Global kernel sum written by the constructor.
    pub kernel_sum: f32,
}

impl Erosion {
    /// `Erosion::Erosion`: random jitter directions from a generator seeded
    /// with 7. (The 128 random values it draws next are only read in rivers
    /// mode.)
    fn new() -> Self {
        let mut state = 7u32;
        let mut directions = [[0.0f32; 2]; 32];
        for direction in directions.iter_mut() {
            let first = lcg(state);
            state = lcg(first);
            let a = first as f32 * RANDOM_SCALE;
            let x = (a + a) - 1.0;
            let b = state as f32 * RANDOM_SCALE;
            let y = (b + b) - 1.0;
            let length = (x * x + y * y).sqrt();
            direction[1] = (1.0 / length) * y;
            direction[0] = (1.0 / length) * x;
        }
        Self {
            iteration: 0,
            random_state: 0,
            directions,
            thermal_scale: 1.0,
            first_level: true,
            masking_mode: 4,
            size: 0.0,
            resolution: 0,
            iterations: 0,
            rock_softness: 0.0,
            scale_levels: 1.0,
            strength: 0.0,
            last_substep: false,
            settling: false,
            finishing: false,
            thermal_strength: 0.0,
            thermal_enabled: false,
            thermal_levels: 1.0,
            talus_slope: 0.0,
            talus_settling: 0,
            thermal_size: 0.0,
            downcutting: 0.0,
            base_level: 0.0,
            flow_volume: 0.0,
            randomness: 0.0,
            laminar_flow: false,
            seed: 0,
            mask_as_thermal: false,
            thermal_smoothness: 0.0,
            inhibition: 0.0,
            debris: 0.0,
            thermal_detail: false,
            debris_size: 0.0,
            height_range: 1.0,
            sediment_removal: false,
            kernel_sum: kernel_sum(),
        }
    }

    /// Clears the thermal scratch and flow counts.
    fn clear_counts(layer: &mut Layer) {
        if let Some(thermal) = &mut layer.thermal {
            thermal.data.fill(0.0);
        }
        layer.flow.data.fill(0);
    }

    /// Random height noise along row `y`'s droplet trails, then the trail
    /// counts cleared.
    fn scatter_debris(&self, layer: &mut Layer, y: i32) {
        let size = layer.resolution;
        let hash = ((((y as u32) ^ 0x811c_9dc5).wrapping_mul(0x0100_0193) ^ self.seed as u32).wrapping_mul(0x0100_0193) ^ size as u32)
            .wrapping_mul(0xca90_8307)
            .wrapping_add(0x22fc_2945);
        let mut state = hash;
        let trail = layer.trail.as_mut().expect("trail map allocated");
        for x in 0..size {
            let index = (y * trail.size + x) as usize;
            if 0.0 < self.randomness {
                state = lcg(state);
                let count = trail.data[index];
                if count != 0 && count < 10 {
                    let amplitude = (layer.cell * 0.1 * count as f32) as f64 * self.randomness * self.strength * self.rock_softness;
                    let offset = amplitude as f32 as f64 * (state as f64 * 2.3283064365386963e-10 - 0.5);
                    let height_index = layer.height.index(x, y);
                    layer.height.data[height_index] += offset as f32;
                }
            }
            trail.data[index] = 0;
        }
    }

    /// One thermal sub-step: the talus pass over every row (in the engine's
    /// interleaved job order), then the sediment it moved is applied.
    fn thermal_step(&mut self, layer: &mut Layer) {
        for job in 0..layer.resolution {
            thermal_pass(self, layer, job);
        }
        apply_sediment(layer);
    }

    /// The iterations run at one pyramid level.
    fn run_level(&mut self, layer: &mut Layer) {
        let mut thermal_fraction = 1.0f32;
        if self.first_level && (layer.level as f64) < self.thermal_levels {
            thermal_fraction = if self.thermal_levels == 1.0 {
                1.0
            } else {
                (self.thermal_levels - (self.thermal_levels as i32) as f64) as f32
            };
            self.first_level = false;
        }
        let thermal_iterations = (self.iterations as f32 * thermal_fraction) as i32;
        let tail = 0.47124f64 - self.thermal_levels;
        let power = 2.0f32.powf(tail as f32);
        // The CE engine's own constant (ln 2 to single precision).
        #[allow(clippy::approx_constant)]
        let scaled = (power / 0.6931472f32) as f64;
        layer.thermal_factor = 1.0 / (2.000004f64 - scaled) as f32;
        if self.flow_volume > 0.0 || self.randomness > 0.0 || self.debris > 0.0 {
            layer.trail = Some(U16Map::new(layer.resolution));
        }
        Self::clear_counts(layer);
        let per_side = layer.resolution / 8;
        let droplets = per_side * per_side;
        self.random_state = lcg(self.seed as u32);
        let iterations = self.iterations;
        let droplet_iterations = (iterations as f32 * layer.fraction) as i32;
        if self.thermal_enabled {
            copy_sediment(layer);
        }
        self.finishing = false;
        self.iteration = 0;
        while self.iteration < iterations {
            if 0.0 < self.strength && self.iteration < droplet_iterations {
                for droplet in 0..droplets {
                    run_droplet(self, layer, droplet);
                }
            }
            if self.thermal_enabled && self.iteration < thermal_iterations && (layer.level as f64) < self.thermal_levels {
                for substep in 0..10 {
                    self.settling = (iterations * 10 - self.talus_settling) - 1 <= substep + self.iteration * 10;
                    self.last_substep = substep == 9;
                    self.thermal_step(layer);
                }
            }
            if 0.0 < self.strength && (0.0 < self.flow_volume || 0.0 < self.randomness || 0.0 < self.debris) && self.iteration < droplet_iterations - 1 {
                for y in 0..layer.resolution {
                    self.scatter_debris(layer, y);
                }
            }
            self.iteration += 1;
        }
        if self.thermal_enabled {
            self.finishing = true;
            self.last_substep = false;
            for _ in 0..4 {
                self.thermal_step(layer);
            }
            self.last_substep = true;
            for _ in 0..3 {
                self.thermal_step(layer);
            }
            if layer.level == 0 {
                self.iteration = -1;
            }
            self.thermal_step(layer);
        }
        let level = layer.level as f64;
        if level < self.thermal_levels {
            let decay = ((self.thermal_levels - level) * -0.6931471824645996).exp() as f32;
            self.thermal_scale = if decay <= 0.1 { 0.1 } else { decay };
        }
        layer.trail = None;
    }

    /// Erodes a half-resolution copy of `fine` (recursively
    /// coarser while the feature scale allows), then rebuilds `fine` from it.
    fn run_coarse_levels(&mut self, fine: &mut Layer) {
        let mut coarse = fine.coarser(self.thermal_enabled, self.scale_levels, self.kernel_sum);
        fine.store_residual(&coarse);
        if (coarse.level as f64) < self.scale_levels - 1.0 && 0xb < coarse.resolution {
            self.run_coarse_levels(&mut coarse);
        }
        self.run_level(&mut coarse);
        fine.restore_from(&coarse);
    }

    /// Applies sediment removal to row `y` and folds its loose sediment into
    /// the deposit map.
    fn finish_row(&self, layer: &mut Layer, y: i32) {
        for x in 0..layer.resolution {
            let index = layer.height.index(x, y);
            if self.sediment_removal {
                let mut sediment = layer.sediment.data[index];
                let mut deposit = layer.deposit.data[index];
                let removal = layer.removal.as_ref().expect("removal map present");
                let strength = removal_power(removal.data[index]);
                let mut height = layer.height.data[index];
                if 0.0 < strength && 0.0 < deposit + sediment {
                    height -= (deposit + sediment) * strength;
                    sediment = sediment - strength * sediment;
                    deposit = deposit - strength * deposit;
                }
                layer.height.data[index] = height;
                layer.deposit.data[index] = if deposit <= 0.0 { 0.0 } else { deposit };
                layer.sediment.data[index] = if sediment <= 0.0 { 0.0 } else { sediment };
            }
            layer.deposit.data[index] += layer.sediment.data[index];
        }
    }

    /// `Erosion::erode_terrain`.
    fn erode_terrain(&mut self, layer: &mut Layer) {
        layer.cell = (self.size / self.resolution as f64) as f32;
        self.first_level = true;
        self.thermal_scale = 1.0;
        if 1.0 < self.scale_levels {
            self.run_coarse_levels(layer);
        }
        self.run_level(layer);
        if self.thermal_enabled && self.thermal_detail {
            for row in 0..layer.resolution {
                fine_detail(self, layer, row);
            }
        }
        for y in 0..layer.resolution {
            self.finish_row(layer, y);
        }
    }
}

fn to_f32(value: f64) -> f32 {
    value as f32
}

/// `ErosionWrapper.ErodeTerrain(heightmap, mask, sedimentRemovalMask,
/// fixed: true)`. `heightmap` holds [0, 1] heights; `mask` and `removal` the
/// optional selective-processing and sediment-removal maps.
pub fn erode(settings: &ErosionSettings, heightmap: &[f32], mask: Option<&[f32]>, removal: Option<&[f32]>) -> ErosionOutput {
    let resolution = (heightmap.len() as f64).sqrt() as i32;
    let mut erosion = Erosion::new();
    erosion.size = settings.size;
    erosion.resolution = resolution;
    let mut layer = Layer::new(0, resolution, settings.enable_thermal);
    layer.cell = to_f32(settings.size / resolution as f64);
    erosion.seed = settings.seed;
    if settings.mask_by_shader && mask.is_some() {
        erosion.masking_mode = settings.masking_mode;
        if settings.masking_mode != 0 {
            erosion.mask_as_thermal = settings.mask_as_thermal_erosion_strength;
        }
    } else {
        erosion.masking_mode = 4;
    }
    erosion.thermal_detail = settings.thermal_detail;
    erosion.debris_size = settings.debris_size;
    erosion.iterations = (settings.duration * 256.0) as i32;
    erosion.downcutting = settings.downcutting;
    erosion.inhibition = settings.retardation_of_downcutting;
    erosion.base_level = settings.base_level * settings.base_level + 0.003;
    erosion.rock_softness = settings.rock_softness;
    erosion.flow_volume = settings.flow_volume;
    erosion.randomness = settings.randomness;
    erosion.laminar_flow = settings.laminar_flow;
    let scale_ratio = settings.scale * 0.5 / (settings.size / resolution as f64);
    erosion.scale_levels = scale_ratio.ln() * std::f64::consts::LOG2_E;
    erosion.strength = settings.fluvial_erosion_strength * 0.5;
    erosion.thermal_strength = settings.thermal_erosion_strength;
    erosion.thermal_enabled = settings.enable_thermal;
    let talus_angle = if settings.talus_angle < 89.9990005493164 { settings.talus_angle } else { 89.9990005493164 };
    erosion.talus_slope = (talus_angle * 0.0174532925199433).tan();
    erosion.talus_settling = (settings.talus_settling * 128.0) as i32;
    erosion.thermal_smoothness = settings.thermal_smoothness;
    let thermal_size = if settings.thermal_size > 1e-06 { settings.thermal_size } else { 1e-06 };
    let thermal_ratio = thermal_size * 0.5 / (settings.size / resolution as f64);
    let thermal_levels = thermal_ratio.ln() * std::f64::consts::LOG2_E;
    erosion.thermal_levels = if 1.0 > thermal_levels { 1.0 } else { thermal_levels };
    erosion.thermal_size = thermal_size;
    erosion.debris = 0.0;
    erosion.height_range = settings.height_range;
    erosion.sediment_removal = settings.sediment_removal > 0.0 && removal.is_some();

    let height_range = settings.height_range;
    for (target, value) in layer.height.data.iter_mut().zip(heightmap) {
        *target = (*value as f64 * height_range) as f32;
    }
    if erosion.masking_mode != 4 {
        let values = mask.expect("mask present when masking");
        layer.mask = Some(FloatMap { size: resolution, data: values.to_vec() });
    }
    if erosion.sediment_removal {
        let values = removal.expect("removal map present");
        let mut map = FloatMap::new(resolution, 0.0);
        for (target, value) in map.data.iter_mut().zip(values) {
            let clamped = if *value < 1.0 {
                if *value > 0.0 { *value } else { 0.0 }
            } else {
                1.0
            };
            *target = (settings.sediment_removal * clamped as f64) as f32;
        }
        layer.removal = Some(map);
    }

    erosion.erode_terrain(&mut layer);

    let inverse = 1.0 / height_range;
    let rescale = |values: &[f32]| -> Vec<f32> { values.iter().map(|value| (*value as f64 * inverse) as f32).collect() };
    let flow = layer.flow.data.iter().map(|count| (*count as f64 * 0.0024999999441206455) as f32).collect();
    ErosionOutput {
        height: rescale(&layer.height.data),
        deposit: rescale(&layer.deposit.data),
        wear: rescale(&layer.wear.data),
        flow,
    }
}
