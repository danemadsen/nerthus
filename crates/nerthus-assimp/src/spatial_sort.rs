//! `SpatialSort`: positions sorted by their distance along a skewed plane
//! normal from their centroid, for finding positions near each other.
//!
//! The sort is `std::sort`, which isn't stable: positions at exactly the
//! same distance (which happens in big meshes) stay in the order MSVC's
//! sort leaves them, and that order decides groups.

use crate::msvc;
use crate::vector::{Vector3, normalize, square_length, sub};

#[derive(Clone, Copy)]
struct Entry {
    index: u32,
    position: Vector3,
    distance: f32,
}

pub(crate) struct SpatialSort {
    normal: Vector3,
    centroid: Vector3,
    entries: Vec<Entry>,
}

impl SpatialSort {
    /// The default constructor: the plane normal (0.8523, 0.34321,
    /// 0.5736), normalised.
    pub fn new() -> Self {
        SpatialSort { normal: normalize([0.8523, 0.34321, 0.5736]), centroid: [0.0; 3], entries: Vec::new() }
    }

    /// `Append` without finalising: indices continue from the last ones.
    pub fn append(&mut self, positions: &[Vector3]) {
        let initial = self.entries.len();
        for (offset, &position) in positions.iter().enumerate() {
            self.entries.push(Entry { index: (initial + offset) as u32, position, distance: 0.0 });
        }
    }

    fn distance(&self, position: Vector3) -> f32 {
        let relative = sub(position, self.centroid);
        relative[0] * self.normal[0] + relative[1] * self.normal[1] + relative[2] * self.normal[2]
    }

    /// `Finalize`: the centroid (each position scaled by 1/count and
    /// summed), the distances, then the sort.
    pub fn finalize(&mut self) {
        let scale = 1.0f32 / self.entries.len() as f32;
        for index in 0..self.entries.len() {
            let position = self.entries[index].position;
            let scaled = [scale * position[0], scale * position[1], scale * position[2]];
            self.centroid = [self.centroid[0] + scaled[0], self.centroid[1] + scaled[1], self.centroid[2] + scaled[2]];
        }
        for index in 0..self.entries.len() {
            let distance = self.distance(self.entries[index].position);
            self.entries[index].distance = distance;
        }
        msvc::sort(&mut self.entries, |a, b| a.distance < b.distance);
    }

    /// `GenerateMappingTable`: a group number per position, positions
    /// within `radius` of a group's first (in sorted order) joining it.
    pub fn generate_mapping_table(&self, radius: f32) -> (Vec<u32>, u32) {
        let mut table = vec![u32::MAX; self.entries.len()];
        let squared = radius * radius;
        let mut group = 0u32;
        let mut index = 0;
        while index < self.entries.len() {
            let distance = self.distance(self.entries[index].position);
            let max_distance = distance + radius;
            table[self.entries[index].index as usize] = group;
            let first = self.entries[index].position;
            index += 1;
            while index < table.len() && self.entries[index].distance < max_distance && square_length(sub(self.entries[index].position, first)) < squared {
                table[self.entries[index].index as usize] = group;
                index += 1;
            }
            group += 1;
        }
        (table, group)
    }
}
