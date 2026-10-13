//! LibRaw's `dcraw_process` and `dcraw_make_mem_image` with IM's settings
//! (`use_camera_wb` 1, `output_bps` 16, `user_flip` 0, the rest LibRaw's
//! defaults: AHD demosaicing, sRGB output, BT.709 gamma 0.45/4.5 and
//! automatic brightness from the 1% brightest pixels). The arithmetic
//! follows the C types: float scaling and colour conversion, truncating
//! conversions, int black levels.

use anyhow::Result;

use super::identify::{fc, fcol, Raw, D65_WHITE, XYZ_RGB};
use super::unpack::RawData;

/// `LIBRAW_AHD_TILE`.
pub(super) const TILE: usize = 512;
/// `LIBRAW_HISTOGRAM_SIZE`.
const HISTOGRAM_SIZE: usize = 0x2000;
/// `gamm[0..2]`: BT.709's power and toe slope.
const GAMMA_POWER: f64 = 0.45;
const GAMMA_SLOPE: f64 = 4.5;

/// What `dcraw_make_mem_image` returns: `colors` 16-bit samples a pixel.
pub(super) struct Output {
    pub width: usize,
    pub height: usize,
    pub colors: usize,
    pub samples: Vec<u16>,
}

/// `imgdata.image` and the fields the processing steps change.
pub(super) struct Image<'a> {
    pub state: &'a mut Raw,
    pub pixels: Vec<[u16; 4]>,
    pub width: usize,
    pub height: usize,
    data_maximum: u32,
}

/// `CLIP`.
pub(super) fn clip(value: i32) -> u16 {
    value.clamp(0, 65535) as u16
}

pub(super) fn process(state: &mut Raw, raw: RawData) -> Result<Output> {
    let width = state.width as usize;
    let height = state.height as usize;
    let is_bayer = state.filters != 0 || state.colors == 1;
    let subtract_inline = is_bayer;
    let mut image = Image { state, pixels: Vec::new(), width, height, data_maximum: 0 };
    image.raw2image(&raw, subtract_inline);
    drop(raw);

    if !subtract_inline || image.data_maximum == 0 {
        adjust_bl(image.state);
        image.subtract_black();
    }
    image.adjust_maximum();
    image.scale_colors();
    image.pre_interpolate();
    // Quality 3: AHD for Bayer patterns, Markesteijn's three passes for
    // X-Trans, VNG for four colours and other patterns.
    let filters = image.state.filters;
    if filters == 9 {
        image.xtrans_interpolate(3)?;
    } else if filters != 0 && (image.state.colors > 3 || filters <= 1000) {
        image.vng_interpolate();
    } else if filters != 0 {
        image.ahd_interpolate();
    }
    let histogram = image.convert_to_rgb();
    image.stretch();
    Ok(image.make_mem_image(&histogram))
}

/// `adjust_bl`: folds the repeat pattern of a 2×2 CFA into the four
/// channel blacks, moves their common part (and the pattern's) into
/// `black`, then adds `black` back to each channel.
fn adjust_bl(state: &mut Raw) {
    let cblack = &mut state.cblack;
    if state.filters > 1000 && cblack[4].div_ceil(2) == 1 && cblack[5].div_ceil(2) == 1 {
        let mut colours = [0usize; 4];
        let mut last_green = None;
        let mut greens = 0;
        for (index, colour) in colours.iter_mut().enumerate() {
            *colour = fc(state.filters, index as u32 / 2, index as u32 % 2) as usize;
            if *colour == 1 {
                greens += 1;
                last_green = Some(index);
            }
        }
        if let (true, Some(index)) = (greens > 1, last_green) {
            colours[index] = 3;
        }
        for (index, &colour) in colours.iter().enumerate() {
            let pattern = 6 + (index / 2 % cblack[4] as usize) * cblack[5] as usize + index % 2 % cblack[5] as usize;
            cblack[colour] = cblack[colour].wrapping_add(cblack[pattern]);
        }
        cblack[4] = 0;
        cblack[5] = 0;
    } else if state.filters <= 1000 && cblack[4] == 1 && cblack[5] == 1 {
        let pattern_black = cblack[6];
        for value in cblack[..4].iter_mut() {
            *value = value.wrapping_add(pattern_black);
        }
        cblack[4] = 0;
        cblack[5] = 0;
    }
    // The channels' common part (compared as int).
    let mut common = cblack[3] as i32;
    for &value in &cblack[..3] {
        if common > value as i32 {
            common = value as i32;
        }
    }
    for value in cblack[..4].iter_mut() {
        *value = value.wrapping_sub(common as u32);
    }
    state.black = state.black.wrapping_add(common as u32);
    if cblack[4] != 0 && cblack[5] != 0 {
        let length = (cblack[4] * cblack[5]) as usize;
        let pattern = &mut cblack[6..6 + length];
        let mut common = pattern[0] as i32;
        for &value in pattern.iter().skip(1) {
            if common > value as i32 {
                common = value as i32;
            }
        }
        let mut nonzero = 0;
        for value in pattern.iter_mut() {
            *value = value.wrapping_sub(common as u32);
            if *value != 0 {
                nonzero += 1;
            }
        }
        state.black = state.black.wrapping_add(common as u32);
        if nonzero == 0 {
            cblack[4] = 0;
            cblack[5] = 0;
        }
    }
    let black = state.black;
    for value in cblack[..4].iter_mut() {
        *value = value.wrapping_add(black);
    }
}

