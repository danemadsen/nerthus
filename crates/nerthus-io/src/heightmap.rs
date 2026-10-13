//! Heightmap and colour map export/import with the build's encodings.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use nerthus_core::Field;

/// Export formats in order; the index is the node's `SaveFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Exr32,
    Tiff32,
    Tiff16,
    Png16,
    Png8,
    Psd16,
    Raw16,
    R16,
    R32,
    Pfm,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 10] = [
        ExportFormat::Exr32,
        ExportFormat::Tiff32,
        ExportFormat::Tiff16,
        ExportFormat::Png16,
        ExportFormat::Png8,
        ExportFormat::Psd16,
        ExportFormat::Raw16,
        ExportFormat::R16,
        ExportFormat::R32,
        ExportFormat::Pfm,
    ];

    /// The format a `SaveFormat` selects: negative values mean "default" which is TIFF (32-bit).
    pub fn from_save_format(save_format: i32) -> Self {
        if save_format < 0 {
            return ExportFormat::Tiff32;
        }
        ExportFormat::ALL.get(save_format as usize).copied().unwrap_or(ExportFormat::Tiff32)
    }

    pub fn save_format(self) -> i32 {
        ExportFormat::ALL.iter().position(|format| *format == self).unwrap_or(1) as i32
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Exr32 => "OpenEXR (32-bit)",
            ExportFormat::Tiff32 => "TIFF (32-bit)",
            ExportFormat::Tiff16 => "TIFF (16-bit)",
            ExportFormat::Png16 => "PNG (16-bit)",
            ExportFormat::Png8 => "PNG (8-bit)",
            ExportFormat::Psd16 => "Photoshop (16-bit)",
            ExportFormat::Raw16 => "RAW (16-bit ushort)",
            ExportFormat::R16 => "R16 (16-bit ushort)",
            ExportFormat::R32 => "RAW (32-bit float)",
            ExportFormat::Pfm => "PFM (32-bit float)",
        }
    }

    /// The format a file name's extension selects (the writer switches on
    /// the extension, including the `.tif16` and `.png8` pseudo
    /// extensions; Magick writes `.tiff` at its 16-bit default depth).
    pub fn from_extension(extension: &str) -> Option<ExportFormat> {
        match extension.to_ascii_lowercase().as_str() {
            "exr" => Some(ExportFormat::Exr32),
            "tif" => Some(ExportFormat::Tiff32),
            "tif16" | "tiff" => Some(ExportFormat::Tiff16),
            "png" => Some(ExportFormat::Png16),
            "png8" => Some(ExportFormat::Png8),
            "psd" => Some(ExportFormat::Psd16),
            "raw" => Some(ExportFormat::Raw16),
            "r16" => Some(ExportFormat::R16),
            "r32" => Some(ExportFormat::R32),
            "pfm" => Some(ExportFormat::Pfm),
            _ => None,
        }
    }

    /// File extension written to disk.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Exr32 => "exr",
            ExportFormat::Tiff32 => "tif",
            ExportFormat::Tiff16 => "tiff",
            ExportFormat::Png16 | ExportFormat::Png8 => "png",
            ExportFormat::Psd16 => "psd",
            ExportFormat::Raw16 => "raw",
            ExportFormat::R16 => "r16",
            ExportFormat::R32 => "r32",
            ExportFormat::Pfm => "pfm",
        }
    }
}

/// How heights are mapped before writing (`Build.Range.Type`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeMode {
    /// Values as-is.
    Normalized,
    /// Multiplied by height / scale (type 1).
    WorldScale { height: f32, scale: f32 },
    /// Auto-level (type 2).
    AutoLevel,
    /// Linear remap to [min, max] (type 3).
    Custom { min: f32, max: f32 },
}

impl RangeMode {
    pub fn from_build(range_type: i32, min: f64, max: f64, height: f32, scale: f32) -> Self {
        match range_type {
            1 => RangeMode::WorldScale { height, scale },
            2 => RangeMode::AutoLevel,
            3 => RangeMode::Custom { min: min as f32, max: max as f32 },
            _ => RangeMode::Normalized,
        }
    }

    pub fn apply(self, data: &mut Vec<f32>, size: usize) {
        match self {
            RangeMode::Normalized => {}
            RangeMode::WorldScale { height, scale } => {
                let factor = height / scale;
                for value in data.iter_mut() {
                    *value *= factor;
                }
            }
            RangeMode::AutoLevel => {
                *data = strided_auto_level(data, size);
            }
            RangeMode::Custom { min, max } => {
                let span = max - min;
                for value in data.iter_mut() {
                    *value = *value * span + min;
                }
            }
        }
    }
}

