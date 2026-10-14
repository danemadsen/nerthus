//! `ComputeNormalsWithSmoothingsGroups` (`SmoothingGroups.inl`) over
//! `SGSpatialSort`, as the 3DS and ASE importers use it on their unique
//! (three vertices per face) meshes.

use std::cmp::Ordering;

use crate::msvc;
use crate::vector::{Vector3, cross, dot, normalize, normalize_safe, square_length, sub};

/// `SGSpatialSort` entries.
#[derive(Clone, Copy)]
struct SortEntry {
    index: u32,
    position: Vector3,
    smooth_groups: u32,
    distance: f32,
}

/// `SGSpatialSort`: positions with their smoothing groups, sorted by their
/// distance along a fixed plane normal.
pub(crate) struct SgSpatialSort {
    entries: Vec<SortEntry>,
    plane_normal: Vector3,
}

impl SgSpatialSort {
    pub(crate) fn new() -> Self {
        SgSpatialSort { entries: Vec::new(), plane_normal: normalize([0.8523, 0.34321, 0.5736]) }
    }

    /// `Add`.
    pub(crate) fn add(&mut self, position: Vector3, index: u32, smooth_groups: u32) {
        let distance = dot(position, self.plane_normal);
        self.entries.push(SortEntry { index, position, smooth_groups, distance });
    }

    /// `Prepare`: sorted by distance (MSVC's `std::sort`).
    pub(crate) fn prepare(&mut self) {
        msvc::sort(&mut self.entries, |a, b| a.distance < b.distance);
    }

    /// `FindPositions`: the entries within `radius`; with `exact`, of the
    /// same smoothing groups, else of a shared one (any, for group 0, and
    /// entries without one).
    pub(crate) fn find_positions(&self, position: Vector3, group: u32, radius: f32, exact: bool, results: &mut Vec<u32>) {
        let entries = &self.entries;
        results.clear();
        let distance = dot(position, self.plane_normal);
        let min_distance = distance - radius;
        let max_distance = distance + radius;
        let (Some(first), Some(last)) = (entries.first(), entries.last()) else {
            return;
        };
        if max_distance < first.distance || min_distance > last.distance {
            return;
        }
        let mut index = entries.len() / 2;
        let mut step = entries.len() / 4;
        while step > 1 {
            if entries[index].distance < min_distance {
                index += step;
            } else {
                index -= step;
            }
            step /= 2;
        }
        while index > 0 && entries[index].distance > min_distance {
            index -= 1;
        }
        while index < entries.len() - 1 && entries[index].distance < min_distance {
            index += 1;
        }

        let square_epsilon = radius * radius;
        for entry in &entries[index..] {
            let below = entry.distance.partial_cmp(&max_distance) == Some(Ordering::Less);
            if !below {
                break;
            }
            let near = square_length(sub(entry.position, position)) < square_epsilon;
            let shared = if exact {
                entry.smooth_groups == group
            } else {
                group == 0 || entry.smooth_groups & group != 0 || entry.smooth_groups == 0
            };
            if near && shared {
                results.push(entry.index);
            }
        }
    }
}

/// `ComputeNormalsWithSmoothingsGroups`: each face's (unnormalised) normal
/// on its vertices, then every vertex's normal the normalised sum over the
/// vertices near it sharing its face's smoothing group. `faces` are each
/// face's indices and smoothing groups.
pub(crate) fn compute_normals(positions: &[Vector3], faces: &[([u32; 3], u32)]) -> Vec<Vector3> {
    let mut face_normals = vec![[0.0f32; 3]; positions.len()];
    for (indices, _) in faces {
        let [first, second, third] = indices.map(|index| positions[index as usize]);
        let normal = cross(sub(second, first), sub(third, first));
        for &index in indices {
            face_normals[index as usize] = normal;
        }
    }

    let mut min = [1e10f32; 3];
    let mut max = [-1e10f32; 3];
    for position in positions {
        for axis in 0..3 {
            if position[axis] < min[axis] {
                min[axis] = position[axis];
            }
            if max[axis] < position[axis] {
                max[axis] = position[axis];
            }
        }
    }
    let epsilon = square_length(sub(max, min)).sqrt() * 1e-5;

    let mut sort = SgSpatialSort::new();
    for &(indices, group) in faces {
        for index in indices {
            sort.add(positions[index as usize], index, group);
        }
    }
    sort.prepare();

    let mut normals = vec![[0.0f32; 3]; positions.len()];
    let mut done = vec![false; positions.len()];
    let mut results = Vec::new();
    for &(indices, group) in faces {
        for index in indices {
            if done[index as usize] {
                continue;
            }
            sort.find_positions(positions[index as usize], group, epsilon, false, &mut results);
            let mut sum = [0.0f32; 3];
            for &found in &results {
                let normal = face_normals[found as usize];
                sum = [sum[0] + normal[0], sum[1] + normal[1], sum[2] + normal[2]];
            }
            let normal = normalize_safe(sum);
            for &found in &results {
                normals[found as usize] = normal;
                done[found as usize] = true;
            }
        }
    }
    normals
}
