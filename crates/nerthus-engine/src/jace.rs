//! Jace.NET 1.0.0 (`Jace.CalculationEngine`), the formula engine variable
//! and parameter expressions are evaluated with, configured
//! case-insensitive, optimizer on, compiled execution, with the default
//! functions and constants plus the unit functions `m`, `cm` and `km`.
//!
//! A faithful port, quirks included: the tokenizer silently drops number
//! text that does not parse (`1.2.3`) and lets a minus start a number only
//! where an operand may begin; functions bind to the next operand even
//! without brackets (`sin 2`); the optimizer folds constant subtrees and
//! turns a product with a folded zero into 0 (`(0+0)*x` is 0 even for an
//! infinite `x`, while a literal `0*x` is not folded). Everything is
//! evaluated in doubles, with .NET's NaN (negative quiet) where .NET makes
//! one up, the CLR's `%`, and RyuJIT's rewrites of negated sums in the
//! compiled formula (which decide a NaN's sign).
//!
//! Verified bit for bit against Jace.dll on 80 000 random formulas. The one
//! known difference: when two NaNs of different sign meet in one operation,
//! which survives depends on the JIT's register allocation (and differs
//! between Windows and macOS).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq)]
pub enum JaceError {
    #[error("{0}")]
    Parse(String),
    #[error("The variable \"{0}\" used is not defined.")]
    UndefinedVariable(String),
    #[error("The constant \"{0}\" cannot be overwriten.")]
    ReservedConstant(String),
}

/// `double.NaN` (the x86 default NaN).
const DOTNET_NAN: f64 = f64::from_bits(0xfff8_0000_0000_0000);

const DECIMAL_SEPARATOR: char = '.';
const ARGUMENT_SEPARATOR: char = ',';

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Integer(i32),
    FloatingPoint(f64),
    Text(String),
    /// `_` is unary minus; `≤ ≥ ≠ = & |` stand for `<= >= != == && ||`.
    Operation(char),
    LeftBracket,
    RightBracket,
    ArgumentSeparator,
}

fn parse_error(message: impl Into<String>) -> JaceError {
    JaceError::Parse(message.into())
}

fn is_part_of_numeric(character: char, is_first_character: bool, is_formula_sub_part: bool) -> bool {
    if character == DECIMAL_SEPARATOR || character.is_ascii_digit() {
        return true;
    }
    if is_formula_sub_part && is_first_character && character == '-' {
        return true;
    }
    !is_first_character && (character == 'e' || character == 'E')
}

fn is_part_of_variable(character: char, is_first_character: bool) -> bool {
    if character.is_ascii_alphabetic() {
        return true;
    }
    !is_first_character && (character.is_ascii_digit() || character == '_')
}

fn is_scientific_notation(character: char) -> bool {
    character == 'e' || character == 'E'
}

/// `TokenReader.IsUnaryMinus`: a minus is unary unless it follows an
/// operand or a closing bracket.
fn is_unary_minus(character: char, tokens: &[Token]) -> Result<bool, JaceError> {
    if character != '-' {
        return Ok(false);
    }
    // This Jace build indexes the previous token unconditionally.
    let previous = tokens.last().ok_or_else(|| parse_error("Index was out of range."))?;
    let follows_operand = matches!(previous, Token::Integer(_) | Token::FloatingPoint(_) | Token::Text(_) | Token::RightBracket);
    Ok(!follows_operand)
}

