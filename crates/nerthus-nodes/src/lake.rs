//! The native simple lake (`Lake_Create`, `lake::lake_create`): water
//! poured in at a point runs down to low ground, and the basin it settles
//! in fills to a level without spilling over its rim.
//!
//! Every search is the native scanline flood fill over the ground at or
//! below a level, so the order cells are visited in (which decides ties)
//! is the native one.

use nerthus_engine::Map;

use crate::x86;

/// `lake::Parameters`.
#[derive(Clone, Copy, Debug)]
pub struct LakeSettings {
    /// The pixel the water is poured in at.
    pub x: i32,
    pub y: i32,
    /// How deep the basin fills (`WaterLevel`).
    pub level: f32,
    /// Raise the terrain to the water surface.
    pub render_surface: bool,
}

/// What `Lake_Create` fills in.
pub struct LakeMaps {
    /// The terrain, raised to the water surface when asked.
    pub heights: Map,
    /// How far below the surface the ground lies under the water.
    pub depth: Map,
    /// 1 under the water.
    pub water: Map,
    /// The water surface's height under the water.
    pub surface: Map,
}

/// A cell and its height.
#[derive(Clone, Copy, Debug)]
struct Cell {
    x: i32,
    y: i32,
    height: f32,
}

/// One run of flooded cells on a row (`{left, right, y}`, inclusive).
#[derive(Clone, Copy, Debug)]
struct Span {
    left: i32,
    right: i32,
    y: i32,
}

/// The native flood fill: spans are
/// queued first in, first out; each starts from a seed, takes the cells
/// to its left first (walking left), then the seed and the cells to its
/// right; then the rows above and below the span are searched for seeds
/// within its columns (four-connected).
struct ScanlineFill<'a, Floods, Visit>
where
    Floods: Fn(i32, i32) -> bool,
    Visit: FnMut(i32, i32),
{
    width: i32,
    height: i32,
    visited: Vec<bool>,
    spans: Vec<Span>,
    floods: &'a Floods,
    visit: &'a mut Visit,
}

impl<Floods, Visit> ScanlineFill<'_, Floods, Visit>
where
    Floods: Fn(i32, i32) -> bool,
    Visit: FnMut(i32, i32),
{
    fn is_visited(&self, x: i32, y: i32) -> bool {
        self.visited[(y * self.width + x) as usize]
    }

    fn take(&mut self, x: i32, y: i32) {
        self.visited[(y * self.width + x) as usize] = true;
        (self.visit)(x, y);
    }

    /// The span through an unvisited flooded seed; returns
    /// the column after it (after the seed when the seed does not flood).
    fn scan(&mut self, x: i32, y: i32) -> i32 {
        if self.is_visited(x, y) || !(self.floods)(x, y) {
            return x + 1;
        }
        let mut left = x;
        while left > 0 && !self.is_visited(left - 1, y) && (self.floods)(left - 1, y) {
            self.take(left - 1, y);
            left -= 1;
        }
        let mut right = x;
        while right < self.width && !self.is_visited(right, y) && (self.floods)(right, y) {
            self.take(right, y);
            right += 1;
        }
        let span = Span { left, right: right - 1, y };
        self.spans.push(span);
        right
    }

    /// Seeds new spans on `row` within a span's columns.
    fn scan_row(&mut self, span: Span, row: i32) {
        let mut column = span.left;
        while column <= span.right {
            if !self.is_visited(column, row) && (self.floods)(column, row) {
                column = self.scan(column, row);
            } else {
                column += 1;
            }
        }
    }

    fn run(&mut self, x: i32, y: i32) {
        self.scan(x, y);
        let mut next = 0;
        while next < self.spans.len() {
            let span = self.spans[next];
            next += 1;
            let above = span.y - 1;
            if above >= 0 {
                self.scan_row(span, above);
            }
            let below = span.y + 1;
            if below < self.height {
                self.scan_row(span, below);
            }
        }
    }
}

