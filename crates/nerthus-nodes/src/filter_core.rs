//! The frequency filters (`FilterCore`), built from differences of
//! [`fast_blur`] blurs.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::fast_blur;
use crate::maths;
use crate::mathv;

/// Lanes of `Vector<float>` on x64 with AVX2. Sums accumulated lane by
/// lane must use the same width to round the same.
pub const VECTOR_LANES: usize = 8;

/// `FilterMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    LowPass,
    HighPass,
    LowShelf,
    HighShelf,
    BandPass,
    Notch,
    Bell,
}

impl FilterMode {
    pub fn from_name(name: &str) -> Self {
        match name {
            "HighPass" => Self::HighPass,
            "LowShelf" => Self::LowShelf,
            "HighShelf" => Self::HighShelf,
            "BandPass" => Self::BandPass,
            "Notch" => Self::Notch,
            "Bell" => Self::Bell,
            _ => Self::LowPass,
        }
    }
}

/// `FilterCore.MeasureDC`: the mean of a square plane, summed in
/// `VECTOR_LANES` lanes per row, then rows in order, then lanes in order.
pub fn measure_dc(data: &[f32], resolution: usize) -> f32 {
    let row_sums: Vec<[f32; VECTOR_LANES]> = data
        .par_chunks(resolution)
        .take(resolution)
        .map(|row| {
            let mut lanes = [0.0f32; VECTOR_LANES];
            for block in row.chunks(VECTOR_LANES) {
                for (lane, &value) in block.iter().enumerate() {
                    lanes[lane] += value;
                }
            }
            lanes
        })
        .collect();

    let mut totals = [0.0f32; VECTOR_LANES];
    for row in &row_sums {
        for (lane, &value) in row.iter().enumerate() {
            totals[lane] += value;
        }
    }
    let mut sum = 0.0f32;
    for &value in &totals {
        sum += value;
    }
    sum / (resolution * resolution) as f32
}

/// `FilterCore.FilterProcess`: `frequency` 0..1 picks the blur radius (a
/// quarter of the width down to almost nothing), `gain` scales shelves and
/// bells, `q` sets the band width, and `keep_dc` restores each channel's
/// mean afterwards.
pub fn filter_process(input: &Map, frequency: f32, gain: f32, q: f32, mode: FilterMode, keep_dc: bool) -> Map {
    let mut map = input.clone();
    let width = map.width();
    let mut dc_correction = vec![0.0f32; map.channel_count()];
    if keep_dc {
        for (channel, correction) in dc_correction.iter_mut().enumerate() {
            *correction = measure_dc(map.channel(channel), width);
        }
    }

    let radius = (1.0 - frequency * 0.9999) * width as f32 * 0.25;
    let is_band = matches!(mode, FilterMode::BandPass | FilterMode::Notch | FilterMode::Bell);
    let mut blurred = None;
    let mut low = None;
    let mut high = None;
    if is_band {
        let width_factor = 0.7 * q + 0.2;
        low = Some(fast_blur::blur_map(&map, radius * width_factor));
        high = Some(fast_blur::blur_map(&map, radius / width_factor));
    } else {
        blurred = Some(fast_blur::blur_map(&map, radius));
    }

    map = match mode {
        FilterMode::LowPass => blurred.expect("low pass blurs"),
        FilterMode::HighPass => {
            let blur = blurred.expect("high pass blurs");
            map.zip_values(&blur, |data, blur| data - blur)
        }
        FilterMode::LowShelf => {
            let blur = blurred.expect("low shelf blurs");
            map.zip_values(&blur, |data, blur| mathv::mix(data - blur, data, gain))
        }
        FilterMode::HighShelf => {
            let blur = blurred.expect("high shelf blurs");
            map.zip_values(&blur, |data, blur| mathv::mix(blur, data, gain))
        }
        FilterMode::BandPass => {
            let band_gain = maths::exp(q * q * 2.5);
            let low = low.expect("band pass blurs");
            let high = high.expect("band pass blurs");
            low.zip_values(&high, |low, high| (low - high) * band_gain)
        }
        FilterMode::Notch => {
            let band = low.expect("notch blurs").subtract(&high.expect("notch blurs"));
            map.zip_values(&band, |data, band| data - band)
        }
        FilterMode::Bell => {
            let band = low.expect("bell blurs").subtract(&high.expect("bell blurs"));
            map.zip_values(&band, |data, band| band * (gain - 1.0) + data)
        }
    };

    if keep_dc {
        for (channel, correction) in dc_correction.iter_mut().enumerate() {
            *correction -= measure_dc(map.channel(channel), width);
        }
        for (channel, &correction) in dc_correction.iter().enumerate() {
            for value in map.channel_mut(channel) {
                *value += correction;
            }
        }
    }
    map
}
