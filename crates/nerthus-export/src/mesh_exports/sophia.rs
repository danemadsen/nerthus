//! `Sophia`: the adaptive ("Sophia")
//! triangulation behind Mesher's Adaptive topology. The heightmap is
//! padded to (n + 1)² and split as a binary triangle tree: a triangle is
//! split at its hypotenuse while the precomputed error there exceeds
//! `max_error`, which is searched for so the mesh lands near the target
//! polygon count.
//!
//! Ported with its state handling: the vertex and triangle counters
//! survive from one `process_mesh` to the next (only some passes reset
//! them), so later levels of detail start their error search from inflated
//! counts and their first mesh carries unused slots.

use nerthus_nodes::mesh_rasterizer::Mesh;

/// .NET's `MathF.Max`: NaN wins, and +0 beats −0.
fn dotnet_max(left: f32, right: f32) -> f32 {
    if left != right {
        if !left.is_nan() {
            return if right < left { left } else { right };
        }
        return left;
    }
    if right.is_sign_negative() { left } else { right }
}

pub struct Sophia {
    /// The triangle tree's side, n.
    size: i32,
    /// The padded grid's side, n + 1.
    stride: i32,
    terrain: Vec<f32>,
    /// The error at each split point.
    errors: Vec<f32>,
    /// Each grid point's vertex number, from 1 (0 = unused).
    vertex_ids: Vec<i32>,
    /// (x, y) per vertex.
    vertices: Vec<i32>,
    /// Three vertex indices per triangle.
    triangles: Vec<i32>,
    /// The next triangle slot to fill.
    triangle_cursor: i32,
    vertex_count: i32,
    triangle_count: i32,
    max_error: f32,
    safety_threshold: f32,
    optimization_threshold: i32,
    accuracy: f32,
    finder_accuracy: f32,
    pub target_polycount: i32,
}

impl Sophia {
    /// `new Sophia(ref terrain, polygonCount)`.
    pub fn new(terrain: &[f32], polygon_count: i32) -> Self {
        let padded = pad(terrain);
        let side = (padded.len() as f64).sqrt() as i32;
        let mut sophia = Self {
            size: side - 1,
            stride: side,
            terrain: padded,
            errors: Vec::new(),
            vertex_ids: Vec::new(),
            vertices: Vec::new(),
            triangles: Vec::new(),
            triangle_cursor: 0,
            vertex_count: 0,
            triangle_count: 0,
            max_error: 1.0,
            safety_threshold: 1e-8,
            optimization_threshold: 1000,
            accuracy: 0.9,
            finder_accuracy: 0.9,
            target_polycount: polygon_count,
        };
        if sophia.target_polycount <= 50000 {
            sophia.max_error = 1.0;
        }
        sophia
    }

    fn at(&self, x: i32, y: i32) -> usize {
        (y * self.stride + x) as usize
    }

    /// The share of grid points sitting (to 1e-7) on their eight
    /// neighbours' mean — written as |Σ neighbours| / 8 − centre.
    fn flatness(&self) -> f32 {
        let side = self.stride;
        let mut total = 0i32;
        for row in 0..side {
            let above = (row - 1).max(0);
            let below = (row + 1).min(side - 1);
            for column in 0..side {
                let left = (column - 1).max(0);
                let right = (column + 1).min(side - 1);
                let terrain = &self.terrain;
                let sum = terrain[self.at(left, above)]
                    + terrain[self.at(column, above)]
                    + terrain[self.at(right, above)]
                    + terrain[self.at(left, row)]
                    + terrain[self.at(right, row)]
                    + terrain[self.at(left, below)]
                    + terrain[self.at(column, below)]
                    + terrain[self.at(right, below)];
                if sum.abs() * 0.125 - terrain[self.at(column, row)] < 1e-7 {
                    total += 1;
                }
            }
        }
        total as f32 / (side * side) as f32
    }

    /// `ProcessMesh`: searches the error bound, then tightens it until the
    /// mesh reaches the target; when that takes over a thousand passes the
    /// target shrinks by the flat share and it starts again.
    pub fn process_mesh(&mut self) {
        let mut target = self.target_polycount.min((self.size - 1).wrapping_mul(self.size - 1));
        loop {
            self.find_error(target);
            self.reset();
            let mut passes = 0;
            loop {
                passes += 1;
                self.build();
                if self.vertices.len() as i32 >= target || passes > self.optimization_threshold {
                    break;
                }
                self.max_error *= self.accuracy;
                self.reset();
            }
            if passes > self.optimization_threshold - 1 {
                target = (target as f32 * (1.0 - self.flatness())) as i32;
                continue;
            }
            break;
        }
    }