impl Image<'_> {
    /// The colour of a pixel of the (visible) image: `fcol`.
    pub(super) fn colour(&self, row: usize, column: usize) -> usize {
        fcol(self.state, row as i32, column as i32)
    }

    /// `raw2image_ex`: the visible area of the raw data into `image`,
    /// subtracting the channel blacks of CFA data on the way.
    fn raw2image(&mut self, raw: &RawData, subtract_black: bool) {
        let width = self.width;
        let height = self.height;
        let extra = match self.state.filters {
            0 => 0,
            9 => 6,
            _ => 2,
        };
        self.pixels = vec![[0u16; 4]; (width + extra) * (height + extra)];
        let mut cblack = [0u16; 4];
        if subtract_black {
            adjust_bl(self.state);
            for (value, &black) in cblack.iter_mut().zip(self.state.cblack.iter()) {
                *value = black as u16;
            }
        }
        let raw_width = self.state.raw_width as usize;
        let raw_height = self.state.raw_height as usize;
        let top = self.state.top_margin as usize;
        let left = self.state.left_margin as usize;
        let copy_height = height.min(raw_height.saturating_sub(top));
        let copy_width = width.min(raw_width.saturating_sub(left));
        let mut data_maximum = 0u16;
        match raw {
            RawData::Bayer(raw_image) => {
                for row in 0..copy_height {
                    for column in 0..copy_width {
                        let mut value = raw_image[(row + top) * raw_width + column + left];
                        let colour = self.colour(row, column);
                        if value > cblack[colour] {
                            value -= cblack[colour];
                            data_maximum = data_maximum.max(value);
                        } else {
                            value = 0;
                        }
                        self.pixels[row * width + column][colour] = value;
                    }
                }
            }
            RawData::Color4(color4) => {
                for row in 0..copy_height {
                    let source = (row + top) * raw_width + left;
                    self.pixels[row * width..row * width + copy_width].copy_from_slice(&color4[source..source + copy_width]);
                }
            }
            RawData::Color3(color3) => {
                for row in 0..copy_height {
                    for column in 0..copy_width {
                        let pixel = color3[(row + top) * raw_width + left + column];
                        self.pixels[row * width + column] = [pixel[0], pixel[1], pixel[2], 0];
                    }
                }
            }
        }
        if subtract_black {
            self.data_maximum = data_maximum as u32;
            self.state.maximum = self.state.maximum.wrapping_sub(self.state.black);
            self.state.cblack[..4].fill(0);
            self.state.black = 0;
        }
    }

    /// `subtract_black_internal`.
    fn subtract_black(&mut self) {
        let cblack = &self.state.cblack;
        let size = self.width * self.height;
        if cblack[..4].iter().any(|&black| black != 0) || (cblack[4] != 0 && cblack[5] != 0) {
            let channel_black = [cblack[0] as i32, cblack[1] as i32, cblack[2] as i32, cblack[3] as i32];
            let pattern = cblack[4] != 0 && cblack[5] != 0;
            let mut data_maximum = 0i32;
            let width = self.width;
            for (index, pixel) in self.pixels[..size].iter_mut().enumerate() {
                let pattern_black = if pattern {
                    let row = index / width % cblack[4] as usize;
                    let column = index % width % cblack[5] as usize;
                    cblack[6 + row * cblack[5] as usize + column] as i32
                } else {
                    0
                };
                for (value, black) in pixel.iter_mut().zip(channel_black) {
                    let difference = *value as i32 - pattern_black - black;
                    *value = clip(difference);
                    data_maximum = data_maximum.max(difference);
                }
            }
            self.data_maximum = (data_maximum & 0xffff) as u32;
            self.state.maximum = self.state.maximum.wrapping_sub(self.state.black);
            self.state.cblack.fill(0);
            self.state.black = 0;
        } else {
            let mut data_maximum = 0u16;
            for pixel in &self.pixels[..size] {
                for &value in pixel {
                    data_maximum = data_maximum.max(value);
                }
            }
            self.data_maximum = data_maximum as u32;
        }
    }

    /// `adjust_maximum`: the data's real maximum, if within 75% of it.
    fn adjust_maximum(&mut self) {
        let real_maximum = self.data_maximum & 0xffff;
        let maximum = self.state.maximum;
        if real_maximum > 0 && real_maximum < maximum && real_maximum as f32 > maximum as f32 * 0.75 {
            self.state.maximum = real_maximum;
        }
    }

    /// `scale_colors`: the camera white balance (or the grey world when
    /// the file has none), normalised to the smallest multiplier and the
    /// white level.
    fn scale_colors(&mut self) {
        let state = &mut *self.state;
        if state.cam_mul[0] < -0.5 || state.cam_mul[0] <= 0.00001 {
            // Automatic white balance over 8×8 blocks without clipped
            // pixels.
            let mut sums = [0.0f64; 8];
            let width = self.width;
            let height = self.height;
            let mut row = 0;
            while row < height {
                let mut column = 0;
                while column < width {
                    let mut block = [0u32; 8];
                    let mut clipped = false;
                    'block: for y in row..(row + 8).min(height) {
                        for x in column..(column + 8).min(width) {
                            for channel in 0..4 {
                                let (colour, value) = if state.filters != 0 {
                                    let colour = fcol(state, y as i32, x as i32);
                                    (colour, self.pixels[y * width + x][colour] as i32)
                                } else {
                                    (channel, self.pixels[y * width + x][channel] as i32)
                                };
                                if value > state.maximum as i32 - 25 {
                                    clipped = true;
                                    break 'block;
                                }
                                let value = (value - state.cblack[colour] as i32).max(0);
                                block[colour] = block[colour].wrapping_add(value as u32);
                                block[colour + 4] += 1;
                                if state.filters != 0 {
                                    break;
                                }
                            }
                        }
                    }
                    if !clipped {
                        for (sum, value) in sums.iter_mut().zip(block) {
                            *sum += value as f64;
                        }
                    }
                    column += 8;
                }
                row += 8;
            }
            for channel in 0..4 {
                if sums[channel] != 0.0 {
                    state.pre_mul[channel] = (sums[channel + 4] / sums[channel]) as f32;
                }
            }
        }
        if state.cam_mul[0] > 0.00001 {
            // The camera's multipliers (LibRaw's white table is empty for
            // DNG files).
            if state.cam_mul[2] > 0.00001 {
                state.pre_mul = state.cam_mul;
            }
        }
        if state.pre_mul[1] == 0.0 {
            state.pre_mul[1] = 1.0;
        }
        if state.pre_mul[3] == 0.0 {
            state.pre_mul[3] = if state.colors < 4 { state.pre_mul[1] } else { 1.0 };
        }
        state.maximum = state.maximum.wrapping_sub(state.black);
        let mut minimum = f64::MAX;
        for &value in &state.pre_mul {
            if minimum > value as f64 {
                minimum = value as f64;
            }
        }
        // highlight 0: scale to the smallest multiplier.
        let mut scale = [1.0f32; 4];
        if minimum > 0.00001 && state.maximum > 0 {
            let maximum = state.maximum as f32;
            for (multiplier, value) in state.pre_mul.iter_mut().zip(scale.iter_mut()) {
                *multiplier /= minimum as f32;
                *value = *multiplier * 65535.0 / maximum;
            }
        }
        let cblack = &mut state.cblack;
        if state.filters > 1000 && cblack[4].div_ceil(2) == 1 && cblack[5].div_ceil(2) == 1 {
            for index in 0..4u32 {
                let colour = fc(state.filters, index / 2, index % 2) as usize;
                let pattern = 6 + (index as usize / 2 % cblack[4] as usize) * cblack[5] as usize + index as usize % 2 % cblack[5] as usize;
                cblack[colour] = cblack[colour].wrapping_add(cblack[pattern]);
            }
            cblack[4] = 0;
            cblack[5] = 0;
        }

        // scale_colors_loop: zero samples (missing colours) stay zero when
        // a black is subtracted.
        let size = self.width * self.height;
        let width = self.width;
        let channel_black = [cblack[0] as i32, cblack[1] as i32, cblack[2] as i32, cblack[3] as i32];
        if cblack[4] != 0 && cblack[5] != 0 {
            for (index, pixel) in self.pixels[..size].iter_mut().enumerate() {
                let row = index / width % cblack[4] as usize;
                let column = index % width % cblack[5] as usize;
                let pattern_black = cblack[6 + row * cblack[5] as usize + column] as i32;
                for ((value, black), factor) in pixel.iter_mut().zip(channel_black).zip(scale) {
                    if *value == 0 {
                        continue;
                    }
                    let difference = *value as i32 - pattern_black - black;
                    *value = clip((difference as f32 * factor) as i32);
                }
            }
        } else if channel_black.iter().any(|&black| black != 0) {
            for pixel in self.pixels[..size].iter_mut() {
                for ((value, black), factor) in pixel.iter_mut().zip(channel_black).zip(scale) {
                    if *value == 0 {
                        continue;
                    }
                    let difference = *value as i32 - black;
                    *value = clip((difference as f32 * factor) as i32);
                }
            }
        } else {
            for pixel in self.pixels[..size].iter_mut() {
                for (value, factor) in pixel.iter_mut().zip(scale) {
                    *value = clip((*value as f32 * factor) as i32);
                }
            }
        }
    }

    /// `pre_interpolate`: the second green of a Bayer pattern (colour 3)
    /// back into channel 1.
    fn pre_interpolate(&mut self) {
        let filters = self.state.filters;
        if filters > 1000 && self.state.colors == 3 {
            let width = self.width;
            let mut row = (fc(filters, 1, 0) >> 1) as usize;
            while row < self.height {
                let mut column = (fc(filters, row as u32, 1) & 1) as usize;
                while column < width {
                    let pixel = &mut self.pixels[row * width + column];
                    pixel[1] = pixel[3];
                    column += 2;
                }
                row += 2;
            }
            self.state.filters &= !((filters & 0x55555555) << 1);
        }
    }

    /// `border_interpolate`: each missing colour of the pixels within
    /// `border` of an edge, the mean of its 3×3 neighbours of that colour.
    pub(super) fn border_interpolate(&mut self, border: usize) {
        let width = self.width;
        let height = self.height;
        let colors = self.state.colors;
        for row in 0..height {
            let mut column = 0;
            while column < width {
                if column == border && row >= border && row < height.wrapping_sub(border) {
                    column = width - border;
                }
                let mut sums = [0u32; 8];
                for y_offset in 0..3 {
                    let y = (row + y_offset).wrapping_sub(1);
                    for x_offset in 0..3 {
                        let x = (column + x_offset).wrapping_sub(1);
                        if y < height && x < width {
                            let colour = self.colour(y, x);
                            sums[colour] += self.pixels[y * width + x][colour] as u32;
                            sums[colour + 4] += 1;
                        }
                    }
                }
                let own = self.colour(row, column);
                for channel in 0..colors {
                    if channel != own && sums[channel + 4] != 0 {
                        self.pixels[row * width + column][channel] = (sums[channel] / sums[channel + 4]) as u16;
                    }
                }
                column += 1;
            }
        }
    }

    /// `ahd_interpolate`: Adaptive Homogeneity-Directed demosaicing in
    /// 512×512 tiles overlapping by 6: greens interpolated horizontally
    /// and vertically, red and blue from each, and per pixel the direction
    /// whose CIELab neighbourhood is more homogeneous. Tiles only read the
    /// pixels' own channels, which earlier tiles leave alone.
    fn ahd_interpolate(&mut self) {
        let lab = Cielab::new(self.state);
        self.border_interpolate(5);
        let mut rgb = [vec![[0u16; 3]; TILE * TILE], vec![[0u16; 3]; TILE * TILE]];
        let mut lab_tiles = [vec![[0i16; 3]; TILE * TILE], vec![[0i16; 3]; TILE * TILE]];
        let mut homogeneity = vec![[0u8; 2]; TILE * TILE];
        let height = self.height;
        let width = self.width;
        let mut top = 2;
        while top + 5 < height {
            let mut left = 2;
            while left + 5 < width {
                self.ahd_green(top, left, &mut rgb);
                for (rgb_tile, lab_tile) in rgb.iter_mut().zip(lab_tiles.iter_mut()) {
                    self.ahd_red_blue(top, left, rgb_tile, lab_tile, &lab);
                }
                self.ahd_homogeneity(top, left, &lab_tiles, &mut homogeneity);
                self.ahd_combine(top, left, &rgb, &homogeneity);
                left += TILE - 6;
            }
            top += TILE - 6;
        }
    }

    /// `ahd_interpolate_green_h_and_v`: green at red and blue pixels, from
    /// the row (direction 0) and the column (direction 1).
    fn ahd_green(&self, top: usize, left: usize, rgb: &mut [Vec<[u16; 3]>; 2]) {
        let width = self.width;
        let row_limit = (top + TILE).min(self.height - 2);
        let column_limit = (left + TILE).min(width - 2);
        let pixels = &self.pixels;
        for row in top..row_limit {
            let mut column = left + (self.colour(row, left) & 1);
            let colour = self.colour(row, column);
            while column < column_limit {
                let index = row * width + column;
                let at = |offset: isize, channel: usize| -> i32 { pixels[(index as isize + offset) as usize][channel] as i32 };
                let tile = (row - top) * TILE + column - left;
                let wide = width as isize;
                let value = ((at(-1, 1) + at(0, colour) + at(1, 1)) * 2 - at(-2, colour) - at(2, colour)) >> 2;
                rgb[0][tile][1] = ulim(value, at(-1, 1), at(1, 1)) as u16;
                let value = ((at(-wide, 1) + at(0, colour) + at(wide, 1)) * 2 - at(-2 * wide, colour) - at(2 * wide, colour)) >> 2;
                rgb[1][tile][1] = ulim(value, at(-wide, 1), at(wide, 1)) as u16;
                column += 2;
            }
        }
    }

    /// `ahd_interpolate_r_and_b_in_rgb_and_convert_to_cielab`: red and
    /// blue of one direction's tile, from colour differences to its
    /// greens, and the tile in CIELab.
    fn ahd_red_blue(&self, top: usize, left: usize, rgb: &mut [[u16; 3]], lab: &mut [[i16; 3]], cielab: &Cielab) {
        let width = self.width;
        let wide = width as isize;
        let row_limit = (top + TILE - 1).min(self.height - 3);
        let column_limit = (left + TILE - 1).min(width - 3);
        let pixels = &self.pixels;
        let span = TILE as isize;
        for row in top + 1..row_limit {
            for column in left + 1..column_limit {
                let index = row * width + column;
                let at = |offset: isize, channel: usize| -> i32 { pixels[(index as isize + offset) as usize][channel] as i32 };
                let tile = (row - top) * TILE + column - left;
                let green = |rgb: &[[u16; 3]], offset: isize| -> i32 { rgb[(tile as isize + offset) as usize][1] as i32 };
                let opposite = 2 - self.colour(row, column);
                let (colour, value) = if opposite == 1 {
                    // A green pixel: the colour across from the row's
                    // neighbours, the one below from the column's.
                    let colour = self.colour(row + 1, column);
                    let across = 2 - colour;
                    let horizontal = at(0, 1) + ((at(-1, across) + at(1, across) - green(rgb, -1) - green(rgb, 1)) >> 1);
                    rgb[tile][across] = clip(horizontal);
                    let vertical = at(0, 1) + ((at(-wide, colour) + at(wide, colour) - green(rgb, -span) - green(rgb, span)) >> 1);
                    (colour, vertical)
                } else {
                    // Red at blue or blue at red: from the diagonals.
                    let diagonals = at(-wide - 1, opposite) + at(-wide + 1, opposite) + at(wide - 1, opposite) + at(wide + 1, opposite);
                    let greens = green(rgb, -span - 1) + green(rgb, -span + 1) + green(rgb, span - 1) + green(rgb, span + 1);
                    (opposite, green(rgb, 0) + ((diagonals - greens + 1) >> 2))
                };
                rgb[tile][colour] = clip(value);
                let own = self.colour(row, column);
                rgb[tile][own] = pixels[index][own];
                lab[tile] = cielab.convert(rgb[tile]);
            }
        }
    }

    /// `ahd_interpolate_build_homogeneity_map`: per pixel and direction,
    /// how many of its four neighbours are within the smaller of the two
    /// directions' largest lightness and chroma differences.
    fn ahd_homogeneity(&self, top: usize, left: usize, lab: &[Vec<[i16; 3]>; 2], homogeneity: &mut [[u8; 2]]) {
        const DIRECTIONS: [isize; 4] = [-1, 1, -(TILE as isize), TILE as isize];
        let row_limit = (top + TILE - 2).min(self.height - 4);
        let column_limit = (left + TILE - 2).min(self.width - 4);
        homogeneity.fill([0, 0]);
        for row in top + 2..row_limit {
            for column in left + 2..column_limit {
                let tile = (row - top) * TILE + column - left;
                let mut lightness = [[0u32; 4]; 2];
                let mut chroma = [[0u32; 4]; 2];
                for direction in 0..2 {
                    let centre = lab[direction][tile];
                    for (neighbour, offset) in DIRECTIONS.iter().enumerate() {
                        let adjacent = lab[direction][(tile as isize + offset) as usize];
                        lightness[direction][neighbour] = (centre[0] as i32 - adjacent[0] as i32).unsigned_abs();
                        let a = centre[1] as i32 - adjacent[1] as i32;
                        let b = centre[2] as i32 - adjacent[2] as i32;
                        chroma[direction][neighbour] = a.wrapping_mul(a).wrapping_add(b.wrapping_mul(b)) as u32;
                    }
                }
                let lightness_epsilon = lightness[0][0].max(lightness[0][1]).min(lightness[1][2].max(lightness[1][3]));
                let chroma_epsilon = chroma[0][0].max(chroma[0][1]).min(chroma[1][2].max(chroma[1][3]));
                for direction in 0..2 {
                    let mut count = 0u8;
                    for neighbour in 0..4 {
                        if lightness[direction][neighbour] <= lightness_epsilon && chroma[direction][neighbour] <= chroma_epsilon {
                            count += 1;
                        }
                    }
                    homogeneity[tile][direction] = count;
                }
            }
        }
    }

    /// `ahd_interpolate_combine_homogeneous_pixels`.
    fn ahd_combine(&mut self, top: usize, left: usize, rgb: &[Vec<[u16; 3]>; 2], homogeneity: &[[u8; 2]]) {
        let width = self.width;
        let row_limit = (top + TILE - 3).min(self.height - 5);
        let column_limit = (left + TILE - 3).min(width - 5);
        for row in top + 3..row_limit {
            for column in left + 3..column_limit {
                let tile = (row - top) * TILE + column - left;
                let mut scores = [0u32; 2];
                for tile_row in tile / TILE - 1..=tile / TILE + 1 {
                    for tile_column in tile % TILE - 1..=tile % TILE + 1 {
                        let cell = homogeneity[tile_row * TILE + tile_column];
                        scores[0] += cell[0] as u32;
                        scores[1] += cell[1] as u32;
                    }
                }
                let pixel = &mut self.pixels[row * width + column];
                if scores[0] != scores[1] {
                    let chosen = rgb[(scores[1] > scores[0]) as usize][tile];
                    pixel[..3].copy_from_slice(&chosen);
                } else {
                    for channel in 0..3 {
                        pixel[channel] = ((rgb[0][tile][channel] as u32 + rgb[1][tile][channel] as u32) >> 1) as u16;
                    }
                }
            }
        }
    }

    /// `convert_to_rgb`: camera colours to sRGB (`rgb_cam`, as the sRGB
    /// output matrix is the identity), counting the histogram
    /// `dcraw_make_mem_image` brightens by.
    fn convert_to_rgb(&mut self) -> Vec<[u32; HISTOGRAM_SIZE]> {
        let mut histogram = vec![[0u32; HISTOGRAM_SIZE]; 4];
        let colors = self.state.colors;
        let raw_color = self.state.raw_color || colors == 1;
        let size = self.width * self.height;
        if raw_color {
            for pixel in &self.pixels[..size] {
                for (channel_histogram, &value) in histogram.iter_mut().zip(pixel).take(colors) {
                    channel_histogram[(value >> 3) as usize] += 1;
                }
            }
        } else {
            let mut matrix = [[0.0f32; 4]; 3];
            for (row, values) in matrix.iter_mut().enumerate() {
                values.copy_from_slice(&self.state.rgb_cam[row]);
                for (column, value) in values.iter_mut().enumerate().take(colors) {
                    let mut sum = 0.0f32;
                    for (index, camera_row) in self.state.rgb_cam.iter().enumerate() {
                        let identity = if index == row { 1.0f64 } else { 0.0 };
                        sum += (identity * camera_row[column] as f64) as f32;
                    }
                    *value = sum;
                }
            }
            for pixel in self.pixels[..size].iter_mut() {
                let input = pixel.map(|value| value as f32);
                let mut output = [0.0f32; 3];
                for (row, value) in output.iter_mut().enumerate() {
                    let mut sum = matrix[row][0] * input[0] + matrix[row][1] * input[1] + matrix[row][2] * input[2];
                    if colors == 4 {
                        sum += matrix[row][3] * input[3];
                    }
                    *value = sum;
                }
                for channel in 0..3 {
                    pixel[channel] = clip(output[channel] as i32);
                    histogram[channel][(pixel[channel] >> 3) as usize] += 1;
                }
                if colors == 4 {
                    histogram[3][(pixel[3] >> 3) as usize] += 1;
                }
            }
        }
        if colors == 4 {
            self.state.colors = 3;
        }
        histogram
    }

    /// `stretch`: non-square pixels (DefaultScale) made square by
    /// repeating rows or columns (LibRaw truncates the blend fraction).
    fn stretch(&mut self) {
        let aspect = self.state.pixel_aspect;
        if aspect == 1.0 {
            return;
        }
        let width = self.width;
        let height = self.height;
        let colors = self.state.colors.min(4);
        if aspect < 1.0 {
            let new_height = (height as f64 / aspect + 0.5) as u16 as usize;
            let mut pixels = vec![[0u16; 4]; width * new_height];
            let mut position = 0.0f64;
            for row in 0..new_height {
                let source = position as i32 as usize;
                for column in 0..width {
                    let pixel = self.pixels[source * width + column];
                    pixels[row * width + column][..colors].copy_from_slice(&pixel[..colors]);
                }
                position += aspect;
            }
            self.pixels = pixels;
            self.height = new_height;
        } else {
            let new_width = (width as f64 * aspect + 0.5) as u16 as usize;
            let mut pixels = vec![[0u16; 4]; height * new_width];
            let mut position = 0.0f64;
            for column in 0..new_width {
                let source = position as i32 as usize;
                for row in 0..height {
                    let pixel = self.pixels[row * width + source];
                    pixels[row * new_width + column][..colors].copy_from_slice(&pixel[..colors]);
                }
                position += 1.0 / aspect;
            }
            self.pixels = pixels;
            self.width = new_width;
        }
    }

    /// `dcraw_make_mem_image` (`copy_mem_image`): the gamma curve with its
    /// white at the level only 1% of the pixels exceed, applied to each
    /// colour.
    fn make_mem_image(self, histogram: &[[u32; HISTOGRAM_SIZE]]) -> Output {
        let colors = self.state.colors;
        let threshold = ((self.width * self.height) as i32 as f32 * 0.01f32) as i32;
        let mut white = 0i32;
        for channel_histogram in histogram.iter().take(colors) {
            let mut total = 0u32;
            let mut value = HISTOGRAM_SIZE as i32;
            loop {
                value -= 1;
                if value <= 32 {
                    break;
                }
                total += channel_histogram[value as usize];
                if total as i32 > threshold {
                    break;
                }
            }
            if white < value {
                white = value;
            }
        }
        let curve = gamma_curve_table(GAMMA_POWER, GAMMA_SLOPE, 2, ((white << 3) as f32 / 1.0f32) as i32);
        let mut samples = Vec::with_capacity(self.width * self.height * colors);
        for pixel in &self.pixels[..self.width * self.height] {
            for &value in pixel.iter().take(colors) {
                samples.push(curve[value as usize]);
            }
        }
        Output { width: self.width, height: self.height, colors, samples }
    }
}