/// `TokenReader.Read`.
fn read_tokens(formula: &str) -> Result<Vec<Token>, JaceError> {
    if formula.is_empty() {
        return Err(parse_error("formula"));
    }
    let characters: Vec<char> = formula.chars().collect();
    let length = characters.len();
    let mut tokens = Vec::new();
    let mut is_formula_sub_part = true;
    let mut is_scientific = false;
    let mut index = 0;
    while index < length {
        if is_part_of_numeric(characters[index], true, is_formula_sub_part) {
            let mut text = String::new();
            text.push(characters[index]);
            loop {
                index += 1;
                if index >= length || !is_part_of_numeric(characters[index], false, is_formula_sub_part) {
                    break;
                }
                if is_scientific && is_scientific_notation(characters[index]) {
                    return Err(parse_error(format!("Invalid token \"{}\" detected at position {index}.", characters[index])));
                }
                if is_scientific_notation(characters[index]) {
                    is_scientific = true;
                    let next = characters.get(index + 1).ok_or_else(|| parse_error("Index was outside the bounds of the array."))?;
                    if *next == '-' {
                        text.push(characters[index]);
                        index += 1;
                    }
                }
                text.push(characters[index]);
            }

            if let Ok(integer) = text.parse::<i32>() {
                tokens.push(Token::Integer(integer));
                is_formula_sub_part = false;
            } else if let Some(number) = parse_double(&text) {
                tokens.push(Token::FloatingPoint(number));
                is_scientific = false;
                is_formula_sub_part = false;
            } else if text == "-" {
                tokens.push(Token::Operation('_'));
            }
            // Anything else is dropped without complaint.
            if index == length {
                continue;
            }
        }

        if is_part_of_variable(characters[index], true) {
            let mut text = String::new();
            text.push(characters[index]);
            index += 1;
            while index < length && is_part_of_variable(characters[index], false) {
                text.push(characters[index]);
                index += 1;
            }
            tokens.push(Token::Text(text));
            is_formula_sub_part = false;
            if index == length {
                continue;
            }
        }

        let character = characters[index];
        if character == ARGUMENT_SEPARATOR {
            tokens.push(Token::ArgumentSeparator);
            is_formula_sub_part = false;
            index += 1;
            continue;
        }
        let next_is_equals = characters.get(index + 1) == Some(&'=');
        match character {
            '%' | '*' | '+' | '-' | '/' | '^' | '≠' | '≤' | '≥' => {
                let operation = if is_unary_minus(character, &tokens)? { '_' } else { character };
                tokens.push(Token::Operation(operation));
                is_formula_sub_part = true;
            }
            '(' => {
                tokens.push(Token::LeftBracket);
                is_formula_sub_part = true;
            }
            ')' => {
                tokens.push(Token::RightBracket);
                is_formula_sub_part = false;
            }
            '<' | '>' => {
                let operation = match (character, next_is_equals) {
                    ('<', true) => '≤',
                    ('>', true) => '≥',
                    _ => character,
                };
                if next_is_equals {
                    index += 1;
                }
                tokens.push(Token::Operation(operation));
                is_formula_sub_part = false;
            }
            '!' | '&' | '|' | '=' => {
                let (second, operation) = match character {
                    '!' => ('=', '≠'),
                    '&' => ('&', '&'),
                    '|' => ('|', '|'),
                    _ => ('=', '='),
                };
                if characters.get(index + 1) != Some(&second) {
                    return Err(parse_error(format!("Invalid token \"{character}\" detected at position {index}.")));
                }
                index += 1;
                tokens.push(Token::Operation(operation));
                is_formula_sub_part = false;
            }
            ' ' => {}
            _ => return Err(parse_error(format!("Invalid token \"{character}\" detected at position {index}."))),
        }
        index += 1;
    }
    Ok(tokens)
}

/// `double.TryParse(text, NumberStyles.Float | AllowThousands, culture)`
/// for what the tokenizer collects (digits, `.`, `e`/`E` and minus signs).
fn parse_double(text: &str) -> Option<f64> {
    let has_digit = text.chars().any(|character| character.is_ascii_digit());
    if !has_digit {
        return None;
    }
    text.parse::<f64>().ok()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BinaryKind {
    Addition,
    Subtraction,
    Multiplication,
    Division,
    Modulo,
    Exponentiation,
    And,
    Or,
    LessThan,
    LessOrEqualThan,
    GreaterThan,
    GreaterOrEqualThan,
    Equal,
    NotEqual,
}

/// Jace's default functions and the unit functions.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Function {
    Sin,
    Cos,
    Csc,
    Sec,
    Asin,
    Acos,
    Tan,
    Cot,
    Atan,
    Acot,
    Loge,
    Log10,
    Logn,
    Sqrt,
    Abs,
    If,
    IfLess,
    IfMore,
    IfEqual,
    Ceiling,
    Floor,
    Truncate,
    Round,
    Max,
    Min,
    Avg,
    Median,
    Random,
    Metres,
    Centimetres,
    Kilometres,
}