    fn reset(&mut self) {
        self.triangles = Vec::new();
        self.vertices = Vec::new();
        self.triangle_cursor = 0;
        self.vertex_count = 0;
        self.triangle_count = 0;
    }

    /// Lowers the error bound by `finder_accuracy` until the mesh
    /// has at least half the target's slots.
    fn find_error(&mut self, target: i32) {
        let half = target / 2;
        let mut last = 1.0f32;
        while self.max_error > self.safety_threshold {
            if self.max_error == 0.0 {
                self.max_error = last;
                break;
            }
            self.build();
            if (self.vertices.len() as i32) < half {
                last = self.max_error;
                self.max_error *= self.finder_accuracy;
                self.reset();
                continue;
            }
            break;
        }
    }

    /// `CalculateErrors`: for every split point, from the finest level up,
    /// how far the terrain is from its hypotenuse's midpoint — the largest
    /// of its own and its two children's below the finest level.
    pub fn calculate_errors(&mut self) {
        let side = self.stride as usize;
        self.errors = vec![0.0; side * side];
        let count = self.size.wrapping_mul(self.size).wrapping_mul(2) - 2;
        let inner = count - self.size.wrapping_mul(self.size);
        self.vertex_ids = vec![0; side * side];
        let mut index = count - 1;
        while index >= 0 {
            let [x0, y0, x1, y1] = self.hypotenuse(index);
            let middle_x = (x0 + x1) >> 1;
            let middle_y = (y0 + y1) >> 1;
            let apex_x = middle_x + middle_y - y0;
            let apex_y = middle_y + x0 - middle_x;
            let mean = (self.terrain[self.at(x0, y0)] + self.terrain[self.at(x1, y1)]) / 2.0;
            let middle = self.at(middle_x, middle_y);
            let deviation = (mean - self.terrain[middle]).abs();
            self.errors[middle] = dotnet_max(self.errors[middle], deviation);
            if index < inner {
                let first = self.at((x0 + apex_x) >> 1, (y0 + apex_y) >> 1);
                let second = self.at((x1 + apex_x) >> 1, (y1 + apex_y) >> 1);
                let own = self.errors[middle];
                let first_error = self.errors[first];
                let second_error = self.errors[second];
                self.errors[middle] = dotnet_max(dotnet_max(own, second_error), first_error);
            }
            index -= 1;
        }
    }

