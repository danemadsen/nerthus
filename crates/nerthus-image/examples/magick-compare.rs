//! Runs one nerthus-image operation on a raw single-precision grey image so
//! the result can be compared with the `magick` command line tool.
//!
//! Usage: `magick-compare <operation> <width> <height> <input.raw> <output.raw>
//! <virtual-pixel> [arguments...]`, where the raw files hold little-endian
//! `f32` values normalised to 0..1 (IM's `-define quantum:format=floating-point`).

use std::env;
use std::fs;
use std::process::ExitCode;

use nerthus_image::distort::{color_gradient_image, distort_polar, distort_shepards, distort_srt, implode, linear_gradient_image, radial_gradient_image, swirl};
use nerthus_image::filters::{despeckle, median_filter, nonpeak_filter, wavelet_denoise};
use nerthus_image::levels::{auto_gamma, auto_level, brightness_contrast, contrast, enhance, equalize, gamma, modulate};
use nerthus_image::liquid_rescale::liquid_rescale;
use nerthus_image::misc::{add_noise, clut, spread, MagickRandom};
use nerthus_image::morphology::{morphology, parse_kernel};
use nerthus_image::resize::{adaptive_resize, resize};
use nerthus_image::{Filter, Image, Interpolate, MorphologyMethod, NoiseType, VirtualPixel};

fn read_raw(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect("input file should be readable");
    let mut values = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.as_chunks::<4>().0 {
        let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
        values.push(f32::from_le_bytes(raw));
    }
    values
}

fn write_raw(path: &str, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output file should be writable");
}

fn parse_virtual_pixel(name: &str) -> VirtualPixel {
    match name {
        "mirror" => VirtualPixel::Mirror,
        "tile" => VirtualPixel::Tile,
        "black" => VirtualPixel::Black,
        "white" => VirtualPixel::White,
        "transparent" => VirtualPixel::Transparent,
        _ => VirtualPixel::Edge,
    }
}

