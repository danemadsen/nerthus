//! The 5x5 smoothing kernel the erosion pyramid resamples with, and the
//! down/up-sampling operators built on it (`LandscapeLayer` methods in the
//! CE engine).
//!
//! Every sum is written in the order the compiled code accumulates it, so
//! results match bit for bit.

/// Kernel weights, `K[5 * i + j]`.
pub const KERNEL: [f32; 25] = [
    0.003765, 0.015019, 0.023792, 0.015019, 0.003765, //
    0.015019, 0.059912, 0.094907, 0.059912, 0.015019, //
    0.023792, 0.094907, 0.150342, 0.094907, 0.023792, //
    0.015019, 0.059912, 0.094907, 0.059912, 0.015019, //
    0.003765, 0.015019, 0.023792, 0.015019, 0.003765,
];

/// `Erosion::Erosion` sums the table row by row into a global
/// (`sum = row[0] + sum + row[1] + row[2] + row[3] + row[4]`).
pub fn kernel_sum() -> f32 {
    let mut sum = 0.0f32;
    for row in KERNEL.chunks(5) {
        sum = row[0] + sum + row[1] + row[2] + row[3] + row[4];
    }
    sum
}

/// Clamped index into a `size`-wide map.
#[inline]
fn clamp(value: i32, size: i32) -> i32 {
    let mut clamped = if value < 0 { 0 } else { value };
    if clamped > size - 1 {
        clamped = size - 1;
    }
    clamped
}

/// A square float map (`BitMap<float>`).
#[derive(Clone, Debug)]
pub struct FloatMap {
    pub size: i32,
    pub data: Vec<f32>,
}

impl FloatMap {
    pub fn new(size: i32, fill: f32) -> Self {
        let count = (size * size).max(0) as usize;
        Self { size, data: vec![fill; count] }
    }

    #[inline]
    pub fn at(&self, x: i32, y: i32) -> f32 {
        self.data[(y * self.size + x) as usize]
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y * self.size + x) as usize
    }
}

/// Downsampled value of coarse pixel (`x`, `y`) from the fine map: the
/// kernel centred on fine pixel (2x, 2y), divided by the kernel sum.
pub fn downsample(fine: &FloatMap, x: i32, y: i32, sum: f32) -> f32 {
    let size = fine.size;
    let k = &KERNEL;
    let column = |offset: i32| clamp(2 * x + offset, size);
    let row = |offset: i32| clamp(2 * y + offset, size);
    let at = |dx: i32, dy: i32| fine.at(column(dx), row(dy));
    // k[5 * i + j] weights fine pixel (column 2x - 2 + i, row 2y - 2 + j).
    let total = k[0] * at(-2, -2)
        + k[5] * at(-1, -2)
        + k[10] * at(0, -2)
        + k[15] * at(1, -2)
        + k[20] * at(2, -2)
        + k[1] * at(-2, -1)
        + k[6] * at(-1, -1)
        + k[11] * at(0, -1)
        + k[16] * at(1, -1)
        + k[21] * at(2, -1)
        + k[2] * at(-2, 0)
        + k[7] * at(-1, 0)
        + k[12] * at(0, 0)
        + k[17] * at(1, 0)
        + k[22] * at(2, 0)
        + k[3] * at(-2, 1)
        + k[8] * at(-1, 1)
        + k[13] * at(0, 1)
        + k[18] * at(1, 1)
        + k[23] * at(2, 1)
        + k[4] * at(-2, 2)
        + k[9] * at(-1, 2)
        + k[14] * at(0, 2)
        + k[19] * at(1, 2)
        + k[24] * at(2, 2);
    total / sum
}

/// Coarse pixel reader for the upsamplers: offsets are relative to
/// (`x >> 1`, `y >> 1`) and clamped to the coarse map.
struct CoarseTaps<'a> {
    map: &'a FloatMap,
    x: i32,
    y: i32,
}

impl CoarseTaps<'_> {
    #[inline]
    fn at(&self, dx: i32, dy: i32) -> f32 {
        let size = self.map.size;
        self.map.at(clamp(self.x + dx, size), clamp(self.y + dy, size))
    }
}

/// The adjoint of the downsampler at fine pixel (`x`, `y`) of the parent
/// layer. It is compiled four times — for heights, deposits (which also
/// reports an unused weight), loose sediment and wear — with the same
/// accumulation order.
pub fn upsample(coarse: &FloatMap, x: u32, y: u32) -> f32 {
    let k = &KERNEL;
    let taps = CoarseTaps { map: coarse, x: (x as i32) >> 1, y: (y as i32) >> 1 };
    let at = |dx: i32, dy: i32| taps.at(dx, dy);
    match (x & 1, y & 1) {
        (0, 0) => {
            let total = k[0] * at(-1, -1) + 0.0
                + k[10] * at(0, -1)
                + k[20] * at(1, -1)
                + k[2] * at(-1, 0)
                + k[12] * at(0, 0)
                + k[22] * at(1, 0)
                + k[4] * at(-1, 1)
                + k[14] * at(0, 1)
                + k[24] * at(1, 1);
            total / (k[10] * 4.0 + k[0] * 4.0 + k[12])
        }
        (0, _) => {
            let total = k[1] * at(-1, 0) + 0.0 + k[11] * at(0, 0) + k[21] * at(1, 0) + k[3] * at(-1, 1) + k[13] * at(0, 1) + k[23] * at(1, 1);
            total / (k[11] * 2.0 + k[1] * 4.0)
        }
        (_, 0) => {
            let total = k[5] * at(0, -1) + 0.0 + k[15] * at(1, -1) + k[7] * at(0, 0) + k[17] * at(1, 0) + k[9] * at(0, 1) + k[19] * at(1, 1);
            total / (k[7] * 2.0 + k[5] * 4.0)
        }
        _ => {
            let total = k[6] * at(0, 0) + 0.0 + k[16] * at(1, 0) + k[8] * at(0, 1) + k[18] * at(1, 1);
            total / (k[6] * 4.0)
        }
    }
}