/// `ULIM`: `value` clamped between two bounds in either order.
fn ulim(value: i32, first: i32, second: i32) -> i32 {
    if first < second {
        value.min(second).max(first)
    } else {
        value.min(first).max(second)
    }
}

/// `cielab`'s tables: the cube root (LibRaw's `powf(r, 1/3)` and linear
/// toe) and camera → XYZ (D65-normalised).
pub(super) struct Cielab {
    cbrt: &'static [f32],
    xyz_cam: [[f32; 4]; 3],
    colors: usize,
}

static CUBE_ROOTS: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();

/// LibRaw's `powf(r, 1.f / 3.0f)`, correctly rounded from double
/// precision so it does not depend on the platform's `powf`.
fn cube_root(value: f32) -> f32 {
    (value as f64).powf((1.0f32 / 3.0f32) as f64) as f32
}

impl Cielab {
    pub(super) fn new(state: &Raw) -> Cielab {
        let cbrt = CUBE_ROOTS.get_or_init(|| {
            (0..0x10000)
                .map(|index| {
                    let ratio = index as f32 / 65535.0f32;
                    if ratio > 0.008856f32 {
                        cube_root(ratio)
                    } else {
                        7.787f32 * ratio + 16.0f32 / 116.0f32
                    }
                })
                .collect()
        });
        let colors = state.colors;
        let mut xyz_cam = [[0.0f32; 4]; 3];
        for (row, values) in xyz_cam.iter_mut().enumerate() {
            for (column, value) in values.iter_mut().enumerate().take(colors.min(4)) {
                for (index, camera_row) in state.rgb_cam.iter().enumerate() {
                    *value += (XYZ_RGB[row][index] * camera_row[column] as f64 / D65_WHITE[row] as f64) as f32;
                }
            }
        }
        Cielab { cbrt, xyz_cam, colors }
    }

