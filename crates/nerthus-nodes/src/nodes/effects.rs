//! Image-style effects: Flip, Pixelate, Swirl, Edge and Whorl.

use nerthus_core::dotnet_random::DotNetRandom;
use nerthus_engine::{BuildContext, NodeBuilder, NodeError};
use nerthus_image::distort;
use nerthus_image::misc::{self, MagickRandom};
use nerthus_image::quanta;
use nerthus_image::resize;
use nerthus_image::{Interpolate, VirtualPixel};

use crate::magick;
use crate::masking;
use crate::maths;
use crate::others;

pub struct Flip;

impl NodeBuilder for Flip {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let flipped = match context.enum_name("Type") {
            "Y" => maths::flip_y(&input),
            "XY" => maths::flip_xy(&input),
            _ => maths::flip_x(&input),
        };
        context.commit(flipped)
    }
}

pub struct Pixelate;

impl NodeBuilder for Pixelate {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = if context.bool("Autosize") { 0.9977 } else { context.f32("Size") };
        let pixelated = others::pixelate(&input, 1.0 - size, context.bool("Antialias"));
        context.commit(pixelated)
    }
}

pub struct Swirl;

impl NodeBuilder for Swirl {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let amount = context.f32("Size") * 0.1 * 0.5 * (512.0 / context.resolution.x as f32);
        let center = [context.f32("X"), 1.0 - context.f32("Y")];
        let swirled = others::swirl(&input, amount, center);
        context.commit(swirled)
    }
}

pub struct Edge;

impl NodeBuilder for Edge {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = context.f32("Size");
        let edged = match context.enum_name("Style") {
            "Precise" => masking::precise_edge(&input, context.i32("Pixels")),
            "Round" => masking::vignette(&input, size, context.f32("Softness")),
            _ => masking::mask_by_square_edge(&input, size),
        };
        context.commit(edged)
    }
}

/// Whorl: twists the terrain with ImageMagick's Shepards distortion. Stretch
/// drags points near random centres; Spin moves random points onto others
/// with a stronger falloff.
pub struct Whorl;

impl NodeBuilder for Whorl {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let Some(input) = context.input("In") else {
            return Err(NodeError::Failed("Node not connected.".to_string()));
        };
        let mut image = magick::to_image(&input);
        image.virtual_pixel = VirtualPixel::Mirror;
        image.interpolate = Interpolate::Spline;

        let mut random = DotNetRandom::new(context.i32("Seed"));
        let width = image.width as i32;
        let power = context.f32("Power");
        let whorls = context.i32("Whorls");
        let reach = ((image.width as f32 * power) as f64 * 4.0) as i32;
        let mut points = Vec::new();
        let mut power_artifact = None;
        if context.enum_name("Type") == "Spin" {
            power_artifact = Some(power as f64 * 8.0);
            for _ in 0..whorls * 4 {
                points.push(random.next_range(0, width) as f64);
            }
        } else {
            for _ in 0..whorls {
                let centre = random.next_range(0, width);
                for _ in 0..4 {
                    points.push(random.next_range(centre - reach, centre + reach) as f64);
                }
            }
        }
        let warped = distort::distort_shepards(&image, &points, power_artifact).map_err(|error| NodeError::Failed(error.to_string()))?;
        context.commit(magick::to_map(&warped))
    }
}

/// Origami: folds the terrain into flat facets. It is seam carved down to a
/// few pixels (`LiquidRescale`), jittered with a seeded `Spread` and blown
/// back up with mesh interpolation (`AdaptiveResize`).
pub struct Origami;

impl NodeBuilder for Origami {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let image = magick::to_image(&input);
        let resolution = input.resolution();
        let amount = 0.0 + context.f32("Amount") * (0.15 - 0.0);
        let folds = ((amount * 512.0) as i32).max(0) as usize;

        let (folded_width, folded_height) = resize::fit_size(resolution.x, resolution.y, folds, folds);
        let folded = quanta::liquid_rescale(&image, folded_width, folded_height);
        // `MagickNET.SetRandomSeed((ulong)Seed)` reaches IM's
        // `SetRandomSecretKey` as a 32-bit `unsigned long` on Windows.
        let mut random = MagickRandom::with_secret_key(context.i32("Seed") as u32);
        let jittered = misc::spread(&folded, Interpolate::Bilinear, 1.0, &mut random);
        let unfolded = resize::adaptive_resize(&jittered, resolution.x, resolution.y);
        context.commit(magick::to_map(&unfolded))
    }
}
