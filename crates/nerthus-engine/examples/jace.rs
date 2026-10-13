//! Evaluates each line of a formula file the way the engine configures Jace and
//! prints the double's bits (or `error`), for comparison with Jace.dll
//! itself.
//!
//! usage: jace <formulas.txt>

use std::env;
use std::fs;

use nerthus_engine::jace::Calculator;

const TERRAIN_HEIGHT: f32 = 2500.0;

fn main() {
    let path = env::args().nth(1).expect("usage: jace <formulas.txt>");
    let text = fs::read_to_string(path).expect("the formula file should be readable");

    let mut calculator = Calculator::new(TERRAIN_HEIGHT);
    let constants = [
        ("a", 1.5),
        ("b", -2.0),
        ("big", f64::INFINITY),
        ("zero", 0.0),
        ("Resolution", 1024.0),
        ("PX", (1.0f32 / 1024.0) as f64),
    ];
    for (name, value) in constants {
        calculator.add_constant(name, value).expect("the constant is not reserved");
    }

    for line in text.lines() {
        match calculator.calculate(line) {
            Ok(value) => println!("{:016x}", value.to_bits()),
            Err(_) => println!("error"),
        }
    }
}
