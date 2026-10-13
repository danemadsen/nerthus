//! Runs the Rust port of the native FastBlur4 on a raw f32 map, for
//! comparison with the native shim's `native-run`.
//!
//! usage: fast-blur <size> <radius> <in.raw> <out.raw>

use std::env;
use std::fs;

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let size: usize = arguments[0].parse().expect("size");
    let radius: f32 = arguments[1].parse().expect("radius");
    let bytes = fs::read(&arguments[2]).expect("input");
    let mut data: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect();
    nerthus_nodes::fast_blur::fast_blur4(&mut data, size, size, radius);
    let output: Vec<u8> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(&arguments[3], output).expect("output");
}