    /// The hypotenuse ends of triangle `index` in the tree (two
    /// roots, then each triangle's children), from its path bits.
    fn hypotenuse(&self, index: i32) -> [i32; 4] {
        let size = self.size;
        let mut path = index + 2;
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, 0, 0);
        let mut apex_x = 0;
        let mut apex_y = 0;
        if path & 1 == 1 {
            x1 = size;
            y1 = size;
            apex_x = size;
        } else {
            x0 = size;
            y0 = size;
            apex_y = size;
        }
        loop {
            path >>= 1;
            if path <= 1 {
                break;
            }
            let middle_x = (x0 + x1) >> 1;
            let middle_y = (y0 + y1) >> 1;
            if path & 1 == 1 {
                x1 = x0;
                y1 = y0;
                x0 = apex_x;
                y0 = apex_y;
            } else {
                x0 = x1;
                y0 = y1;
                x1 = apex_x;
                y1 = apex_y;
            }
            apex_x = middle_x;
            apex_y = middle_y;
        }
        [x0, y0, x1, y1]
    }

    /// Counts the mesh at the current error bound, then fills
    /// it. The counters are not reset here.
    fn build(&mut self) {
        self.vertex_ids.iter_mut().for_each(|id| *id = 0);
        let size = self.size;
        self.count(0, 0, size, size, size, 0);
        self.count(size, size, 0, 0, 0, size);
        self.vertices = vec![0; (self.vertex_count.wrapping_mul(2)).max(0) as usize];
        self.triangles = vec![0; (self.triangle_count.wrapping_mul(3)).max(0) as usize];
        self.emit(0, 0, size, size, size, 0);
        self.emit(size, size, 0, 0, 0, size);
    }

    fn splits(&self, x0: i32, y0: i32, x1: i32, y1: i32, apex_x: i32, apex_y: i32) -> bool {
        let middle_x = (x0 + x1) >> 1;
        let middle_y = (y0 + y1) >> 1;
        (x0 - apex_x).abs() + (y0 - apex_y).abs() > 1 && self.errors[self.at(middle_x, middle_y)] > self.max_error
    }

    /// Numbers the vertices of the triangles this one ends up as.
    fn count(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, apex_x: i32, apex_y: i32) {
        if self.splits(x0, y0, x1, y1, apex_x, apex_y) {
            let middle_x = (x0 + x1) >> 1;
            let middle_y = (y0 + y1) >> 1;
            self.count(apex_x, apex_y, x0, y0, middle_x, middle_y);
            self.count(x1, y1, apex_x, apex_y, middle_x, middle_y);
            return;
        }
        for (x, y) in [(x0, y0), (x1, y1), (apex_x, apex_y)] {
            let point = self.at(x, y);
            if self.vertex_ids[point] == 0 {
                self.vertex_count += 1;
                self.vertex_ids[point] = self.vertex_count;
            }
        }
        self.triangle_count += 1;
    }

    /// Writes the vertices and triangles [`count`] numbered.
    fn emit(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, apex_x: i32, apex_y: i32) {
        if self.splits(x0, y0, x1, y1, apex_x, apex_y) {
            let middle_x = (x0 + x1) >> 1;
            let middle_y = (y0 + y1) >> 1;
            self.emit(apex_x, apex_y, x0, y0, middle_x, middle_y);
            self.emit(x1, y1, apex_x, apex_y, middle_x, middle_y);
            return;
        }
        let first = self.vertex_ids[self.at(x0, y0)] - 1;
        let second = self.vertex_ids[self.at(x1, y1)] - 1;
        let third = self.vertex_ids[self.at(apex_x, apex_y)] - 1;
        for (vertex, x, y) in [(first, x0, y0), (second, x1, y1), (third, apex_x, apex_y)] {
            self.vertices[2 * vertex as usize] = x;
            self.vertices[2 * vertex as usize + 1] = y;
        }
        for vertex in [first, second, third] {
            self.triangles[self.triangle_cursor as usize] = vertex;
            self.triangle_cursor += 1;
        }
    }

    /// `SaveMesh`'s mesh (`MeshPort_TrianglesToMesh`): vertices at
    /// (x, y) / (n + 1) with heights × `ratio`, and the grid it came from.
    pub fn mesh(&self, ratio: f32) -> (Mesh, (i32, i32)) {
        let stride = self.stride as f32;
        let (points, _) = self.vertices.as_chunks::<2>();
        let vertices = points
            .iter()
            .map(|&[column, row]| {
                let x = column as f32 / stride;
                let y = row as f32 / stride;
                let height = self.terrain[self.at(column, row)] * ratio;
                [x, y, height]
            })
            .collect();
        let (corners, _) = self.triangles.as_chunks::<3>();
        let triangles = corners.iter().map(|corner| corner.map(|index| index as u32)).collect();
        let mesh = Mesh { vertices, triangles, normals: Vec::new() };
        (mesh, (self.stride, self.stride))
    }
}

/// An n × n terrain padded to (n + 1)², the last row then the
/// last column repeating their neighbours.
fn pad(terrain: &[f32]) -> Vec<f32> {
    let side = (terrain.len() as f64).sqrt() as usize;
    let padded_side = side + 1;
    let mut padded = vec![0.0f32; padded_side * padded_side];
    if padded_side * padded_side == terrain.len() {
        padded.copy_from_slice(terrain);
        return padded;
    }
    let mut next = 0;
    for row in 0..side {
        for column in 0..side {
            padded[row * padded_side + column] = terrain[next];
            next += 1;
        }
    }
    if padded_side >= 2 {
        for column in 0..padded_side {
            padded[padded_side * (padded_side - 1) + column] = padded[padded_side * (padded_side - 2) + column];
        }
        for row in 0..padded_side {
            padded[padded_side * row + padded_side - 1] = padded[padded_side * row + padded_side - 2];
        }
    }
    padded
}