fn parse_interpolate(name: &str) -> Interpolate {
    match name {
        "nearest" => Interpolate::Nearest,
        "integer" => Interpolate::Integer,
        "catrom" => Interpolate::Catrom,
        "mesh" => Interpolate::Mesh,
        "spline" => Interpolate::Spline,
        "average" => Interpolate::Average,
        _ => Interpolate::Bilinear,
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() < 6 {
        eprintln!("usage: magick-compare <operation> <width> <height> <input.raw> <output.raw> <virtual-pixel> [arguments...]");
        return ExitCode::FAILURE;
    }
    let operation = arguments[0].as_str();
    let width: usize = arguments[1].parse().expect("width should be an integer");
    let height: usize = arguments[2].parse().expect("height should be an integer");
    let input = read_raw(&arguments[3]);
    // Grey or RGB input, told apart by the file size.
    let channels = input.len() / (width * height);
    let mut image = Image::from_data(width, height, channels, input);
    image.virtual_pixel = parse_virtual_pixel(&arguments[5]);
    let extra = &arguments[6..];
    let numbers: Vec<f64> = extra.iter().filter_map(|value| value.parse().ok()).collect();

    let result = match operation {
        "srt" => distort_srt(&image, &numbers),
        "resize-catrom" => {
            image.filter = Some(Filter::Catrom);
            Ok(resize(&image, numbers[0] as usize, numbers[1] as usize))
        }
        "srt-catrom" => {
            image.filter = Some(Filter::Catrom);
            distort_srt(&image, &numbers)
        }
        "srt-point" => {
            image.filter = Some(Filter::Point);
            distort_srt(&image, &numbers)
        }
        "polar" => distort_polar(&image, &numbers),
        "shepards" => {
            // First argument is the shepards:power artifact (0 = unset), the
            // second the interpolation method name, then the control points.
            let power = if numbers[0] == 0.0 { None } else { Some(numbers[0]) };
            image.interpolate = parse_interpolate(&extra[1]);
            let points: Vec<f64> = extra[2..].iter().map(|value| value.parse().expect("control points should be numbers")).collect();
            distort_shepards(&image, &points, power)
        }
        "swirl" => Ok(swirl(&image, numbers[0], Interpolate::Bilinear)),
        "gradient" => Ok(linear_gradient_image(width, height)),
        "color-gradient" => {
            let start = [numbers[0], numbers[1], numbers[2]];
            let stop = [numbers[3], numbers[4], numbers[5]];
            Ok(color_gradient_image(width, height, start, stop))
        }
        "clut" => {
            // Arguments: table width, start r g b, stop r g b (0..1), interpolation.
            let table_width: usize = extra[0].parse().expect("table width");
            let start = [numbers[1], numbers[2], numbers[3]];
            let stop = [numbers[4], numbers[5], numbers[6]];
            let table = color_gradient_image(table_width, 1, start, stop);
            clut(&mut image, &table, parse_interpolate(&extra[7]));
            Ok(image)
        }
        "spread" => {
            // Arguments: radius, interpolation, LP64 secret key.
            let radius: f64 = extra[0].parse().expect("radius");
            let key: u64 = extra[2].parse().expect("seed");
            let mut random = MagickRandom::with_secret_key_lp64(key);
            image.interpolate = parse_interpolate(&extra[1]);
            Ok(spread(&image, parse_interpolate(&extra[1]), radius, &mut random))
        }
        "morphology" => {
            // Arguments: method, kernel string, iterations.
            let method = match extra[0].to_ascii_lowercase().as_str() {
                "erode" => MorphologyMethod::Erode,
                "open" => MorphologyMethod::Open,
                "close" => MorphologyMethod::Close,
                "smooth" => MorphologyMethod::Smooth,
                "tophat" => MorphologyMethod::TopHat,
                "bottomhat" => MorphologyMethod::BottomHat,
                "edge" => MorphologyMethod::Edge,
                "edgein" => MorphologyMethod::EdgeIn,
                "edgeout" => MorphologyMethod::EdgeOut,
                "hitandmiss" => MorphologyMethod::HitAndMiss,
                "thinning" => MorphologyMethod::Thinning,
                "thicken" => MorphologyMethod::Thicken,
                "distance" => MorphologyMethod::Distance,
                _ => MorphologyMethod::Dilate,
            };
            let kernels = parse_kernel(&extra[1]).expect("kernel should parse");
            let iterations: isize = extra[2].parse().expect("iterations should be an integer");
            Ok(morphology(&image, method, &kernels, iterations))
        }
        "radial-gradient" => Ok(radial_gradient_image(width, height)),
        "modulate" => {
            modulate(&mut image, numbers[0], numbers[1], numbers[2]);
            Ok(image)
        }
        "contrast" => {
            contrast(&mut image, numbers[0] != 0.0);
            Ok(image)
        }
        "brightness-contrast" => {
            brightness_contrast(&mut image, numbers[0], numbers[1]);
            Ok(image)
        }
        "enhance" => Ok(enhance(&image)),
        "despeckle" => Ok(despeckle(&image)),
        "median" => Ok(median_filter(&image, numbers[0] as usize, numbers[1] as usize)),
        "nonpeak" => Ok(nonpeak_filter(&image, numbers[0] as usize, numbers[1] as usize)),
        "wavelet-denoise" => Ok(wavelet_denoise(&image, numbers[0] / 65535.0, numbers[1])),
        "equalize" => {
            equalize(&mut image);
            Ok(image)
        }
        "auto-level" => {
            auto_level(&mut image);
            Ok(image)
        }
        "auto-gamma" => {
            auto_gamma(&mut image);
            Ok(image)
        }
        "gamma" => {
            gamma(&mut image, numbers[0]);
            Ok(image)
        }
        "noise" => {
            // Arguments: noise type name, then the LP64 secret key.
            let noise = match extra[0].as_str() {
                "gaussian" => NoiseType::Gaussian,
                "impulse" => NoiseType::Impulse,
                "laplacian" => NoiseType::Laplacian,
                "multiplicative" => NoiseType::Multiplicative,
                "poisson" => NoiseType::Poisson,
                "uniform" => NoiseType::Uniform,
                _ => NoiseType::Random,
            };
            let key: u64 = extra[1].parse().expect("seed should be an integer");
            let mut random = MagickRandom::with_secret_key_lp64(key);
            Ok(add_noise(&image, noise, 1.0, &mut random))
        }
        "implode" => Ok(implode(&image, numbers[0], Interpolate::Bilinear)),
        "liquid-rescale" => Ok(liquid_rescale(&image, numbers[0] as usize, numbers[1] as usize)),
        "adaptive-resize" => Ok(adaptive_resize(&image, numbers[0] as usize, numbers[1] as usize)),
        "origami" => {
            // Arguments: folded size, LP64 secret key.
            let folds = numbers[0] as usize;
            let key: u64 = extra[1].parse().expect("seed should be an integer");
            let carved = liquid_rescale(&image, folds, folds);
            let mut random = MagickRandom::with_secret_key_lp64(key);
            let spread_image = spread(&carved, Interpolate::Bilinear, 1.0, &mut random);
            Ok(adaptive_resize(&spread_image, width, height))
        }
        _ => {
            eprintln!("unknown operation {operation}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(output) => {
            write_raw(&arguments[4], &output.data);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