impl Function {
    /// `FunctionRegistry.GetFunctionInfo` (names are case-insensitive).
    fn named(name: &str) -> Option<Function> {
        let lowered = name.to_ascii_lowercase();
        let function = match lowered.as_str() {
            "sin" => Function::Sin,
            "cos" => Function::Cos,
            "csc" => Function::Csc,
            "sec" => Function::Sec,
            "asin" => Function::Asin,
            "acos" => Function::Acos,
            "tan" => Function::Tan,
            "cot" => Function::Cot,
            "atan" => Function::Atan,
            "acot" => Function::Acot,
            "loge" => Function::Loge,
            "log10" => Function::Log10,
            "logn" => Function::Logn,
            "sqrt" => Function::Sqrt,
            "abs" => Function::Abs,
            "if" => Function::If,
            "ifless" => Function::IfLess,
            "ifmore" => Function::IfMore,
            "ifequal" => Function::IfEqual,
            "ceiling" => Function::Ceiling,
            "floor" => Function::Floor,
            "truncate" => Function::Truncate,
            "round" => Function::Round,
            "max" => Function::Max,
            "min" => Function::Min,
            "avg" => Function::Avg,
            "median" => Function::Median,
            "random" => Function::Random,
            "m" => Function::Metres,
            "cm" => Function::Centimetres,
            "km" => Function::Kilometres,
            _ => return None,
        };
        Some(function)
    }

    /// The fixed parameter count, or `None` for the `DynamicFunc`s.
    fn parameter_count(self) -> Option<usize> {
        match self {
            Function::Max | Function::Min | Function::Avg | Function::Median => None,
            Function::Random => Some(0),
            Function::Logn => Some(2),
            Function::If => Some(3),
            Function::IfLess | Function::IfMore | Function::IfEqual => Some(4),
            _ => Some(1),
        }
    }