/// Floods from (x, y) over the cells `floods` accepts, handing each to
/// `visit` in the native order.
fn flood_fill(width: i32, height: i32, x: i32, y: i32, floods: &impl Fn(i32, i32) -> bool, visit: &mut impl FnMut(i32, i32)) {
    let cell_count = (width * height) as usize;
    let mut fill = ScanlineFill {
        width,
        height,
        visited: vec![false; cell_count],
        spans: Vec::new(),
        floods,
        visit,
    };
    fill.run(x, y);
}

fn height_at(heights: &Map, x: i32, y: i32) -> f32 {
    let width = heights.width() as i32;
    heights.data()[(y * width + x) as usize]
}

/// `lake::search_closest`: over the ground at or below `level` connected
/// to (x, y), the cell that best weighs how far it lies below `level`
/// against its distance in map widths: `(level − h) / (d² + softening)`,
/// d at least 1e-9. `None` when no cell scores above zero.
fn search_closest(heights: &Map, x: i32, y: i32, level: f32, softening: f32) -> Option<Cell> {
    let width = heights.width() as i32;
    let height = heights.height() as i32;
    let scale = 1.0 / width as f32;
    let floods = |cell_x: i32, cell_y: i32| height_at(heights, cell_x, cell_y) <= level;
    let mut best_score = 0.0f32;
    let mut best = None;
    let mut visit = |cell_x: i32, cell_y: i32| {
        let offset_x = cell_x.wrapping_sub(x);
        let offset_y = cell_y.wrapping_sub(y);
        let squared = offset_x.wrapping_mul(offset_x).wrapping_add(offset_y.wrapping_mul(offset_y));
        let distance = x86::maxss(1e-9, (squared as f32).sqrt()) * scale;
        let cell_height = height_at(heights, cell_x, cell_y);
        let score = (1.0 / (distance * distance + softening)) * (level - cell_height);
        if score > best_score {
            best_score = score;
            best = Some(Cell { x: cell_x, y: cell_y, height: cell_height });
        }
    };
    flood_fill(width, height, x, y, &floods, &mut visit);
    best
}

/// `lake::search_min`: the lowest cell of the ground at or below `level`
/// connected to (x, y), the first visited on ties; `None` when none lies
/// below `f32::MAX`.
fn search_min(heights: &Map, x: i32, y: i32, level: f32) -> Option<Cell> {
    let width = heights.width() as i32;
    let height = heights.height() as i32;
    let floods = |cell_x: i32, cell_y: i32| height_at(heights, cell_x, cell_y) <= level;
    let mut lowest: Option<Cell> = None;
    let mut visit = |cell_x: i32, cell_y: i32| {
        let cell_height = height_at(heights, cell_x, cell_y);
        let lowest_height = lowest.map_or(f32::MAX, |cell| cell.height);
        if cell_height < lowest_height {
            lowest = Some(Cell { x: cell_x, y: cell_y, height: cell_height });
        }
    };
    flood_fill(width, height, x, y, &floods, &mut visit);
    lowest
}

