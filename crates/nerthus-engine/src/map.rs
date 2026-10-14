//! `Map`: planar float channels at a (not necessarily square)
//! resolution, tagged with how the data should be presented.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// `Resolution` (`X` columns by `Y` rows).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Resolution {
    #[serde(rename = "X")]
    pub x: usize,
    #[serde(rename = "Y")]
    pub y: usize,
}

impl Resolution {
    pub const fn new(x: usize, y: usize) -> Self {
        Self { x, y }
    }

    pub const fn square(size: usize) -> Self {
        Self { x: size, y: size }
    }

    pub const fn area(self) -> usize {
        self.x * self.y
    }
}

/// How a map's values are meant to be read (`RenderIntent`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderIntent {
    /// Inherit the intent of the port that produced the map.
    #[default]
    Default,
    Heightfield,
    Mask,
    Color,
    Raw,
}

impl RenderIntent {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Default" => Some(Self::Default),
            "Heightfield" => Some(Self::Heightfield),
            "Mask" => Some(Self::Mask),
            "Color" => Some(Self::Color),
            "Raw" => Some(Self::Raw),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Heightfield => "Heightfield",
            Self::Mask => "Mask",
            Self::Color => "Color",
            Self::Raw => "Raw",
        }
    }
}

/// `MapInfo`: metadata travelling with a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapInfo {
    pub render_intent: RenderIntent,
}

impl Default for MapInfo {
    fn default() -> Self {
        Self {
            render_intent: RenderIntent::Heightfield,
        }
    }
}

/// A map: one `f32` plane per channel, row-major (`y * x_size + x`).
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    resolution: Resolution,
    channels: Vec<Vec<f32>>,
    pub info: MapInfo,
}

impl Map {
    /// The bytes its channels hold.
    pub fn byte_size(&self) -> usize {
        self.channels.iter().map(|channel| channel.len() * std::mem::size_of::<f32>()).sum()
    }

    /// A zero-filled map (`new Map(resolution, channels)`).
    pub fn new(resolution: Resolution, channel_count: usize) -> Self {
        let channel_count = channel_count.max(1);
        let channels = (0..channel_count).map(|_| vec![0.0; resolution.area()]).collect();
        Self {
            resolution,
            channels,
            info: MapInfo::default(),
        }
    }

    /// A single-channel map over `data`.
    pub fn from_data(resolution: Resolution, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), resolution.area(), "map data does not match the resolution");
        Self {
            resolution,
            channels: vec![data],
            info: MapInfo::default(),
        }
    }

    /// A map over the given planes (`new Map(resolution, params float[][])`).
    pub fn from_channels(resolution: Resolution, channels: Vec<Vec<f32>>) -> Self {
        assert!(!channels.is_empty(), "a map needs at least one channel");
        for channel in &channels {
            assert_eq!(channel.len(), resolution.area(), "map channel does not match the resolution");
        }
        Self {
            resolution,
            channels,
            info: MapInfo::default(),
        }
    }

    /// Channels of several maps side by side (`new Map(params Map[])`); the
    /// intent comes from the first map.
    pub fn stack(maps: &[&Map]) -> Self {
        let resolution = maps[0].resolution;
        let mut channels = Vec::new();
        for map in maps {
            assert_eq!(map.resolution, resolution, "stacked maps must share a resolution");
            for channel in &map.channels {
                channels.push(channel.clone());
            }
        }
        Self {
            resolution,
            channels,
            info: maps[0].info,
        }
    }

    pub fn resolution(&self) -> Resolution {
        self.resolution
    }

    pub fn width(&self) -> usize {
        self.resolution.x
    }

    pub fn height(&self) -> usize {
        self.resolution.y
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// The first channel (`Map.Data`).
    pub fn data(&self) -> &[f32] {
        &self.channels[0]
    }

    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.channels[0]
    }

    pub fn channel(&self, index: usize) -> &[f32] {
        &self.channels[index]
    }

    pub fn channel_mut(&mut self, index: usize) -> &mut [f32] {
        &mut self.channels[index]
    }

    /// Every channel, mutably at once.
    pub fn channels_mut(&mut self) -> Vec<&mut [f32]> {
        self.channels.iter_mut().map(|channel| channel.as_mut_slice()).collect()
    }

    /// The channel the red/green/blue accessors return: index clamped to
    /// the last channel, so a grey map reads the same plane three times.
    pub fn rgb_channel(&self, index: usize) -> &[f32] {
        let clamped = index.min(self.channels.len() - 1);
        &self.channels[clamped]
    }

    pub fn into_channels(self) -> Vec<Vec<f32>> {
        self.channels
    }

    pub fn into_data(mut self) -> Vec<f32> {
        self.channels.swap_remove(0)
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.channels[0][y * self.resolution.x + x]
    }

    pub fn set(&mut self, x: usize, y: usize, value: f32) {
        let width = self.resolution.x;
        self.channels[0][y * width + x] = value;
    }

    /// Interleaved samples (`ToInterleavedArray`).
    pub fn to_interleaved(&self) -> Vec<f32> {
        let channel_count = self.channels.len();
        let mut interleaved = vec![0.0; self.resolution.area() * channel_count];
        for (channel_index, channel) in self.channels.iter().enumerate() {
            for (pixel, &value) in channel.iter().enumerate() {
                interleaved[pixel * channel_count + channel_index] = value;
            }
        }
        interleaved
    }

    /// A map from interleaved samples (`FromInterleavedArray`); samples
    /// past the map's area are left out.
    pub fn from_interleaved(resolution: Resolution, interleaved: &[f32], channel_count: usize) -> Self {
        let mut channels = vec![vec![0.0; resolution.area()]; channel_count];
        let pixels = interleaved.chunks_exact(channel_count).take(resolution.area());
        for (pixel, samples) in pixels.enumerate() {
            for (channel_index, &value) in samples.iter().enumerate() {
                channels[channel_index][pixel] = value;
            }
        }
        Self::from_channels(resolution, channels)
    }
}

