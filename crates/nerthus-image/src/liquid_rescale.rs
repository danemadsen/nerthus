//! IM `LiquidRescaleImage`: seam carving with the behaviour of liblqr (the
//! library IM delegates to) for the settings Magick.NET's
//! `LiquidRescale(width, height)` passes, no seam slack (`delta_x` 0) and no
//! rigidity. Without slack every seam is a straight line, so the carver
//! repeatedly drops the column whose summed energy is lowest, then does the
//! same to the rows of what is left.

use rayon::prelude::*;

use crate::resize::resize;
use crate::Image;

/// IM's `QuantumRange`: IM hands liblqr `QuantumScale * quantum` as floats.
const QUANTUM_RANGE: f32 = 65535.0;
/// liblqr keeps a cumulative energy when an update moves it by less.
const UPDATE_TOLERANCE: f64 = 1e-5;
/// liblqr's starting value for the lowest seam energy (`1 << 29`).
const SEAM_ENERGY_LIMIT: f32 = 536_870_912.0;

/// `MagickImage.LiquidRescale(width, height)`. Only reductions are carved:
/// a side asked to grow keeps its size.
pub fn liquid_rescale(image: &Image, width: usize, height: usize) -> Image {
    carve(image, width, height, QUANTUM_RANGE)
}

/// [`liquid_rescale`] for an image whose values times `to_quantum` are its
/// quanta (65535 for normalised images, 1 for images holding quanta).
pub(crate) fn carve(image: &Image, width: usize, height: usize, to_quantum: f32) -> Image {
    if width == image.width && height == image.height {
        return image.clone();
    }
    if width <= 2 || height <= 2 {
        return resize(image, width, height);
    }

    let brightness = carver_brightness(image, to_quantum);

    // Columns first, from the brightness one column after another.
    let mut by_column = vec![0.0f64; brightness.len()];
    for (y, row) in brightness.chunks_exact(image.width).enumerate() {
        for (x, &value) in row.iter().enumerate() {
            by_column[x * image.height + y] = value;
        }
    }
    let columns = carve_lines(&by_column, image.width, image.height, width);

    // liblqr then flattens the narrowed image and transposes it, so its rows
    // are carved the same way; one row after another is the narrowed image
    // in reading order.
    let narrowed_width = columns.len();
    let mut by_row = Vec::with_capacity(narrowed_width * image.height);
    for row in brightness.chunks_exact(image.width) {
        for &column in &columns {
            by_row.push(row[column]);
        }
    }
    let rows = carve_lines(&by_row, image.height, narrowed_width, height);

    let channels = image.channels;
    let mut result = image.new_like(narrowed_width, rows.len());
    for (y, &row) in rows.iter().enumerate() {
        for (x, &column) in columns.iter().enumerate() {
            let source = (row * image.width + column) * channels;
            let target = (y * narrowed_width + x) * channels;
            let pixel = &image.data[source..source + channels];
            result.data[target..target + channels].copy_from_slice(pixel);
        }
    }
    result
}

/// The brightness liblqr reads for each pixel (`LQR_ER_BRIGHTNESS`): grey,
/// or the mean of red, green and blue, times alpha when there is one.
fn carver_brightness(image: &Image, to_quantum: f32) -> Vec<f64> {
    let channels = image.channels;
    image
        .data
        .par_chunks_exact(channels)
        .map(|pixel| {
            let carver_value = |channel: usize| -> f64 {
                let quantum = pixel[channel] * to_quantum;
                let value = (1.0 / QUANTUM_RANGE as f64) * quantum as f64;
                value as f32 as f64
            };
            match channels {
                1 => carver_value(0),
                2 => carver_value(0) * carver_value(1),
                3 => (carver_value(0) + carver_value(1) + carver_value(2)) / 3.0,
                _ => (carver_value(0) + carver_value(1) + carver_value(2)) / 3.0 * carver_value(3),
            }
        })
        .collect()
}