/// Auto-levels with the min/max of every eighth row.
fn strided_auto_level(data: &[f32], size: usize) -> Vec<f32> {
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    let mut row = 0;
    while row < size {
        for &value in &data[row * size..(row + 1) * size] {
            min = min.min(value);
            max = max.max(value);
        }
        row += 8;
    }
    let scale = 1.0 / (max - min);
    data.iter().map(|value| (value - min) * scale).collect()
}

fn quantize_16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

fn quantize_8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Writes a field (grey or colour) in `format`. `range` only applies to grey
/// data; `path` gets the format's extension appended unless it has it.
pub fn export_field(field: &Field, path: &Path, format: ExportFormat, range: RangeMode, flip_y: bool) -> Result<PathBuf> {
    let size = field.size;
    let channels = field.channels;
    let mut data = field.data.clone();
    if channels == 1 {
        range.apply(&mut data, size);
    }
    if flip_y {
        flip_rows(&mut data, size, channels);
    }
    let path = format_path(path, format);
    write_samples(&path, &data, size, channels, format)?;
    Ok(path)
}

/// `path` with `format`'s extension appended (names may contain dots, so
/// an existing extension is only kept when it already is the format's).
pub fn format_path(path: &Path, format: ExportFormat) -> PathBuf {
    let extension = format.extension();
    let current = path.extension().and_then(|value| value.to_str()).unwrap_or("");
    if current.eq_ignore_ascii_case(extension) {
        return path.to_path_buf();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Encodes interleaved samples to `path`. Grey + alpha and RGBA
/// keep their alpha where the format has one.
pub fn write_samples(path: &Path, data: &[f32], size: usize, channels: usize, format: ExportFormat) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match format {
        ExportFormat::Png16 => write_png16(path, data, size, channels),
        ExportFormat::Png8 => write_png8(path, data, size, channels),
        ExportFormat::Tiff32 if channels == 1 => write_tiff32(path, data, size),
        ExportFormat::Tiff32 | ExportFormat::Tiff16 => write_tiff16(path, data, size, channels),
        ExportFormat::Exr32 => write_exr(path, data, size, channels),
        ExportFormat::Raw16 | ExportFormat::R16 => write_raw16(path, &to_gray(data, channels), size),
        ExportFormat::R32 => write_r32(path, &to_gray(data, channels), size),
        ExportFormat::Pfm => write_pfm(path, data, size, channels),
        ExportFormat::Psd16 => write_psd16(path, data, size, channels),
    }
}

/// Writes 8-bit samples in whatever format `path`'s extension names (the
/// `MagickImage.Write(path)` fallback for extensions with no case of their
/// own).
pub fn write_image_file(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let samples: Vec<u8> = data.iter().map(|&value| quantize_8(value)).collect();
    let side = size as u32;
    let color = match channels {
        1 => image::ColorType::L8,
        2 => image::ColorType::La8,
        3 => image::ColorType::Rgb8,
        _ => image::ColorType::Rgba8,
    };
    image::save_buffer(path, &samples, side, side, color).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn to_gray(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return data.to_vec();
    }
    if channels == 2 {
        return data.chunks(2).map(|pixel| pixel[0]).collect();
    }
    data.chunks(channels).map(|pixel| 0.212_656 * pixel[0] + 0.715_158 * pixel[1] + 0.072_186 * pixel[2]).collect()
}

fn flip_rows(data: &mut [f32], size: usize, channels: usize) {
    let row = size * channels;
    for y in 0..size / 2 {
        let (top, bottom) = data.split_at_mut((size - 1 - y) * row);
        top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
    }
}

fn write_png16(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    let samples: Vec<u16> = data.iter().map(|&value| quantize_16(value)).collect();
    let side = size as u32;
    match channels {
        1 => {
            let image = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        2 => {
            let image = image::ImageBuffer::<image::LumaA<u16>, Vec<u16>>::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        3 => {
            let image = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        _ => {
            let image = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
    }
    Ok(())
}

fn write_png8(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    let samples: Vec<u8> = data.iter().map(|&value| quantize_8(value)).collect();
    let side = size as u32;
    match channels {
        1 => {
            let image = image::GrayImage::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        2 => {
            let image = image::GrayAlphaImage::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        3 => {
            let image = image::RgbImage::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
        _ => {
            let image = image::RgbaImage::from_raw(side, side, samples).context("buffer size")?;
            image.save(path)?;
        }
    }
    Ok(())
}

fn write_tiff32(path: &Path, data: &[f32], size: usize) -> Result<()> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = tiff::encoder::TiffEncoder::new(file)?;
    encoder.write_image::<tiff::encoder::colortype::Gray32Float>(size as u32, size as u32, data)?;
    Ok(())
}

fn write_tiff16(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = tiff::encoder::TiffEncoder::new(file)?;
    let samples: Vec<u16> = data.iter().map(|&value| quantize_16(value)).collect();
    let side = size as u32;
    match channels {
        1 => encoder.write_image::<tiff::encoder::colortype::Gray16>(side, side, &samples)?,
        2 => {
            let gray: Vec<u16> = samples.chunks(2).map(|pixel| pixel[0]).collect();
            encoder.write_image::<tiff::encoder::colortype::Gray16>(side, side, &gray)?;
        }
        3 => encoder.write_image::<tiff::encoder::colortype::RGB16>(side, side, &samples)?,
        _ => encoder.write_image::<tiff::encoder::colortype::RGBA16>(side, side, &samples)?,
    }
    Ok(())
}

fn write_exr(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    use exr::prelude::*;
    let channel_names: &[&str] = match channels {
        1 => &["Y"],
        2 => &["Y", "A"],
        3 => &["R", "G", "B"],
        _ => &["R", "G", "B", "A"],
    };
    let mut channel_list = smallvec::SmallVec::new();
    for (index, name) in channel_names.iter().enumerate() {
        let samples: Vec<f32> = data.chunks(channels).map(|pixel| pixel[index]).collect();
        channel_list.push(AnyChannel::new(*name, FlatSamples::F32(samples)));
    }
    let channels_sorted = AnyChannels::sort(channel_list);
    let layer = Layer::new(Vec2(size, size), LayerAttributes::default(), Encoding::UNCOMPRESSED, channels_sorted);
    let image = Image::from_layer(layer);
    image.write().to_file(path)?;
    Ok(())
}

/// RAW: little-endian ushort, `(ushort)(clamp01(v) * 65535)` (truncated).
fn write_raw16(path: &Path, data: &[f32], size: usize) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    for &value in data.iter().take(size * size) {
        let sample = (value.clamp(0.0, 1.0) * 65535.0) as u16;
        writer.write_all(&sample.to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

fn write_r32(path: &Path, data: &[f32], size: usize) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    for &value in data.iter().take(size * size) {
        writer.write_all(&value.to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

fn write_pfm(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    let gray = channels <= 2;
    let header = if gray { "Pf" } else { "PF" };
    write!(writer, "{header}\n{size} {size}\n-1.0\n")?;
    // PFM stores rows bottom to top.
    for y in (0..size).rev() {
        for x in 0..size {
            let index = (y * size + x) * channels;
            let colors = if gray { 1 } else { 3 };
            for channel in 0..colors {
                writer.write_all(&data[index + channel].to_le_bytes())?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

/// Minimal flattened 16-bit Photoshop document (alpha as an extra channel).
fn write_psd16(path: &Path, data: &[f32], size: usize, channels: usize) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    let gray = channels <= 2;
    let channel_count = channels as u16;
    writer.write_all(b"8BPS")?;
    writer.write_all(&1u16.to_be_bytes())?;
    writer.write_all(&[0u8; 6])?;
    writer.write_all(&channel_count.to_be_bytes())?;
    writer.write_all(&(size as u32).to_be_bytes())?;
    writer.write_all(&(size as u32).to_be_bytes())?;
    writer.write_all(&16u16.to_be_bytes())?;
    let color_mode: u16 = if gray { 1 } else { 3 };
    writer.write_all(&color_mode.to_be_bytes())?;
    writer.write_all(&0u32.to_be_bytes())?;
    writer.write_all(&0u32.to_be_bytes())?;
    writer.write_all(&0u32.to_be_bytes())?;
    writer.write_all(&0u16.to_be_bytes())?;
    for channel in 0..channels {
        for index in 0..size * size {
            let value = data[index * channels + channel];
            writer.write_all(&quantize_16(value).to_be_bytes())?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Reads an image or raw heightmap into a square field of `size` (grey).
pub fn import_heightmap(path: &Path, size: usize) -> Result<Field> {
    let (data, side) = read_gray(path)?;
    let data = if side == size { data } else { nerthus_image::resize::resize_gray(&data, size) };
    Ok(Field::from_data(size, 1, data))
}

/// Reads an image into RGB at `size`.
pub fn import_color(path: &Path, size: usize) -> Result<Field> {
    let decoded = image::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let rgb = decoded.to_rgb32f();
    let (width, height) = rgb.dimensions();
    let square = width.min(height) as usize;
    let mut data = Vec::with_capacity(square * square * 3);
    for y in 0..square {
        for x in 0..square {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            data.extend_from_slice(&pixel.0);
        }
    }
    let data = if square == size { data } else { nerthus_image::resize::resize_rgb(&data, size) };
    Ok(Field::from_data(size, 3, data))
}

/// Reads any supported heightmap file as a square grey buffer.
pub fn read_gray(path: &Path) -> Result<(Vec<f32>, usize)> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "r32" => read_r32(path),
        "raw" | "r16" => read_raw16(path),
        "exr" => read_exr(path),
        "pfm" => read_pfm(path),
        _ => read_image_gray(path),
    }
}

fn read_r32(path: &Path) -> Result<(Vec<f32>, usize)> {
    let bytes = std::fs::read(path)?;
    let count = bytes.len() / 4;
    let side = (count as f64).sqrt() as usize;
    let (words, _) = bytes.as_chunks::<4>();
    let data: Vec<f32> = words.iter().take(side * side).map(|word| f32::from_le_bytes(*word)).collect();
    Ok((data, side))
}

fn read_raw16(path: &Path) -> Result<(Vec<f32>, usize)> {
    let bytes = std::fs::read(path)?;
    let count = bytes.len() / 2;
    let side = (count as f64).sqrt() as usize;
    let (pairs, _) = bytes.as_chunks::<2>();
    let data: Vec<f32> = pairs.iter().take(side * side).map(|pair| u16::from_le_bytes(*pair) as f32 / 65535.0).collect();
    Ok((data, side))
}

fn read_exr(path: &Path) -> Result<(Vec<f32>, usize)> {
    use exr::prelude::*;
    let image = read_first_flat_layer_from_file(path)?;
    let layer = image.layer_data;
    let width = layer.size.0;
    let height = layer.size.1;
    let side = width.min(height);
    let channel = layer
        .channel_data
        .list
        .iter()
        .find(|channel| matches!(channel.name.to_string().as_str(), "Y" | "R" | "G" | "B" | "y"))
        .or_else(|| layer.channel_data.list.first())
        .context("EXR has no channels")?;
    let samples: Vec<f32> = channel.sample_data.values_as_f32().collect();
    let mut data = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            data.push(samples[y * width + x]);
        }
    }
    Ok((data, side))
}

fn read_pfm(path: &Path) -> Result<(Vec<f32>, usize)> {
    let bytes = std::fs::read(path)?;
    let mut fields = Vec::new();
    let mut cursor = 0;
    while fields.len() < 4 && cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..cursor]).to_string());
    }
    cursor += 1;
    if fields.len() < 4 {
        bail!("invalid PFM header");
    }
    let channels = if fields[0] == "PF" { 3 } else { 1 };
    let width: usize = fields[1].parse()?;
    let height: usize = fields[2].parse()?;
    let scale: f32 = fields[3].parse()?;
    let little_endian = scale < 0.0;
    let side = width.min(height);
    let mut data = vec![0.0f32; side * side];
    for row in 0..height {
        let y = height - 1 - row;
        for x in 0..width {
            let offset = cursor + ((row * width + x) * channels) * 4;
            if offset + 4 > bytes.len() {
                break;
            }
            let raw = [bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]];
            let value = if little_endian { f32::from_le_bytes(raw) } else { f32::from_be_bytes(raw) };
            if x < side && y < side {
                data[y * side + x] = value;
            }
        }
    }
    Ok((data, side))
}

fn read_image_gray(path: &Path) -> Result<(Vec<f32>, usize)> {
    let decoded = image::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let gray = decoded.to_luma32f();
    let (width, height) = gray.dimensions();
    let side = width.min(height) as usize;
    let mut data = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            data.push(gray.get_pixel(x as u32, y as u32).0[0]);
        }
    }
    Ok((data, side))
}