/// Element-wise arithmetic: the result has as many channels
/// as the larger operand, a single-channel operand applies to every channel
/// (`Map.Generate(mapA, mapB, …)`), and the result carries default info.
impl Map {
    /// `Map.Generate(map, function)`: a new map with `function` applied to
    /// every value.
    pub fn map_values(&self, function: impl Fn(f32) -> f32 + Sync) -> Map {
        let channels = self
            .channels
            .iter()
            .map(|channel| channel.par_iter().map(|&value| function(value)).collect())
            .collect();
        Map {
            resolution: self.resolution,
            channels,
            info: MapInfo::default(),
        }
    }

    /// `Map.Generate(mapA, mapB, function)`.
    pub fn zip_values(&self, other: &Map, function: impl Fn(f32, f32) -> f32 + Sync) -> Map {
        assert_eq!(self.resolution, other.resolution, "maps must share a resolution");
        let channel_count = self.channel_count().max(other.channel_count());
        let channels = (0..channel_count)
            .map(|channel| {
                let left = self.rgb_channel(channel);
                let right = other.rgb_channel(channel);
                left.par_iter().zip(right.par_iter()).map(|(&a, &b)| function(a, b)).collect()
            })
            .collect();
        Map {
            resolution: self.resolution,
            channels,
            info: MapInfo::default(),
        }
    }

    /// In-place version of [`Map::map_values`] (`Map.Process(function)`).
    pub fn apply(&mut self, function: impl Fn(f32) -> f32 + Sync) {
        for channel in &mut self.channels {
            channel.par_iter_mut().for_each(|value| *value = function(*value));
        }
    }

    pub fn add(&self, other: &Map) -> Map {
        self.zip_values(other, |a, b| a + b)
    }

    pub fn subtract(&self, other: &Map) -> Map {
        self.zip_values(other, |a, b| a - b)
    }

    pub fn multiply(&self, other: &Map) -> Map {
        self.zip_values(other, |a, b| a * b)
    }

    /// `Map.Divide(mapA, mapB)`: zero divisors become `1e-35`.
    pub fn divide(&self, other: &Map) -> Map {
        self.zip_values(other, |a, b| a / non_zero(b))
    }

    /// `Map.Divide(map, scalar)`: a multiplication by the reciprocal.
    pub fn divide_scalar(&self, scalar: f32) -> Map {
        let reciprocal = 1.0 / non_zero(scalar);
        self.multiply_scalar(reciprocal)
    }

    /// `scalar / map`, with zero divisors made `1e-35`.
    pub fn divide_from(&self, scalar: f32) -> Map {
        self.map_values(|value| scalar / non_zero(value))
    }

    pub fn add_scalar(&self, scalar: f32) -> Map {
        self.map_values(|value| value + scalar)
    }

    pub fn multiply_scalar(&self, scalar: f32) -> Map {
        self.map_values(|value| value * scalar)
    }

    /// `scalar - map`.
    pub fn subtract_from(&self, scalar: f32) -> Map {
        self.map_values(|value| scalar - value)
    }

    /// A single-channel map with every value set to `value` (a blank map).
    pub fn filled(resolution: Resolution, value: f32) -> Map {
        let data = vec![value; resolution.area()];
        Map::from_data(resolution, data)
    }

    /// `Map.Clear(value)` on every channel.
    pub fn fill(&mut self, value: f32) {
        for channel in &mut self.channels {
            channel.fill(value);
        }
    }

    /// `Map.Generate(resolution, channels, context => …)`: `function` gets
    /// the column and row of each value as floats.
    pub fn generate(resolution: Resolution, function: impl Fn(f32, f32) -> f32 + Sync) -> Map {
        let width = resolution.x;
        let mut data = vec![0.0; resolution.area()];
        data.par_chunks_mut(width.max(1)).enumerate().for_each(|(row, values)| {
            let y = row as f32;
            for (column, value) in values.iter_mut().enumerate() {
                *value = function(column as f32, y);
            }
        });
        Map::from_data(resolution, data)
    }
}

/// The guard against dividing by zero.
fn non_zero(divisor: f32) -> f32 {
    if divisor == 0.0 { 1e-35 } else { divisor }
}