/// `lake::lake_create`.
pub fn create(heights: &Map, settings: &LakeSettings) -> LakeMaps {
    let resolution = heights.resolution();
    let mut maps = LakeMaps {
        heights: heights.clone(),
        depth: Map::new(resolution, 1),
        water: Map::new(resolution, 1),
        surface: Map::new(resolution, 1),
    };
    // NaN levels go on, as the native comparison lets them.
    if settings.level <= 0.0 {
        return maps;
    }

    // Where the water settles: from the pour point it hops to the best
    // low cell, at most five times, until it stays put. (A search with
    // softening 0.001 could also run each time, but its distance is
    // measured from the first search's cell, so it never wins; it is left
    // out.)
    // The found cell lives on into the basin searches, which only replace
    // it when they find a lower one.
    let mut x = settings.x;
    let mut y = settings.y;
    let mut level = height_at(heights, x, y);
    let mut found_x = 0;
    let mut found_y = 0;
    for _ in 0..5 {
        let closest = search_closest(heights, x, y, level, 0.01);
        let found_height = match closest {
            Some(cell) => {
                found_x = cell.x;
                found_y = cell.y;
                cell.height
            }
            None => f32::MAX,
        };
        if x == found_x && y == found_y {
            break;
        }
        level = 0.2 * settings.level + found_height;
        x = found_x;
        y = found_y;
    }

    // The basin: its lowest point, then how high it fills. When water at
    // the full level would reach lower ground, the level is halved towards
    // the highest one (nine steps) that keeps the basin's lowest point.
    let mut lowest_x = found_x;
    let mut lowest_y = found_y;
    let mut lowest_height = f32::MAX;
    if let Some(cell) = search_min(heights, x, y, level) {
        lowest_x = cell.x;
        lowest_y = cell.y;
        lowest_height = cell.height;
    }
    let basin_x = lowest_x;
    let basin_y = lowest_y;
    let base = lowest_height;
    let mut surface_level = base + settings.level;
    if let Some(cell) = search_min(heights, basin_x, basin_y, surface_level) {
        lowest_x = cell.x;
        lowest_y = cell.y;
    }
    let spills = lowest_x != basin_x || lowest_y != basin_y;
    if spills {
        let mut kept = 0.2 * settings.level * 0.9 + base;
        let mut spilled = base + settings.level;
        for _ in 0..9 {
            let middle = (spilled + kept) * 0.5;
            if let Some(cell) = search_min(heights, basin_x, basin_y, middle) {
                lowest_x = cell.x;
                lowest_y = cell.y;
            }
            let still_basin = lowest_x == basin_x && lowest_y == basin_y;
            if still_basin {
                kept = middle;
            } else {
                spilled = middle;
            }
        }
        surface_level = kept;
    }

    // The water.
    let width = resolution.x as i32;
    let height = resolution.y as i32;
    let floods = |cell_x: i32, cell_y: i32| height_at(heights, cell_x, cell_y) <= surface_level;
    let mut flooded = Vec::new();
    let mut visit = |cell_x: i32, cell_y: i32| flooded.push((cell_y * width + cell_x) as usize);
    flood_fill(width, height, basin_x, basin_y, &floods, &mut visit);
    let ground = heights.data();
    for index in flooded {
        maps.depth.data_mut()[index] = surface_level - ground[index];
        maps.water.data_mut()[index] = 1.0;
        maps.surface.data_mut()[index] = surface_level;
        if settings.render_surface {
            maps.heights.data_mut()[index] = surface_level;
        }
    }
    maps
}

#[cfg(test)]
mod tests {
    use nerthus_engine::Resolution;

    use super::*;

    fn row_map(heights: &[f32]) -> Map {
        let resolution = Resolution::new(heights.len(), 1);
        Map::from_data(resolution, heights.to_vec())
    }

    #[test]
    fn the_fill_takes_the_cells_left_of_the_seed_first() {
        let floods = |_: i32, _: i32| true;
        let mut order = Vec::new();
        let mut visit = |x: i32, _: i32| order.push(x);
        flood_fill(5, 1, 2, 0, &floods, &mut visit);
        assert_eq!(order, [1, 0, 2, 3, 4]);
    }

    #[test]
    fn the_lowest_point_is_the_first_visited_of_equal_ones() {
        let heights = row_map(&[0.0, 0.0, 0.0]);
        let lowest = search_min(&heights, 1, 0, 0.5).expect("the seed floods");
        assert_eq!((lowest.x, lowest.y), (0, 0));
    }

    #[test]
    fn no_level_leaves_no_water() {
        let heights = row_map(&[0.2, 0.1, 0.3]);
        let settings = LakeSettings {
            x: 1,
            y: 0,
            level: 0.0,
            render_surface: true,
        };
        let maps = create(&heights, &settings);
        assert!(maps.water.data().iter().all(|&value| value == 0.0));
        assert_eq!(maps.heights.data(), heights.data());
    }
}