/// Carves `count - target` straight seams from `map`, which holds `count`
/// lines of `length` brightness values each, and returns the lines that
/// remain in order.
fn carve_lines(map: &[f64], count: usize, length: usize, target: usize) -> Vec<usize> {
    let mut lines: Vec<usize> = (0..count).collect();
    if target >= count {
        return lines;
    }

    // Energies and their running sums move with their line as seams are
    // removed, so both are kept per original line.
    let mut energy = vec![0.0f32; count * length];
    let mut cumulative = vec![0.0f32; count * length];
    energy
        .par_chunks_mut(length)
        .zip(cumulative.par_chunks_mut(length))
        .enumerate()
        .for_each(|(position, (line_energy, line_cumulative))| {
            fill_energy(map, length, &lines, position, line_energy);
            line_cumulative[0] = line_energy[0];
            for index in 1..length {
                line_cumulative[index] = line_energy[index] + line_cumulative[index - 1];
            }
        });

    let seams = count - target;
    for seam in 0..seams {
        let mut lowest = SEAM_ENERGY_LIMIT;
        let mut chosen = 0;
        for (position, &line) in lines.iter().enumerate() {
            let total = cumulative[line * length + length - 1];
            if total < lowest {
                lowest = total;
                chosen = position;
            }
        }
        lines.remove(chosen);
        if seam + 1 == seams {
            break;
        }

        // Only the two lines that met across the seam see new neighbours.
        let first = chosen.saturating_sub(1);
        let last = chosen.min(lines.len() - 1);
        for position in first..=last {
            let line = lines[position];
            let line_energy = &mut energy[line * length..(line + 1) * length];
            fill_energy(map, length, &lines, position, line_energy);
            let line_cumulative = &mut cumulative[line * length..(line + 1) * length];
            update_cumulative(line_energy, line_cumulative);
        }
    }
    lines
}

/// liblqr's default energy (`LQR_EF_GRAD_XABS`) along the line at
/// `position`: the absolute brightness gradient across the lines, central
/// inside and one-sided at either end.
fn fill_energy(map: &[f64], length: usize, lines: &[usize], position: usize, energy: &mut [f32]) {
    let last = lines.len() - 1;
    let (before, after, scale) = if position == 0 {
        (lines[0], lines[1], 1.0)
    } else if position < last {
        (lines[position - 1], lines[position + 1], 0.5)
    } else {
        (lines[position - 1], lines[position], 1.0)
    };
    let before_values = &map[before * length..(before + 1) * length];
    let after_values = &map[after * length..(after + 1) * length];
    for (index, value) in energy.iter_mut().enumerate() {
        let gradient = (after_values[index] - before_values[index]) * scale;
        *value = gradient.abs() as f32;
    }
}

/// liblqr's incremental update of the running energy sums: a sum that would
/// move by less than [`UPDATE_TOLERANCE`] keeps its old value.
fn update_cumulative(energy: &[f32], cumulative: &mut [f32]) {
    cumulative[0] = energy[0];
    for index in 1..energy.len() {
        let updated = energy[index] + cumulative[index - 1];
        let change = (cumulative[index] - updated).abs() as f64;
        if change < UPDATE_TOLERANCE {
            continue;
        }
        cumulative[index] = updated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_columns_are_carved_first() {
        // A ramp in the middle columns and flat ground either side: the
        // carver keeps the ramp's edges and drops flat columns.
        let width = 8;
        let height = 4;
        let mut image = Image::new(width, height, 1);
        for y in 0..height {
            for x in 0..width {
                let value = if x < 3 { 0.0 } else if x < 5 { 0.5 } else { 1.0 };
                image.set(x, y, 0, value);
            }
        }
        let carved = liquid_rescale(&image, 4, height);
        assert_eq!((carved.width, carved.height), (4, height));
        let first_row: Vec<f32> = (0..4).map(|x| carved.get(x, 0, 0)).collect();
        assert_eq!(first_row, vec![0.0, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn identity_size_is_a_copy() {
        let image = Image::filled(5, 5, 1, 0.25);
        assert_eq!(liquid_rescale(&image, 5, 5), image);
    }
}
