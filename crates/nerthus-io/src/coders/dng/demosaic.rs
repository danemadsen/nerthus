//! LibRaw's other demosaicing for DNG files: `lin_interpolate` and
//! `vng_interpolate` (`misc_demosaic.cpp`) for four-colour and other
//! non-Bayer patterns, and `xtrans_interpolate` (`xtrans_demosaic.cpp`,
//! Frank Markesteijn's algorithm, three passes at quality 3) for X-Trans.
//! VNG writes each row two rows late, so it only reads rows it has not
//! changed yet. X-Trans tiles read pixels earlier tiles have already
//! written (the overlap's greens bound later ones), so the tiles run in
//! LibRaw's order, top to bottom and left to right.

use anyhow::{bail, Result};

use super::identify::fcol;
use super::process::{clip, Cielab, Image, TILE};

/// `vng_interpolate`'s gradient terms: y1, x1, y2, x2, weight, gradients.
const VNG_TERMS: [i8; 384] = [
    -2, -2, 0, -1, 0, 0x01, -2, -2, 0, 0, 1, 0x01, -2, -1, -1, 0, 0, 0x01, -2, -1, 0, -1, 0, 0x02, -2, -1, 0, 0, 0, 0x03, -2, -1, 0, 1, 1, 0x01, -2, 0, 0, -1, 0, 0x06, -2, 0, 0, 0, 1, 0x02, -2, 0, 0, 1, 0, 0x03, -2, 1, -1, 0, 0, 0x04, -2, 1, 0, -1, 1, 0x04, -2, 1,
    0, 0, 0, 0x06, -2, 1, 0, 1, 0, 0x02, -2, 2, 0, 0, 1, 0x04, -2, 2, 0, 1, 0, 0x04, -1, -2, -1, 0, 0, -128, -1, -2, 0, -1, 0, 0x01, -1, -2, 1, -1, 0, 0x01, -1, -2, 1, 0, 1, 0x01, -1, -1, -1, 1, 0, -120, -1, -1, 1, -2, 0, 0x40, -1, -1, 1, -1, 0, 0x22,
    -1, -1, 1, 0, 0, 0x33, -1, -1, 1, 1, 1, 0x11, -1, 0, -1, 2, 0, 0x08, -1, 0, 0, -1, 0, 0x44, -1, 0, 0, 1, 0, 0x11, -1, 0, 1, -2, 1, 0x40, -1, 0, 1, -1, 0, 0x66, -1, 0, 1, 0, 1, 0x22, -1, 0, 1, 1, 0, 0x33, -1, 0, 1, 2, 1, 0x10, -1, 1, 1, -1, 1, 0x44, -1, 1,
    1, 0, 0, 0x66, -1, 1, 1, 1, 0, 0x22, -1, 1, 1, 2, 0, 0x10, -1, 2, 0, 1, 0, 0x04, -1, 2, 1, 0, 1, 0x04, -1, 2, 1, 1, 0, 0x04, 0, -2, 0, 0, 1, -128, 0, -1, 0, 1, 1, -120, 0, -1, 1, -2, 0, 0x40, 0, -1, 1, 0, 0, 0x11, 0, -1, 2, -2, 0, 0x40, 0, -1, 2, -1, 0, 0x20,
    0, -1, 2, 0, 0, 0x30, 0, -1, 2, 1, 1, 0x10, 0, 0, 0, 2, 1, 0x08, 0, 0, 2, -2, 1, 0x40, 0, 0, 2, -1, 0, 0x60, 0, 0, 2, 0, 1, 0x20, 0, 0, 2, 1, 0, 0x30, 0, 0, 2, 2, 1, 0x10, 0, 1, 1, 0, 0, 0x44, 0, 1, 1, 2, 0, 0x10, 0, 1, 2, -1, 1, 0x40, 0, 1, 2, 0, 0, 0x60, 0,
    1, 2, 1, 0, 0x20, 0, 1, 2, 2, 0, 0x10, 1, -2, 1, 0, 0, -128, 1, -1, 1, 1, 0, -120, 1, 0, 1, 2, 0, 0x08, 1, 0, 2, -1, 0, 0x40, 1, 0, 2, 1, 0, 0x10,
];
/// `chood`: the eight neighbours, clockwise from north-west.
const VNG_NEIGHBOURS: [i8; 16] = [-1, -1, -1, 0, -1, 1, 0, 1, 1, 1, 1, 0, 1, -1, 0, -1];