    /// `cielab`: 64 × L*, a*, b* of a camera RGB pixel.
    pub(super) fn convert(&self, rgb: [u16; 3]) -> [i16; 3] {
        let mut xyz = [0.5f32; 3];
        for (channel, &sample) in rgb.iter().enumerate().take(self.colors) {
            let value = sample as f32;
            xyz[0] += self.xyz_cam[0][channel] * value;
            xyz[1] += self.xyz_cam[1][channel] * value;
            xyz[2] += self.xyz_cam[2][channel] * value;
        }
        let x = self.cbrt[clip(xyz[0] as i32) as usize];
        let y = self.cbrt[clip(xyz[1] as i32) as usize];
        let z = self.cbrt[clip(xyz[2] as i32) as usize];
        [(64.0 * (116.0 * y - 16.0)) as i32 as i16, (32000.0 * (x - y)) as i32 as i16, (12800.0 * (y - z)) as i32 as i16]
    }
}

/// `gamma_curve(pwr, ts, mode, imax)`: `curve` for mode 1 (the inverse
/// curve, linearising) or 2 (the forward curve), white at `imax`.
pub(super) fn gamma_curve_table(power: f64, slope: f64, mode: i32, imax: i32) -> Vec<u16> {
    let g = gamma_parameters(power, slope);
    let forward = mode - 1 != 0;
    let mut curve = vec![0xffffu16; 0x10000];
    for (index, value) in curve.iter_mut().enumerate() {
        let r = index as f64 / imax as f64;
        if r < 1.0 {
            let mapped = if forward {
                if r < g[3] {
                    r * g[1]
                } else if g[0] != 0.0 {
                    r.powf(g[0]) * (1.0 + g[4]) - g[4]
                } else {
                    r.ln() * g[2] + 1.0
                }
            } else if r < g[2] {
                r / g[1]
            } else if g[0] != 0.0 {
                ((r + g[4]) / (1.0 + g[4])).powf(1.0 / g[0])
            } else {
                ((r - 1.0) / g[2]).exp()
            };
            *value = (65536.0 * mapped) as i32 as u16;
        }
    }
    curve
}