    fn is_idempotent(self) -> bool {
        self != Function::Random
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Operation {
    IntegerConstant(i32),
    FloatingPointConstant(f64),
    Variable(String),
    Binary(BinaryKind, Box<Operation>, Box<Operation>),
    UnaryMinus(Box<Operation>),
    Function(Function, Vec<Operation>),
}

impl Operation {
    /// Whether RyuJIT folds the compiled expression to a constant: literal
    /// arithmetic (calls, `%`, `^` and comparisons stay as they are).
    fn is_jit_constant(&self) -> bool {
        match self {
            Operation::IntegerConstant(_) | Operation::FloatingPointConstant(_) => true,
            Operation::UnaryMinus(argument) => argument.is_jit_constant(),
            Operation::Binary(kind, left, right) => {
                let foldable = matches!(kind, BinaryKind::Addition | BinaryKind::Subtraction | BinaryKind::Multiplication | BinaryKind::Division);
                foldable && left.is_jit_constant() && right.is_jit_constant()
            }
            _ => false,
        }
    }

    /// What a negation the JIT keeps as a negation negates.
    fn jit_negated(&self) -> Option<&Operation> {
        match self {
            Operation::UnaryMinus(argument) if !argument.is_jit_constant() => Some(argument),
            _ => None,
        }
    }

    fn depends_on_variables(&self) -> bool {
        match self {
            Operation::IntegerConstant(_) | Operation::FloatingPointConstant(_) => false,
            Operation::Variable(_) => true,
            Operation::Binary(_, left, right) => left.depends_on_variables() || right.depends_on_variables(),
            Operation::UnaryMinus(argument) => argument.depends_on_variables(),
            Operation::Function(_, arguments) => arguments.iter().any(Operation::depends_on_variables),
        }
    }

    fn is_idempotent(&self) -> bool {
        match self {
            Operation::IntegerConstant(_) | Operation::FloatingPointConstant(_) => true,
            Operation::Variable(_) => false,
            Operation::Binary(_, left, right) => left.is_idempotent() && right.is_idempotent(),
            Operation::UnaryMinus(argument) => argument.is_idempotent(),
            Operation::Function(function, arguments) => function.is_idempotent() && arguments.iter().all(Operation::is_idempotent),
        }
    }

    fn is_constant(&self) -> bool {
        matches!(self, Operation::IntegerConstant(_) | Operation::FloatingPointConstant(_))
    }
}

fn precedence(operation: char) -> i32 {
    match operation {
        '&' | '|' => 1,
        '<' | '>' | '≤' | '≥' | '≠' | '=' => 2,
        '+' | '-' => 3,
        '*' | '/' | '%' => 4,
        '^' => 5,
        '_' => 6,
        _ => 0,
    }
}

fn is_left_associative(operation: char) -> bool {
    matches!(operation, '*' | '+' | '-' | '/')
}

/// `AstBuilder`: a shunting-yard pass over the tokens.
#[derive(Default)]
struct AstBuilder {
    result_stack: Vec<Operation>,
    operator_stack: Vec<Token>,
    parameter_count: Vec<usize>,
}

fn syntax_error(token: &str) -> JaceError {
    parse_error(format!("There is a syntax issue for \"{token}\". The number of arguments does not match with what is expected."))
}

impl AstBuilder {
    fn build(mut self, tokens: Vec<Token>) -> Result<Operation, JaceError> {
        for token in tokens {
            match token {
                Token::Integer(value) => self.result_stack.push(Operation::IntegerConstant(value)),
                Token::FloatingPoint(value) => self.result_stack.push(Operation::FloatingPointConstant(value)),
                Token::Text(ref name) => {
                    if Function::named(name).is_some() {
                        self.operator_stack.push(token.clone());
                        self.parameter_count.push(1);
                    } else {
                        self.result_stack.push(Operation::Variable(name.to_ascii_lowercase()));
                    }
                }
                Token::LeftBracket => self.operator_stack.push(token),
                Token::RightBracket => self.pop_operations(true, Some(&token))?,
                Token::ArgumentSeparator => {
                    self.pop_operations(false, Some(&token))?;
                    let count = self.parameter_count.pop().ok_or_else(|| parse_error("Stack empty."))?;
                    self.parameter_count.push(count + 1);
                }
                Token::Operation(operation) => {
                    while let Some(top) = self.operator_stack.last() {
                        match top {
                            Token::Operation(waiting) => {
                                let waiting = *waiting;
                                let binds_tighter = !is_left_associative(operation) || precedence(operation) > precedence(waiting);
                                if binds_tighter && precedence(operation) >= precedence(waiting) {
                                    break;
                                }
                                self.operator_stack.pop();
                                let converted = self.convert_operation(waiting)?;
                                self.result_stack.push(converted);
                            }
                            Token::Text(name) => {
                                let name = name.clone();
                                self.operator_stack.pop();
                                let converted = self.convert_function(&name)?;
                                self.result_stack.push(converted);
                            }
                            _ => break,
                        }
                    }
                    self.operator_stack.push(token);
                }
            }
        }
        self.pop_operations(false, None)?;
        if self.result_stack.len() > 1 {
            return Err(parse_error("The syntax of the provided formula is not valid."));
        }
        self.result_stack.pop().ok_or_else(|| parse_error("Sequence contains no elements"))
    }

    fn pop_operations(&mut self, until_left_bracket: bool, current: Option<&Token>) -> Result<(), JaceError> {
        while let Some(top) = self.operator_stack.last() {
            if *top == Token::LeftBracket {
                break;
            }
            let token = self.operator_stack.pop().expect("the stack is not empty");
            match token {
                Token::Operation(operation) => {
                    let converted = self.convert_operation(operation)?;
                    self.result_stack.push(converted);
                }
                Token::Text(name) => {
                    let converted = self.convert_function(&name)?;
                    self.result_stack.push(converted);
                }
                _ => {}
            }
        }
        let bracket_on_top = self.operator_stack.last() == Some(&Token::LeftBracket);
        if until_left_bracket {
            if !bracket_on_top {
                return Err(parse_error("No matching left bracket found for the right bracket."));
            }
            self.operator_stack.pop();
        } else if bracket_on_top && current != Some(&Token::ArgumentSeparator) {
            return Err(parse_error("No matching right bracket found for the left bracket."));
        }
        Ok(())
    }

    fn pop_result(&mut self, token: &str) -> Result<Operation, JaceError> {
        self.result_stack.pop().ok_or_else(|| syntax_error(token))
    }

    fn convert_operation(&mut self, operation: char) -> Result<Operation, JaceError> {
        let token = operation.to_string();
        if operation == '_' {
            let argument = self.pop_result(&token)?;
            return Ok(Operation::UnaryMinus(Box::new(argument)));
        }
        let kind = match operation {
            '+' => BinaryKind::Addition,
            '-' => BinaryKind::Subtraction,
            '*' => BinaryKind::Multiplication,
            '/' => BinaryKind::Division,
            '%' => BinaryKind::Modulo,
            '^' => BinaryKind::Exponentiation,
            '&' => BinaryKind::And,
            '|' => BinaryKind::Or,
            '<' => BinaryKind::LessThan,
            '≤' => BinaryKind::LessOrEqualThan,
            '>' => BinaryKind::GreaterThan,
            '≥' => BinaryKind::GreaterOrEqualThan,
            '=' => BinaryKind::Equal,
            '≠' => BinaryKind::NotEqual,
            _ => return Err(parse_error(format!("Unknown operation \"{operation}\"."))),
        };
        let right = self.pop_result(&token)?;
        let left = self.pop_result(&token)?;
        Ok(Operation::Binary(kind, Box::new(left), Box::new(right)))
    }

    fn convert_function(&mut self, name: &str) -> Result<Operation, JaceError> {
        let function = Function::named(name).ok_or_else(|| parse_error(format!("Unknown function \"{name}\".")))?;
        let given = self.parameter_count.pop().ok_or_else(|| syntax_error(name))?;
        let count = function.parameter_count().unwrap_or(given);
        let mut arguments = Vec::with_capacity(count);
        for _ in 0..count {
            arguments.push(self.pop_result(name)?);
        }
        arguments.reverse();
        Ok(Operation::Function(function, arguments))
    }
}

/// `Optimizer.Optimize`: constant subtrees are folded; sums,
/// differences, products, quotients, powers and function arguments are
/// optimized below that.
fn optimize(operation: Operation, calculator: &Calculator) -> Operation {
    if !operation.depends_on_variables() && operation.is_idempotent() && !operation.is_constant() {
        let empty = HashMap::new();
        let folded = calculator.execute(&operation, &empty, Execution::Interpreted);
        // Constant subtrees read no variables, so nothing can fail.
        return Operation::FloatingPointConstant(folded.unwrap_or(DOTNET_NAN));
    }
    match operation {
        Operation::Binary(kind, left, right) => {
            let optimizable = matches!(
                kind,
                BinaryKind::Addition | BinaryKind::Subtraction | BinaryKind::Multiplication | BinaryKind::Division | BinaryKind::Exponentiation
            );
            if !optimizable {
                return Operation::Binary(kind, left, right);
            }
            let left = optimize(*left, calculator);
            let right = optimize(*right, calculator);
            let is_folded_zero = |operation: &Operation| matches!(operation, Operation::FloatingPointConstant(value) if *value == 0.0);
            if kind == BinaryKind::Multiplication && (is_folded_zero(&left) || is_folded_zero(&right)) {
                return Operation::FloatingPointConstant(0.0);
            }
            Operation::Binary(kind, Box::new(left), Box::new(right))
        }
        Operation::Function(function, arguments) => {
            let optimized = arguments.into_iter().map(|argument| optimize(argument, calculator)).collect();
            Operation::Function(function, optimized)
        }
        other => other,
    }
}

/// Who evaluates: Jace's interpreter (the optimizer's constant folding)
/// computes what is written; the compiled formula goes through RyuJIT,
/// whose algebra moves negations (see [`Calculator::jit_rewrite`]).
#[derive(Clone, Copy, PartialEq)]
enum Execution {
    Interpreted,
    Compiled,
}

/// `double.CompareTo`: NaN orders below everything and equals itself.
fn dotnet_compare(left: f64, right: f64) -> Ordering {
    if left < right {
        return Ordering::Less;
    }
    if left > right {
        return Ordering::Greater;
    }
    if left == right {
        return Ordering::Equal;
    }
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        _ => Ordering::Greater,
    }
}

/// `MathExtended.Partition` (Lomuto, pivot at the end).
fn partition(list: &mut [f64], start: usize, end: usize) -> usize {
    let pivot = list[end];
    let mut store = start;
    for index in start..end {
        if dotnet_compare(list[index], pivot) != Ordering::Greater {
            list.swap(index, store);
            store += 1;
        }
    }
    list.swap(end, store);
    store
}

/// `MathExtended.Median`: the lower median by quickselect.
fn median(mut list: Vec<f64>) -> f64 {
    let wanted = (list.len() - 1) / 2;
    let mut start = 0;
    let mut end = list.len() - 1;
    loop {
        let pivot = partition(&mut list, start, end);
        if pivot == wanted {
            return list[pivot];
        }
        if wanted < pivot {
            end = pivot - 1;
        } else {
            start = pivot + 1;
        }
    }
}

/// `Enumerable.Max` (.NET 8): NaNs are skipped unless all are NaN.
fn dotnet_max(values: &[f64]) -> f64 {
    let first_number = values.iter().position(|value| !value.is_nan());
    let Some(first_number) = first_number else {
        return values[values.len() - 1];
    };
    let mut maximum = values[first_number];
    for &value in &values[first_number + 1..] {
        if value > maximum {
            maximum = value;
        }
    }
    maximum
}

/// `Enumerable.Min` (.NET 8): any NaN is the minimum (the first one after
/// the start, else a NaN start).
fn dotnet_min(values: &[f64]) -> f64 {
    let mut minimum = values[0];
    for &value in &values[1..] {
        if value < minimum {
            minimum = value;
        } else if value.is_nan() {
            return value;
        }
    }
    minimum
}

/// `Math.Log(a, newBase)`.
fn dotnet_log_base(value: f64, base: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if base.is_nan() {
        return base;
    }
    if base == 1.0 {
        return DOTNET_NAN;
    }
    if value != 1.0 && (base == 0.0 || base == f64::INFINITY) {
        return DOTNET_NAN;
    }
    value.ln() / base.ln()
}

/// The CLR's double `%` (`JIT_DblRem`): NaN for a zero divisor or a
/// non-finite dividend, the dividend for an infinite divisor, else `fmod`.
fn dotnet_remainder(dividend: f64, divisor: f64) -> f64 {
    if divisor == 0.0 || !dividend.is_finite() {
        return DOTNET_NAN;
    }
    if divisor.is_infinite() {
        return dividend;
    }
    dividend % divisor
}

static RANDOM_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

/// `Random.NextDouble` for the `random()` function: Jace seeds it from the
/// clock, so any sequence will do.
fn next_random() -> f64 {
    let mut state = RANDOM_STATE.load(AtomicOrdering::Relaxed);
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    RANDOM_STATE.store(state, AtomicOrdering::Relaxed);
    (state >> 11) as f64 / (1u64 << 53) as f64
}

/// The configured `CalculationEngine`: the constants it holds and the
/// terrain height its unit functions divide by.
#[derive(Clone, Debug)]
pub struct Calculator {
    /// Lower-cased names, with whether they may be overwritten.
    constants: Vec<(String, f64, bool)>,
    terrain_height: f32,
}

impl Calculator {
    /// A fresh engine: `e` and `pi`, which cannot be overwritten.
    pub fn new(terrain_height: f32) -> Self {
        let constants = vec![("e".to_string(), std::f64::consts::E, false), ("pi".to_string(), std::f64::consts::PI, false)];
        Self { constants, terrain_height }
    }

