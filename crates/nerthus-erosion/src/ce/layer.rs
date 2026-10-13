//! `LandscapeLayer`: one level of the erosion pyramid, and the operations
//! that move data between levels.

use super::kernel::{downsample, upsample, FloatMap};

/// A square `u16` map (`BitMap<unsigned short>`).
#[derive(Clone, Debug)]
pub struct U16Map {
    pub size: i32,
    pub data: Vec<u16>,
}

impl U16Map {
    pub fn new(size: i32) -> Self {
        let count = (size * size).max(0) as usize;
        Self { size, data: vec![0; count] }
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y * self.size + x) as usize
    }
}

/// One pyramid level (`LandscapeLayer`, 136 bytes in the CE engine).
#[derive(Clone, Debug)]
pub struct Layer {
    /// [0] terrain height (scaled by the height range).
    pub height: FloatMap,
    /// [1] thermal erosion scratch map (only with thermal erosion).
    pub thermal: Option<FloatMap>,
    /// [3] deposited sediment.
    pub deposit: FloatMap,
    /// [4] loose sediment.
    pub sediment: FloatMap,
    /// [5] material worn away.
    pub wear: FloatMap,
    /// [6] droplet visits per cell (the Flow output).
    pub flow: U16Map,
    /// [7] droplet trail counts for flow volume and randomness.
    pub trail: Option<U16Map>,
    /// [9] detail removed when the next coarser level was built.
    pub residual: Option<FloatMap>,
    /// [10] selective processing mask.
    pub mask: Option<FloatMap>,
    /// [11] sediment removal strength.
    pub removal: Option<FloatMap>,
    /// [13] per-cell visit marks (full resolution only).
    pub visits: Option<Vec<u8>>,
    pub resolution: i32,
    /// 0 for the full resolution layer.
    pub level: i32,
    /// Share of the iterations run at this level.
    pub fraction: f32,
    pub thermal_factor: f32,
    /// Cell size in world units.
    pub cell: f32,
}

impl Layer {
    /// `LandscapeLayer::LandscapeLayer(level, resolution, thermal, false)`
    /// (the river surface map only exists in rivers mode).
    pub fn new(level: i32, resolution: i32, thermal: bool) -> Self {
        let full_resolution = level == 0;
        let visits = if full_resolution { Some(vec![0u8; (resolution * resolution).max(0) as usize]) } else { None };
        Self {
            height: FloatMap::new(resolution, 0.0),
            thermal: if thermal { Some(FloatMap::new(resolution, 0.0)) } else { None },
            deposit: FloatMap::new(resolution, 0.0),
            sediment: FloatMap::new(resolution, 0.0),
            wear: FloatMap::new(resolution, 0.0),
            flow: U16Map::new(resolution),
            trail: None,
            residual: None,
            mask: None,
            removal: None,
            visits,
            resolution,
            level,
            fraction: 1.0,
            thermal_factor: 1.0,
            cell: 0.0,
        }
    }

    /// Builds the next coarser level from this one (first half of
    /// `Erosion::build_level`): half resolution, doubled
    /// cell size, kernel-downsampled height / mask / removal maps.
    pub fn coarser(&self, thermal: bool, scale_levels: f64, kernel_sum: f32) -> Layer {
        let resolution = self.resolution >> 1;
        let mut coarse = Layer::new(self.level + 1, resolution, thermal);
        let remaining = scale_levels - coarse.level as f64;
        if remaining < 1.0 {
            coarse.fraction = remaining as f32;
        }
        coarse.cell = self.cell + self.cell;
        if let Some(mask) = &self.mask {
            coarse.mask = Some(downsampled(mask, resolution, kernel_sum));
        }
        if let Some(removal) = &self.removal {
            coarse.removal = Some(downsampled(removal, resolution, kernel_sum));
        }
        coarse.height = downsampled(&self.height, resolution, kernel_sum);
        coarse
    }

    /// Stores the detail the coarse level cannot represent
    /// (this layer's height minus the upsampled coarse height).
    pub fn store_residual(&mut self, coarse: &Layer) {
        let size = self.resolution;
        let mut residual = FloatMap::new(size, 0.0);
        for y in 0..size {
            for x in 0..size {
                let smooth = upsample(&coarse.height, x as u32, y as u32);
                let index = residual.index(x, y);
                residual.data[index] = self.height.data[index] - smooth;
            }
        }
        self.residual = Some(residual);
    }

    /// Rebuilds this layer from the eroded coarse level,
    /// re-applying the stored detail and redistributing sediment.
    pub fn restore_from(&mut self, coarse: &Layer) {
        let size = self.resolution;
        let residual = self.residual.take().expect("residual stored before the coarse level ran");
        for y in 0..size as u32 {
            for x in 0..size as u32 {
                let index = self.height.index(x as i32, y as i32);
                let mut height = upsample(&coarse.height, x, y);
                let mut deposit = upsample(&coarse.deposit, x, y);
                let mut sediment = upsample(&coarse.sediment, x, y);
                let loose = sediment + deposit;
                let mut detail = residual.data[index];
                let wear = upsample(&coarse.wear, x, y);
                if 0.0 < loose {
                    let cell = coarse.cell;
                    if 0.0 < loose - detail {
                        let keep = ((cell + cell) / loose - 0.25).clamp(0.0, 1.0);
                        let moved = (keep - 1.0) * detail;
                        deposit += moved * (1.0 - sediment / loose);
                        sediment = sediment + moved * (sediment / loose);
                        detail *= keep;
                    } else {
                        let mut share = cell / wear;
                        if 1.0 <= share {
                            share = 1.0;
                        }
                        deposit = 0.0;
                        sediment = 0.0;
                        detail = share * (detail - loose);
                    }
                }
                height += detail;
                if let Some(removal) = &self.removal {
                    let removal_strength = removal_power(removal.data[index]);
                    if 0.0 < removal_strength && 0.0 < sediment + deposit {
                        height -= removal_strength * (sediment + deposit);
                        sediment = sediment - removal_strength * sediment;
                        deposit = deposit - removal_strength * deposit;
                    }
                }
                if deposit <= 0.0 {
                    deposit = 0.0;
                }
                if sediment <= 0.0 {
                    sediment = 0.0;
                }
                self.height.data[index] = height;
                self.deposit.data[index] = deposit;
                self.sediment.data[index] = sediment;
                self.wear.data[index] = wear;
            }
        }
    }
}

/// `value^16` by repeated squaring, as sediment removal is weighted.
#[inline]
pub fn removal_power(value: f32) -> f32 {
    let mut power = value * value;
    power = power * power;
    power = power * power;
    power * power
}

fn downsampled(fine: &FloatMap, resolution: i32, kernel_sum: f32) -> FloatMap {
    let mut coarse = FloatMap::new(resolution, 0.0);
    for y in 0..resolution {
        for x in 0..resolution {
            let index = coarse.index(x, y);
            coarse.data[index] = downsample(fine, x, y, kernel_sum);
        }
    }
    coarse
}