/// `gamma_curve`'s parameters: the power and toe slope, the toe's end in
/// output and input, the offset and the curve's area.
pub(super) fn gamma_parameters(power: f64, slope: f64) -> [f64; 6] {
    let mut g = [power, slope, 0.0, 0.0, 0.0, 0.0];
    let mut bounds = [0.0f64; 2];
    bounds[(g[1] >= 1.0) as usize] = 1.0;
    if g[1] != 0.0 && (g[1] - 1.0) * (g[0] - 1.0) <= 0.0 {
        for _ in 0..48 {
            g[2] = (bounds[0] + bounds[1]) / 2.0;
            let index = if g[0] != 0.0 {
                ((g[2] / g[1]).powf(-g[0]) - 1.0) / g[0] - 1.0 / g[2] > -1.0
            } else {
                g[2] / (1.0 - 1.0 / g[2]).exp() < g[1]
            };
            bounds[index as usize] = g[2];
        }
        g[3] = g[2] / g[1];
        if g[0] != 0.0 {
            g[4] = g[2] * (1.0 / g[0] - 1.0);
        }
    }
    g[5] = if g[0] != 0.0 {
        1.0 / (g[1] * (g[3] * g[3]) / 2.0 - g[4] * (1.0 - g[3]) + (1.0 - g[3].powf(1.0 + g[0])) * (1.0 + g[4]) / (1.0 + g[0])) - 1.0
    } else {
        1.0 / (g[1] * (g[3] * g[3]) / 2.0 + 1.0 - g[2] - g[3] - g[2] * g[3] * (g[3].ln() - 1.0)) - 1.0
    };
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The double-precision cube roots equal x86-64 macOS libm's `powf`
    /// (what the oracle's Magick.Native calls) for every table entry.
    /// arm64's `powf` is not correctly rounded everywhere (entry 643 is
    /// one ulp high), hence the double-precision table.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    #[test]
    fn cube_roots_match_powf() {
        for index in 0..0x10000 {
            let ratio = index as f32 / 65535.0f32;
            if ratio > 0.008856f32 {
                assert_eq!(cube_root(ratio).to_bits(), ratio.powf(1.0f32 / 3.0f32).to_bits(), "entry {index}");
            }
        }
    }

    /// `gamma_curve(0.45, 4.5, 0, 0)` as LibRaw leaves it in `gamm`.
    #[test]
    fn bt709_gamma_parameters() {
        let parameters = gamma_parameters(GAMMA_POWER, GAMMA_SLOPE);
        let expected = [0.45, 4.5, 0.08124285829863354, 0.018053968510807452, 0.09929682680944099, 0.5171811042184706];
        for (value, expected) in parameters.iter().zip(expected) {
            assert_eq!(value.to_bits(), f64::to_bits(expected));
        }
    }
}
