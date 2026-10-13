//! A port of ExprTk (the C++ Mathematical Expression Toolkit) as the
//! MathX node uses it: `exprtk::parser<float>` with its default
//! settings, a symbol table of variables and constants, and expressions
//! evaluated in `float`.
//!
//! ExprTk is not a plain evaluator: while it parses, it folds constants,
//! inserts implicit multiplications and rewrites many operator patterns
//! (`(c0 + v) + c1` into `(c0 + c1) + v`, `(v0 / v1) / v2` into
//! `v0 / (v1 * v2)`, integer powers into multiplications, …). Those rewrites
//! change float rounding, so the port reproduces them. The snapshot ported
//! is close to the 2024-01-01 release, compiled by MSVC with `__cplusplus`
//! at `199711L`; both show in the details.

pub mod functions;
pub mod lexer;
pub mod node;
pub mod number;
mod parser;
mod synth;

pub use parser::{is_reserved_symbol, SymbolTable};

use node::Node;

/// A compiled expression.
#[derive(Clone, Debug)]
pub struct Expression {
    root: Node,
    slot_count: usize,
}

impl Expression {
    /// The number of variable slots evaluation needs: the symbol table's
    /// variables first, then the expression's own `var`s.
    pub fn slot_count(&self) -> usize {
        self.slot_count
    }

    /// Evaluates the expression with `slots` holding the variables (and
    /// space for locals).
    pub fn value(&self, slots: &mut [f32]) -> f32 {
        self.root.evaluate(slots)
    }

    pub fn root(&self) -> &Node {
        &self.root
    }
}

/// `parser::compile`: the expression or ExprTk's first diagnostic.
pub fn compile(expression: &str, symbols: &SymbolTable) -> Result<Expression, String> {
    let tokens = lexer::tokenize(expression).map_err(|error| error.0)?;
    let end = expression.chars().count();
    let mut parser = parser::Parser::new(tokens, symbols.clone(), end);
    match parser.compile() {
        Some(root) => Ok(Expression { root, slot_count: parser.slot_count }),
        None => Err(parser.errors.first().cloned().unwrap_or_else(|| "No Error".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols() -> SymbolTable {
        SymbolTable { variables: ["x", "y", "a", "b", "c"].map(String::from).to_vec(), constants: vec![("Resolution".to_string(), 4.0), ("PX".to_string(), 0.25)] }
    }

    fn evaluate(expression: &str, values: [f32; 5]) -> f32 {
        let compiled = compile(expression, &symbols()).unwrap();
        let mut slots = vec![0.0; compiled.slot_count()];
        slots[..5].copy_from_slice(&values);
        compiled.value(&mut slots)
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(evaluate("1 + 2 * 3", [0.0; 5]), 7.0);
        assert_eq!(evaluate("2^3^2", [0.0; 5]), 512.0);
        assert_eq!(evaluate("x * 2 + y", [1.5, 0.25, 0.0, 0.0, 0.0]), 3.25);
        assert_eq!(evaluate("resolution * px", [0.0; 5]), 1.0);
        assert_eq!(evaluate("2x", [3.0, 0.0, 0.0, 0.0, 0.0]), 6.0);
        assert_eq!(evaluate("-(-x)", [3.0, 0.0, 0.0, 0.0, 0.0]), 3.0);
        assert_eq!(evaluate("x > 1 ? 10 : 20", [3.0, 0.0, 0.0, 0.0, 0.0]), 10.0);
        assert_eq!(evaluate("if (x > 1, 10, 20)", [0.0; 5]), 20.0);
        assert_eq!(evaluate("var t := x * 2; t + 1", [3.0, 0.0, 0.0, 0.0, 0.0]), 7.0);
        assert_eq!(evaluate("clamp(0, x, 1)", [3.0, 0.0, 0.0, 0.0, 0.0]), 1.0);
        assert_eq!(evaluate("max(x, y, a)", [3.0, 4.0, 1.0, 0.0, 0.0]), 4.0);
    }

    #[test]
    fn snapshot_quirks() {
        let table = symbols();
        assert!(compile("atan(x) + atan(y)", &table).is_err());
        assert!(compile("rand()", &table).unwrap_err().contains("Undefined symbol"));
        assert!(compile("1e39", &table).unwrap_err().contains("Invalid numeric token"));
    }
}
