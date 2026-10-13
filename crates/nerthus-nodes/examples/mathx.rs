//! The Rust side of `native-run mathx`: each formula of a file through
//! `mathx::compute` on size² maps, printing every result's bits (or the
//! error) in the same format.
//!
//! mathx <formulas.txt> <size> <normalized 0|1> <maps 0|1> [variables]

use std::io::Write;

use nerthus_nodes::mathx;

/// The a, b, c maps native-run's `mathTestMaps` makes.
fn test_maps(count: usize) -> [Vec<f32>; 3] {
    let mut a = vec![0.0f32; count];
    let mut b = vec![0.0f32; count];
    let mut c = vec![0.0f32; count];
    for index in 0..count {
        a[index] = index as f32 / count as f32 * 1.5 - 0.25;
        b[index] = (index * 37 % 101) as f32 / 100.0;
        c[index] = (index % 7) as f32 / 7.0 - 0.3;
    }
    [a, b, c]
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 5 {
        eprintln!("mathx <formulas.txt> <size> <normalized 0|1> <maps 0|1> [variables]");
        std::process::exit(2);
    }
    let formulas = std::fs::read_to_string(&arguments[1]).expect("formulas");
    let size: usize = arguments[2].parse().expect("size");
    let normalized = arguments[3] != "0";
    let with_maps = arguments[4] != "0";
    let variables = arguments.get(5).map_or("Resolution=4;PX=0.25;", String::as_str);
    let maps = test_maps(size * size);
    let inputs: [Option<&[f32]>; 3] = if with_maps { [Some(&maps[0]), Some(&maps[1]), Some(&maps[2])] } else { [None, None, None] };

    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for formula in formulas.lines() {
        match mathx::compute(formula, variables, inputs, size, size, normalized) {
            Err(error) => {
                writeln!(output, "error {error}").unwrap();
            }
            Ok(result) => {
                let words: Vec<String> = result.iter().map(|value| format!("{:08x}", value.to_bits())).collect();
                writeln!(output, "{}", words.join(" ")).unwrap();
            }
        }
    }
}