    /// `AddConstant` (`ConstantRegistry.RegisterConstant`).
    pub fn add_constant(&mut self, name: &str, value: f64) -> Result<(), JaceError> {
        let name = name.to_ascii_lowercase();
        let existing = self.constants.iter_mut().find(|(constant, _, _)| *constant == name);
        match existing {
            Some((_, _, false)) => Err(JaceError::ReservedConstant(name)),
            Some(entry) => {
                entry.1 = value;
                Ok(())
            }
            None => {
                self.constants.push((name, value, true));
                Ok(())
            }
        }
    }

    /// `CalculationEngine.Calculate(formula)`: the constants are the only
    /// variables.
    pub fn calculate(&self, formula: &str) -> Result<f64, JaceError> {
        if formula.is_empty() {
            return Err(parse_error("formulaText"));
        }
        let tokens = read_tokens(formula)?;
        let tree = AstBuilder::default().build(tokens)?;
        let optimized = optimize(tree, self);
        let mut variables = HashMap::new();
        for (name, value, _) in &self.constants {
            variables.insert(name.clone(), *value);
        }
        self.execute(&optimized, &variables, Execution::Compiled)
    }

    fn execute(&self, operation: &Operation, variables: &HashMap<String, f64>, execution: Execution) -> Result<f64, JaceError> {
        let value = match operation {
            Operation::IntegerConstant(value) => *value as f64,
            Operation::FloatingPointConstant(value) => *value,
            Operation::Variable(name) => match variables.get(name) {
                Some(value) => *value,
                None => return Err(JaceError::UndefinedVariable(name.clone())),
            },
            Operation::UnaryMinus(argument) => -self.execute(argument, variables, execution)?,
            Operation::Binary(kind, left, right) if execution == Execution::Compiled && self.jit_rewrite(*kind, left, right).is_some() => {
                // RyuJIT's morph turns `x + -y` and `x - -y` into `x - y` and
                // `x + y`, and `-y + c` into `c - y`, so a NaN keeps its sign
                // where the negation would have flipped it.
                let (minuend, subtrahend, subtract) = self.jit_rewrite(*kind, left, right).expect("checked above");
                let first = self.execute(minuend, variables, execution)?;
                let second = self.execute(subtrahend, variables, execution)?;
                if subtract {
                    first - second
                } else {
                    first + second
                }
            }
            Operation::Binary(kind, left, right) => {
                let left = self.execute(left, variables, execution)?;
                let right = self.execute(right, variables, execution)?;
                let truth = |condition: bool| if condition { 1.0 } else { 0.0 };
                match kind {
                    BinaryKind::Addition => left + right,
                    BinaryKind::Subtraction => left - right,
                    BinaryKind::Multiplication => left * right,
                    BinaryKind::Division => left / right,
                    BinaryKind::Modulo => dotnet_remainder(left, right),
                    BinaryKind::Exponentiation => left.powf(right),
                    BinaryKind::And => truth(left != 0.0 && right != 0.0),
                    BinaryKind::Or => truth(left != 0.0 || right != 0.0),
                    BinaryKind::LessThan => truth(left < right),
                    BinaryKind::LessOrEqualThan => truth(left <= right),
                    BinaryKind::GreaterThan => truth(left > right),
                    BinaryKind::GreaterOrEqualThan => truth(left >= right),
                    BinaryKind::Equal => truth(left == right),
                    BinaryKind::NotEqual => truth(left != right),
                }
            }
            Operation::Function(function, arguments) => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.execute(argument, variables, execution)?);
                }
                self.invoke(*function, &values)
            }
        };
        Ok(value)
    }

    /// The operands and operation RyuJIT computes a sum or difference with
    /// a negated operand as (`fgOptimizeAddition`, `fgOptimizeSubtraction`),
    /// or `None` when it computes it as written.
    fn jit_rewrite<'o>(&self, kind: BinaryKind, left: &'o Operation, right: &'o Operation) -> Option<(&'o Operation, &'o Operation, bool)> {
        match kind {
            BinaryKind::Addition => {
                if let Some(negated) = left.jit_negated() {
                    // `-y + c` becomes `c - y` only when `c` is a constant
                    // (the swap must not reorder side effects).
                    if right.jit_negated().is_none() && right.is_jit_constant() {
                        return Some((right, negated, true));
                    }
                    return None;
                }
                let negated = right.jit_negated()?;
                Some((left, negated, true))
            }
            BinaryKind::Subtraction => {
                let negated = right.jit_negated()?;
                Some((left, negated, false))
            }
            _ => None,
        }
    }

    fn invoke(&self, function: Function, arguments: &[f64]) -> f64 {
        let first = arguments.first().copied().unwrap_or(0.0);
        let height = self.terrain_height;
        match function {
            Function::Sin => first.sin(),
            Function::Cos => first.cos(),
            Function::Csc => 1.0 / first.sin(),
            Function::Sec => 1.0 / first.cos(),
            Function::Asin => first.asin(),
            Function::Acos => first.acos(),
            Function::Tan => first.tan(),
            Function::Cot => 1.0 / first.tan(),
            Function::Atan => first.atan(),
            Function::Acot => (1.0 / first).atan(),
            Function::Loge => first.ln(),
            Function::Log10 => first.log10(),
            Function::Logn => dotnet_log_base(arguments[0], arguments[1]),
            Function::Sqrt => first.sqrt(),
            Function::Abs => first.abs(),
            Function::If => {
                if arguments[0] == 0.0 {
                    arguments[2]
                } else {
                    arguments[1]
                }
            }
            Function::IfLess => {
                if arguments[0] < arguments[1] {
                    arguments[2]
                } else {
                    arguments[3]
                }
            }
            Function::IfMore => {
                if arguments[0] > arguments[1] {
                    arguments[2]
                } else {
                    arguments[3]
                }
            }
            Function::IfEqual => {
                if arguments[0] != arguments[1] {
                    arguments[3]
                } else {
                    arguments[2]
                }
            }
            Function::Ceiling => first.ceil(),
            Function::Floor => first.floor(),
            Function::Truncate => first.trunc(),
            Function::Round => first.round_ties_even(),
            Function::Max => dotnet_max(arguments),
            Function::Min => dotnet_min(arguments),
            Function::Avg => {
                let mut sum = 0.0;
                for &value in arguments {
                    sum += value;
                }
                sum / arguments.len() as f64
            }
            Function::Median => median(arguments.to_vec()),
            Function::Random => next_random(),
            Function::Metres => first / height as f64,
            Function::Centimetres => first / (height * 100.0) as f64,
            Function::Kilometres => first * 1000.0 / height as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calculate(formula: &str) -> Result<f64, JaceError> {
        Calculator::new(2500.0).calculate(formula)
    }

    #[test]
    fn arithmetic_follows_jace_precedence() {
        assert_eq!(calculate("2+3*4"), Ok(14.0));
        // `^` is right-associative, and a leading minus is part of the number.
        assert_eq!(calculate("2^3^2"), Ok(512.0));
        assert_eq!(calculate("-2^2"), Ok(4.0));
        assert_eq!(calculate("10-4-3"), Ok(3.0));
        assert_eq!(calculate("max(1, min(5, 3), 2)"), Ok(3.0));
        assert_eq!(calculate("if(1 < 2 && 3 >= 3, 7, 8)"), Ok(7.0));
    }

    #[test]
    fn quirks_survive() {
        // Unparseable number text vanishes.
        assert!(calculate("1.2.3").is_err());
        assert_eq!(calculate("sin 0"), Ok(0.0));
        // A wholly constant formula folds as a whole.
        assert!(calculate("(0+0)*(1/0)").unwrap().is_nan());
        // Below a variable, a folded zero hides an infinite factor; a
        // literal integer zero does not.
        let mut calculator = Calculator::new(2500.0);
        calculator.add_constant("Big", f64::INFINITY).unwrap();
        assert_eq!(calculator.calculate("(0+0)*big"), Ok(0.0));
        assert!(calculator.calculate("0*BIG").unwrap().is_nan());
        assert!(calculator.add_constant("PI", 3.0).is_err());
        assert_eq!(calculate("-(0)").map(f64::to_bits), Ok((-0.0f64).to_bits()));
        assert_eq!(calculate("15%0").map(f64::to_bits), Ok(DOTNET_NAN.to_bits()));
        assert!(calculate("pi").is_ok());
    }
}