impl Image<'_> {
    fn pattern_colour(&self, row: i32, column: i32) -> usize {
        fcol(self.state, row, column)
    }

    /// A sample by flat offset (pixel × 4 + channel) from a pixel, as
    /// LibRaw's `ushort *` arithmetic reads it.
    fn sample(&self, pixel: usize, offset: i32) -> i32 {
        let flat = pixel as i64 * 4 + offset as i64;
        self.pixels[(flat / 4) as usize][(flat % 4) as usize] as i32
    }

    /// `lin_interpolate`: each missing colour, a weighted mean of the 3×3
    /// neighbours of that colour (orthogonal ones count double).
    pub(super) fn lin_interpolate(&mut self) {
        let width = self.width as i32;
        let height = self.height as i32;
        let colors = self.state.colors.min(4);
        let size: i32 = if self.state.filters == 9 { 6 } else { 16 };
        self.border_interpolate(1);
        let mut code = vec![0i32; 16 * 16 * 32];
        for row in 0..size {
            for column in 0..size {
                let base = ((row * 16 + column) * 32) as usize;
                let mut position = base + 1;
                let own = self.pattern_colour(row, column);
                let mut sums = [0i32; 4];
                for y in -1..=1i32 {
                    for x in -1..=1i32 {
                        let shift = (y == 0) as i32 + (x == 0) as i32;
                        let colour = self.pattern_colour(row + y + 48, column + x + 48);
                        if colour == own {
                            continue;
                        }
                        code[position] = (width * y + x) * 4 + colour as i32;
                        code[position + 1] = shift;
                        code[position + 2] = colour as i32;
                        position += 3;
                        sums[colour] += 1 << shift;
                    }
                }
                code[base] = ((position - base) / 3) as i32;
                for (colour, &sum) in sums.iter().enumerate().take(colors) {
                    if colour != own {
                        code[position] = colour as i32;
                        code[position + 1] = if sum > 0 { 256 / sum } else { 0 };
                        position += 2;
                    }
                }
            }
        }
        // lin_interpolate_loop: writes only the pixel's missing colours.
        for row in 1..height - 1 {
            for column in 1..width - 1 {
                let mut position = (((row % size) * 16 + column % size) * 32) as usize;
                let pixel = (row * width + column) as usize;
                let mut sums = [0i32; 4];
                let count = code[position];
                position += 1;
                for _ in 0..count {
                    let value = self.sample(pixel, code[position]);
                    sums[code[position + 2] as usize] += value << code[position + 1];
                    position += 3;
                }
                for _ in 1..colors {
                    let channel = code[position] as usize;
                    self.pixels[pixel][channel] = ((sums[channel] * code[position + 1]) >> 8) as u16;
                    position += 2;
                }
            }
        }
    }

    /// `vng_interpolate`: Variable Number of Gradients — per pixel, the
    /// colour differences of the neighbours in the directions whose
    /// gradients are below a threshold.
    pub(super) fn vng_interpolate(&mut self) {
        self.lin_interpolate();
        let width = self.width as i32;
        let height = self.height as i32;
        let colors = self.state.colors.min(4);
        let (pattern_rows, pattern_columns) = match self.state.filters {
            1 => (16, 16),
            9 => (6, 6),
            _ => (8, 2),
        };
        // The terms and neighbours of each pattern position.
        let mut codes: Vec<Vec<i32>> = Vec::with_capacity(pattern_rows * pattern_columns);
        for row in 0..pattern_rows as i32 {
            for column in 0..pattern_columns as i32 {
                let mut code = Vec::new();
                for term in VNG_TERMS.as_chunks::<6>().0 {
                    let [y1, x1, y2, x2, weight, gradients] = term.map(|value| value as i32);
                    let colour = self.pattern_colour(row + y1 + 144, column + x1 + 144);
                    if self.pattern_colour(row + y2 + 144, column + x2 + 144) != colour {
                        continue;
                    }
                    let diagonal = if self.pattern_colour(row, column + 1) == colour && self.pattern_colour(row + 1, column) == colour { 2 } else { 1 };
                    if (y1 - y2).abs() == diagonal && (x1 - x2).abs() == diagonal {
                        continue;
                    }
                    code.push((y1 * width + x1) * 4 + colour as i32);
                    code.push((y2 * width + x2) * 4 + colour as i32);
                    code.push(weight);
                    for gradient in 0..8 {
                        if gradients & (1 << gradient) != 0 {
                            code.push(gradient);
                        }
                    }
                    code.push(-1);
                }
                code.push(i32::MAX);
                for neighbour in VNG_NEIGHBOURS.as_chunks::<2>().0 {
                    let [y, x] = neighbour.map(|value| value as i32);
                    code.push((y * width + x) * 4);
                    let colour = self.pattern_colour(row, column);
                    if self.pattern_colour(row + y + 144, column + x + 144) != colour && self.pattern_colour(row + y * 2 + 144, column + x * 2 + 144) == colour {
                        code.push((y * width + x) * 8 + colour as i32);
                    } else {
                        code.push(0);
                    }
                }
                codes.push(code);
            }
        }

        // Three rows of results, rotated: a row is written back two rows
        // after it was computed.
        let row_width = self.width;
        let mut rows = [vec![[0u16; 4]; row_width], vec![[0u16; 4]; row_width], vec![[0u16; 4]; row_width]];
        let mut row = 2;
        while row < height - 2 {
            for column in 2..width - 2 {
                let pixel = (row * width + column) as usize;
                let code = &codes[(row as usize % pattern_rows) * pattern_columns + column as usize % pattern_columns];
                let mut position = 0;
                let mut gradients = [0i32; 8];
                while code[position] != i32::MAX {
                    let difference = (self.sample(pixel, code[position]) - self.sample(pixel, code[position + 1])).abs() << code[position + 2];
                    gradients[code[position + 3] as usize] += difference;
                    position += 5;
                    let gradient = code[position - 1];
                    if gradient == -1 {
                        continue;
                    }
                    gradients[gradient as usize] += difference;
                    loop {
                        let gradient = code[position];
                        position += 1;
                        if gradient == -1 {
                            break;
                        }
                        gradients[gradient as usize] += difference;
                    }
                }
                position += 1;
                let mut lowest = gradients[0];
                let mut highest = gradients[0];
                for &gradient in &gradients[1..] {
                    if lowest > gradient {
                        lowest = gradient;
                    }
                    if highest < gradient {
                        highest = gradient;
                    }
                }
                if highest == 0 {
                    rows[2][column as usize] = self.pixels[pixel];
                    continue;
                }
                let threshold = lowest + (highest >> 1);
                let mut sums = [0i32; 4];
                let colour = self.colour(row as usize, column as usize);
                let mut count = 0;
                for &gradient in &gradients {
                    if gradient <= threshold {
                        for (channel, sum) in sums.iter_mut().enumerate().take(colors) {
                            if channel == colour && code[position + 1] != 0 {
                                *sum += (self.pixels[pixel][channel] as i32 + self.sample(pixel, code[position + 1])) >> 1;
                            } else {
                                *sum += self.sample(pixel, code[position] + channel as i32);
                            }
                        }
                        count += 1;
                    }
                    position += 2;
                }
                let own = self.pixels[pixel][colour] as i32;
                for (channel, sum) in sums.iter().enumerate().take(colors) {
                    let mut value = own;
                    if channel != colour {
                        value += (sum - sums[colour]) / count;
                    }
                    rows[2][column as usize][channel] = clip(value);
                }
            }
            if row > 3 {
                let target = ((row - 2) * width + 2) as usize;
                self.pixels[target..target + row_width - 4].copy_from_slice(&rows[0][2..row_width - 2]);
            }
            rows.rotate_left(1);
            row += 1;
        }
        let target = ((row - 2) * width + 2) as usize;
        self.pixels[target..target + row_width - 4].copy_from_slice(&rows[0][2..row_width - 2]);
        let target = ((row - 1) * width + 2) as usize;
        self.pixels[target..target + row_width - 4].copy_from_slice(&rows[1][2..row_width - 2]);
    }

    /// `xtrans_interpolate`: Frank Markesteijn's algorithm for X-Trans:
    /// greens in four directions (eight with the extra passes), red and
    /// blue from them, then per pixel the average of the directions whose
    /// CIELab derivatives are most homogeneous.
    pub(super) fn xtrans_interpolate(&mut self, passes: usize) -> Result<()> {
        const ORTHOGONAL: [i32; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];
        const PATTERNS: [[i32; 16]; 2] = [[0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0], [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1]];
        let tile = TILE as i64;
        let directions: [i64; 4] = [1, tile, tile + 1, tile - 1];
        let width = self.width as i32;
        let height = self.height as i32;
        if width < TILE as i32 || height < TILE as i32 || self.state.filters != 9 {
            bail!("the X-Trans image is too small to interpolate");
        }
        let mut counts = [0; 4];
        for row in 0..6 {
            for column in 0..6 {
                counts[self.pattern_colour(row, column)] += 1;
            }
        }
        if counts[0] < 6 || counts[0] > 10 || counts[1] < 16 || counts[1] > 24 || counts[2] < 6 || counts[2] > 10 || counts[3] != 0 {
            bail!("the X-Trans pattern is not one LibRaw knows");
        }

        // A hexagon of greens around each non-green pixel and of
        // non-greens around each green, as image and tile offsets.
        let mut hexagons = [[[[32700i16; 8]; 2]; 3]; 3];
        let cielab = Cielab::new(self.state);
        let directions_count = 4 << (passes > 1) as usize;
        let (mut min_v, mut max_v, mut min_h, mut max_h) = (0, 0, 0, 0);
        let mut solitary_row = 0i32;
        let mut solitary_column = 0i32;
        for row in 0..3i32 {
            for column in 0..3i32 {
                let mut non_greens = 0;
                let green = (self.pattern_colour(row, column) == 1) as i32;
                for d in (0..10).step_by(2) {
                    if self.pattern_colour(row + ORTHOGONAL[d], column + ORTHOGONAL[d + 2]) == 1 {
                        non_greens = 0;
                    } else {
                        non_greens += 1;
                    }
                    if non_greens == 4 {
                        solitary_row = row;
                        solitary_column = column;
                    }
                    if non_greens == green + 1 {
                        let pattern = &PATTERNS[green as usize];
                        for c in 0..8 {
                            let v = ORTHOGONAL[d] * pattern[c * 2] + ORTHOGONAL[d + 1] * pattern[c * 2 + 1];
                            let h = ORTHOGONAL[d + 2] * pattern[c * 2] + ORTHOGONAL[d + 3] * pattern[c * 2 + 1];
                            min_v = v.min(min_v);
                            max_v = v.max(max_v);
                            // LibRaw tracks the horizontal range with v too.
                            min_h = v.min(min_h);
                            max_h = v.max(max_h);
                            let slot = c ^ ((green as usize * 2) & d);
                            hexagons[row as usize][column as usize][0][slot] = (h + v * width) as i16;
                            hexagons[row as usize][column as usize][1][slot] = (h + v * TILE as i32) as i16;
                        }
                    }
                }
            }
        }
        for offset in hexagons.iter().flatten().flatten().flatten() {
            let offset = *offset as i32;
            if offset > max_h + max_v * width + 1 || offset < min_h + min_v * width - 1 {
                bail!("the X-Trans pattern is not one LibRaw knows");
            }
        }

        // Each non-green pixel's green range (channel 1 the lowest, 3 the
        // highest), shared by the vertical pairs.
        let mut retries = 0i64;
        let mut row = 2i32;
        while row < height - 2 {
            let mut column = 2i32;
            let mut highest: u16 = 0;
            let mut lowest: u16 = 0xffff;
            while column < width - 2 {
                if self.pattern_colour(row, column) == 1 {
                    highest = 0;
                    lowest = 0xffff;
                    column += 1;
                    continue;
                }
                let pixel = (row * width + column) as i64;
                let hexagon = hexagons[(row % 3) as usize][(column % 3) as usize][0];
                if highest == 0 {
                    for &offset in &hexagon[..6] {
                        let value = self.pixels[(pixel + offset as i64) as usize][1];
                        if lowest > value {
                            lowest = value;
                        }
                        if highest < value {
                            highest = value;
                        }
                    }
                }
                self.pixels[pixel as usize][1] = lowest;
                self.pixels[pixel as usize][3] = highest;
                match (row - solitary_row) % 3 {
                    1 => {
                        if row < height - 3 {
                            row += 1;
                            column -= 1;
                        }
                    }
                    2 => {
                        highest = 0;
                        lowest = 0xffff;
                        column += 2;
                        if column < width - 3 && row > 2 {
                            row -= 1;
                            if retries > (width * height) as i64 {
                                bail!("the X-Trans image is damaged");
                            }
                            retries += 1;
                        }
                    }
                    _ => {}
                }
                column += 1;
            }
            row += 1;
        }

        for row in 3..9.min(height - 3) {
            for column in 3..9.min(width - 3) {
                if self.pattern_colour(row, column) == 1 {
                    continue;
                }
                let hexagon = hexagons[(row % 3) as usize][(column % 3) as usize][0];
                for c in 0..2 {
                    let centre = (row * width + column) as i64;
                    let ahead = 3 * hexagon[4 + c] as i64 + centre;
                    let behind = -3 * hexagon[4 + c] as i64 + centre;
                    let limit = (width * height) as i64;
                    if ahead < 0 || ahead >= limit || behind < 0 || behind >= limit {
                        bail!("the X-Trans image is damaged");
                    }
                }
            }
        }

        let plane = TILE * TILE;
        let mut rgb = vec![[0u16; 3]; directions_count * plane];
        let mut lab = vec![[0i16; 3]; plane];
        let mut derivatives = vec![0.0f32; directions_count * plane];
        let mut homogeneity = vec![0u8; directions_count * plane];
        let mut top = 3i32;
        while top < height - 19 {
            let mut left = 3i32;
            while left < width - 19 {
                self.xtrans_tile(top, left, passes, &hexagons, (solitary_row, solitary_column), &mut rgb);
                let tile_rows = (top + TILE as i32).min(height - 3) - top;
                let tile_columns = (left + TILE as i32).min(width - 3) - left;

                // CIELab and its derivatives in each direction.
                for d in 0..directions_count {
                    for row in 2..tile_rows - 2 {
                        for column in 2..tile_columns - 2 {
                            let index = row as usize * TILE + column as usize;
                            lab[index] = cielab.convert(rgb[d * plane + index]);
                        }
                    }
                    let step = directions[d & 3];
                    for row in 3..tile_rows - 3 {
                        for column in 3..tile_columns - 3 {
                            let index = (row as i64) * tile + column as i64;
                            let centre = lab[index as usize];
                            let ahead = lab[(index + step) as usize];
                            let behind = lab[(index - step) as usize];
                            let g = 2 * centre[0] as i32 - ahead[0] as i32 - behind[0] as i32;
                            let a = 2 * centre[1] as i32 - ahead[1] as i32 - behind[1] as i32 + g * 500 / 232;
                            let b = 2 * centre[2] as i32 - ahead[2] as i32 - behind[2] as i32 - g * 500 / 580;
                            let sum = g.wrapping_mul(g).wrapping_add(a.wrapping_mul(a)).wrapping_add(b.wrapping_mul(b));
                            derivatives[d * plane + index as usize] = sum as f32;
                        }
                    }
                }

                // Homogeneity: neighbours within 8× the smallest derivative.
                homogeneity.fill(0);
                for row in 4..tile_rows - 4 {
                    for column in 4..tile_columns - 4 {
                        let index = row as usize * TILE + column as usize;
                        let mut threshold = f32::MAX;
                        for d in 0..directions_count {
                            if threshold > derivatives[d * plane + index] {
                                threshold = derivatives[d * plane + index];
                            }
                        }
                        threshold *= 8.0;
                        for d in 0..directions_count {
                            for v in -1..=1i64 {
                                for h in -1..=1i64 {
                                    let neighbour = (index as i64 + v * tile + h) as usize;
                                    if derivatives[d * plane + neighbour] <= threshold {
                                        homogeneity[d * plane + index] += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                // The average of the most homogeneous directions.
                let mut last_row = tile_rows;
                let mut last_column = tile_columns;
                if height - top < TILE as i32 + 4 {
                    last_row = height - top + 2;
                }
                if width - left < TILE as i32 + 4 {
                    last_column = width - left + 2;
                }
                for row in top.min(8)..last_row - 8 {
                    for column in left.min(8)..last_column - 8 {
                        let index = row as usize * TILE + column as usize;
                        let mut scores = [0i32; 8];
                        for (d, score) in scores.iter_mut().enumerate().take(directions_count) {
                            for v in -2..=2i64 {
                                for h in -2..=2i64 {
                                    *score += homogeneity[d * plane + (index as i64 + v * tile + h) as usize] as i32;
                                }
                            }
                        }
                        for d in 0..directions_count - 4 {
                            if scores[d] < scores[d + 4] {
                                scores[d] = 0;
                            } else if scores[d] > scores[d + 4] {
                                scores[d + 4] = 0;
                            }
                        }
                        let mut best = scores[0] as u16;
                        for &score in &scores[1..directions_count] {
                            if (best as i32) < score {
                                best = score as u16;
                            }
                        }
                        best -= best >> 3;
                        let mut sums = [0i32; 4];
                        for (d, &score) in scores.iter().enumerate().take(directions_count) {
                            if score >= best as i32 {
                                for (channel, sum) in sums.iter_mut().enumerate().take(3) {
                                    *sum += rgb[d * plane + index][channel] as i32;
                                }
                                sums[3] += 1;
                            }
                        }
                        let pixel = ((row + top) * width + column + left) as usize;
                        for channel in 0..3 {
                            self.pixels[pixel][channel] = (sums[channel] / sums[3]) as u16;
                        }
                    }
                }
                left += TILE as i32 - 16;
            }
            top += TILE as i32 - 16;
        }
        self.border_interpolate(8);
        Ok(())
    }

    /// One X-Trans tile's interpolation into `rgb` (one plane per
    /// direction): greens, then each pass's red and blue.
    fn xtrans_tile(&self, top: i32, left: i32, passes: usize, hexagons: &[[[[i16; 8]; 2]; 3]; 3], solitary: (i32, i32), rgb: &mut [[u16; 3]]) {
        let tile = TILE as i64;
        let plane = TILE * TILE;
        let width = self.width as i32;
        let height = self.height as i32;
        let (solitary_row, solitary_column) = solitary;
        let last_row = (top + TILE as i32).min(height - 3);
        let last_column = (left + TILE as i32).min(width - 3);
        let position = |row: i32, column: i32| -> usize { (row - top) as usize * TILE + (column - left) as usize };
        let pixels = &self.pixels;
        let at = |pixel: i64, offset: i64, channel: usize| -> i32 { pixels[(pixel + offset) as usize][channel] as i32 };
        // !((row - sgrow) % 3): which direction pairs swap on this row.
        let swaps = |row: i32| -> usize { ((row - solitary_row) % 3 == 0) as usize };

        for row in top..last_row {
            for column in left..last_column {
                let source = pixels[(row * width + column) as usize];
                rgb[position(row, column)] = [source[0], source[1], source[2]];
            }
        }
        for copy in 1..4 {
            rgb.copy_within(0..plane, copy * plane);
        }

        // Green horizontally, vertically and along both diagonals.
        let mut colours = [[0i32; 8]; 3];
        for row in top..last_row {
            for column in left..last_column {
                let own = self.pattern_colour(row, column);
                if own == 1 {
                    continue;
                }
                let pixel = (row * width + column) as i64;
                let hexagon = hexagons[(row % 3) as usize][(column % 3) as usize][0].map(|offset| offset as i64);
                colours[1][0] = 174 * (at(pixel, hexagon[1], 1) + at(pixel, hexagon[0], 1)) - 46 * (at(pixel, 2 * hexagon[1], 1) + at(pixel, 2 * hexagon[0], 1));
                colours[1][1] = 223 * at(pixel, hexagon[3], 1) + at(pixel, hexagon[2], 1) * 33 + 92 * (at(pixel, 0, own) - at(pixel, -hexagon[2], own));
                for c in 0..2 {
                    colours[1][2 + c] = 164 * at(pixel, hexagon[4 + c], 1) + 92 * at(pixel, -2 * hexagon[4 + c], 1) + 33 * (2 * at(pixel, 0, own) - at(pixel, 3 * hexagon[4 + c], own) - at(pixel, -3 * hexagon[4 + c], own));
                }
                let lowest = at(pixel, 0, 1);
                let highest = at(pixel, 0, 3);
                for c in 0..4 {
                    let value = (colours[1][c] >> 8).min(highest).max(lowest);
                    rgb[(c ^ swaps(row)) * plane + position(row, column)][1] = value as u16;
                }
            }
        }

        // The planes the passes work in: the first four, then (from the
        // second pass) the copies in the last four.
        let mut base = 0usize;
        for pass in 0..passes {
            if pass == 1 {
                rgb.copy_within(0..4 * plane, 4 * plane);
                base = 4;
            }

            // Green again, from the interpolated values of closer pixels.
            if pass > 0 {
                for row in top + 2..last_row - 2 {
                    for column in left + 2..last_column - 2 {
                        let own = self.pattern_colour(row, column);
                        if own == 1 {
                            continue;
                        }
                        let pixel = (row * width + column) as i64;
                        let hexagon = hexagons[(row % 3) as usize][(column % 3) as usize][1].map(|offset| offset as i64);
                        let lowest = at(pixel, 0, 1);
                        let highest = at(pixel, 0, 3);
                        for d in 3..6 {
                            let index = ((base + ((d - 2) ^ swaps(row))) * plane + position(row, column)) as i64;
                            let behind = rgb[(index - 2 * hexagon[d]) as usize];
                            let ahead = rgb[(index + hexagon[d]) as usize];
                            let centre = rgb[index as usize];
                            let value = behind[1] as i32 + 2 * ahead[1] as i32 - behind[own] as i32 - 2 * ahead[own] as i32 + 3 * centre[own] as i32;
                            rgb[index as usize][1] = (value / 3).min(highest).max(lowest) as u16;
                        }
                    }
                }
            }

            // Red and blue for solitary green pixels.
            let mut row = (top - solitary_row + 4) / 3 * 3 + solitary_row;
            while row < last_row - 2 {
                let mut column = (left - solitary_column + 4) / 3 * 3 + solitary_column;
                while column < last_column - 2 {
                    let mut index = (base * plane + position(row, column)) as i64;
                    let mut h = self.pattern_colour(row, column + 1);
                    if h == 1 {
                        break;
                    }
                    let mut differences = [0.0f32; 6];
                    let mut step: i64 = 1;
                    for d in 0..6 {
                        for c in 0..2 {
                            let offset = step << c;
                            let ahead = rgb[(index + offset) as usize];
                            let behind = rgb[(index - offset) as usize];
                            let g = 2 * rgb[index as usize][1] as i32 - ahead[1] as i32 - behind[1] as i32;
                            colours[h][d] = g + ahead[h] as i32 + behind[h] as i32;
                            if d > 1 {
                                let difference = ahead[1] as f32 - behind[1] as f32 - ahead[h] as f32 + behind[h] as f32;
                                differences[d] += difference * difference + (g as f32) * (g as f32);
                            }
                            h ^= 2;
                        }
                        if d > 1 && d & 1 == 1 && differences[d - 1] < differences[d] {
                            for c in 0..2 {
                                colours[c * 2][d] = colours[c * 2][d - 1];
                            }
                        }
                        if d < 2 || d & 1 == 1 {
                            for c in 0..2 {
                                rgb[index as usize][c * 2] = clip(colours[c * 2][d] / 2);
                            }
                            index += plane as i64;
                        }
                        step ^= tile ^ 1;
                        h ^= 2;
                    }
                    column += 3;
                }
                row += 3;
            }

            // Red for blue pixels and blue for red ones.
            for row in top + 3..last_row - 3 {
                for column in left + 3..last_column - 3 {
                    let opposite = 2 - self.pattern_colour(row, column) as i32;
                    if opposite == 1 {
                        continue;
                    }
                    let opposite = opposite as usize;
                    let mut index = (base * plane + position(row, column)) as i64;
                    let c: i64 = if (row - solitary_row) % 3 != 0 { tile } else { 1 };
                    let h: i64 = 3 * (c ^ tile ^ 1);
                    for d in 0..4i64 {
                        let green = |offset: i64| -> i32 { rgb[(index + offset) as usize][1] as i32 };
                        let centre = green(0);
                        let along_c = (centre - green(c)).abs() + (centre - green(-c)).abs();
                        let along_h = (centre - green(h)).abs() + (centre - green(-h)).abs();
                        let i = if d > 1 || (d ^ c) & 1 != 0 || along_c < 2 * along_h { c } else { h };
                        let ahead = rgb[(index + i) as usize];
                        let behind = rgb[(index - i) as usize];
                        let value = (ahead[opposite] as i32 + behind[opposite] as i32 + 2 * centre - ahead[1] as i32 - behind[1] as i32) / 2;
                        rgb[index as usize][opposite] = clip(value);
                        index += plane as i64;
                    }
                }
            }

            // Red and blue for 2×2 blocks of green.
            for row in top + 2..last_row - 2 {
                if (row - solitary_row) % 3 == 0 {
                    continue;
                }
                for column in left + 2..last_column - 2 {
                    if (column - solitary_column) % 3 == 0 {
                        continue;
                    }
                    let mut index = (base * plane + position(row, column)) as i64;
                    let hexagon = hexagons[(row % 3) as usize][(column % 3) as usize][1].map(|offset| offset as i64);
                    for d in (0..8).step_by(2) {
                        let first = rgb[(index + hexagon[d]) as usize];
                        let second = rgb[(index + hexagon[d + 1]) as usize];
                        let centre = rgb[index as usize][1] as i32;
                        if hexagon[d] + hexagon[d + 1] != 0 {
                            let g = 3 * centre - 2 * first[1] as i32 - second[1] as i32;
                            for c in [0, 2] {
                                rgb[index as usize][c] = clip((g + 2 * first[c] as i32 + second[c] as i32) / 3);
                            }
                        } else {
                            let g = 2 * centre - first[1] as i32 - second[1] as i32;
                            for c in [0, 2] {
                                rgb[index as usize][c] = clip((g + first[c] as i32 + second[c] as i32) / 2);
                            }
                        }
                        index += plane as i64;
                    }
                }
            }
        }
    }
}
