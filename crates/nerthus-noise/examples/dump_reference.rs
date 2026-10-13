//! Writes noise samples in the same layout as the FastNoiseSIMD reference
//! harness (`tools/re` docs) so the two can be compared numerically.

use std::io::Write;

use nerthus_noise::{CellularDistanceFunction, CellularReturnType, FastNoise, FractalType, NoiseType, PerturbType};

fn main() {
    const N: usize = 48;
    let types = [0, 1, 2, 3, 4, 5, 7, 8, 9];
    let cell_returns = [0, 1, 2, 3, 4, 5, 6, 7];
    let path = std::env::args().nth(1).unwrap_or_else(|| "rust.bin".to_string());
    let mut file = std::fs::File::create(&path).expect("cannot create output");
    for &t in &types {
        for f in 0..3 {
            for p in 0..3 {
                let mut noise = FastNoise::new(1234 + t * 7 + f * 3 + p);
                noise.set_noise_type(NoiseType::from_index(t));
                noise.set_fractal_type(FractalType::from_index(f));
                noise.set_frequency(0.037);
                noise.set_fractal_octaves(4);
                noise.set_perturb_type(PerturbType::from_index(p));
                noise.set_perturb_amp(2.5);
                noise.set_perturb_frequency(0.21);
                noise.set_perturb_fractal_octaves(3);
                noise.set_axis_scales(1.0, 1.3, 0.8);
                let cell_return = if t == 7 { cell_returns[((f * 3 + p) % 8) as usize] } else { 1 };
                let cell_distance = if t == 7 { (f + p) % 3 } else { 0 };
                noise.set_cellular_return_type(CellularReturnType::from_index(cell_return));
                noise.set_cellular_distance_function(CellularDistanceFunction::from_index(cell_distance));
                noise.set_cellular_noise_lookup_type(NoiseType::PerlinFractal);
                noise.set_cellular_noise_lookup_frequency(0.3);
                let values = noise.fill_slice(N, N, 0, 0, 1.0);
                for value in values {
                    file.write_all(&value.to_le_bytes()).unwrap();
                }
            }
        }
    }
    println!("wrote {path}");
}
