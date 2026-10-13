//! ExprTk's parser (`exprtk::parser<float>` with default settings): a
//! precedence-climbing expression parser whose every operation goes
//! through the expression generator (`synth.rs`).

use crate::exprtk::lexer::{Token, TokenKind};
use crate::exprtk::node::{BinaryOp, FunctionOp, Node, UnaryOp, VarArgOp};
use crate::exprtk::number::string_to_real;
use crate::exprtk::synth::Operation;

/// `details::reserved_symbols`.
pub const RESERVED_SYMBOLS: &[&str] = &[
    "abs", "acos", "acosh", "and", "asin", "asinh", "assert", "atan", "atanh", "atan2", "avg", "break", "case", "ceil", "clamp", "continue", "const", "cos",
    "cosh", "cot", "csc", "default", "deg2grad", "deg2rad", "equal", "erf", "erfc", "exp", "expm1", "false", "floor", "for", "frac", "grad2deg", "hypot",
    "iclamp", "if", "else", "ilike", "in", "inrange", "like", "log", "log10", "log2", "logn", "log1p", "mand", "max", "min", "mod", "mor", "mul", "ncdf",
    "nand", "nor", "not", "not_equal", "null", "or", "pow", "rad2deg", "repeat", "return", "root", "round", "roundn", "sec", "sgn", "shl", "shr", "sin",
    "sinc", "sinh", "sqrt", "sum", "swap", "switch", "tan", "tanh", "true", "trunc", "until", "var", "while", "xnor", "xor", "&", "|",
];

pub fn is_reserved_symbol(symbol: &str) -> bool {
    RESERVED_SYMBOLS.iter().any(|reserved| reserved.eq_ignore_ascii_case(symbol))
}

/// A base function (`base_ops_map_`) and its parameter count.
#[derive(Clone, Copy)]
enum BaseOp {
    Unary(UnaryOp),
    Binary(BinaryOp),
    Trinary(FunctionOp),
}

impl BaseOp {
    fn named(name: &str) -> Option<BaseOp> {
        if let Some(op) = UnaryOp::named(name) {
            return Some(BaseOp::Unary(op));
        }
        if let Some(op) = BinaryOp::function_named(name) {
            return Some(BaseOp::Binary(op));
        }
        FunctionOp::named(name).map(BaseOp::Trinary)
    }

    fn parameter_count(self) -> usize {
        match self {
            BaseOp::Unary(_) => 1,
            BaseOp::Binary(_) => 2,
            BaseOp::Trinary(_) => 3,
        }
    }
}

/// A `var` defined in the expression (`scope_element`).
struct Local {
    name: String,
    slot: usize,
    depth: usize,
    active: bool,
    /// A `const var`'s value.
    constant: Option<f32>,
}

/// The variables and constants an expression may use.
#[derive(Clone, Debug, Default)]
pub struct SymbolTable {
    /// Variable names by slot.
    pub variables: Vec<String>,
    /// Constants (folded into the expression).
    pub constants: Vec<(String, f32)>,
}

impl SymbolTable {
    fn variable(&self, name: &str) -> Option<usize> {
        self.variables.iter().position(|variable| variable.eq_ignore_ascii_case(name))
    }

    fn constant(&self, name: &str) -> Option<f32> {
        let found = self.constants.iter().find(|(constant, _)| constant.eq_ignore_ascii_case(name));
        found.map(|(_, value)| *value)
    }

    /// `symbol_table::symbol_exists`.
    pub fn exists(&self, name: &str) -> bool {
        self.variable(name).is_some() || self.constant(name).is_some()
    }
}

pub(crate) struct Parser {
    tokens: Vec<Token>,
    next: usize,
    current: Token,
    eof: Token,
    symbols: SymbolTable,
    locals: Vec<Local>,
    scope_depth: usize,
    loop_depth: usize,
    parsing_break: bool,
    /// `brkcnt_list_`: whether each loop being parsed uses break/continue.
    break_scopes: Vec<bool>,
    /// The number of variable slots (symbol table variables, then locals).
    pub(crate) slot_count: usize,
    pub(crate) errors: Vec<String>,
    pub(crate) synthesis_error: String,
    pub(crate) side_effect_present: bool,
}

/// `precedence_level`.
const LEVEL00: u8 = 0;
const LEVEL11: u8 = 11;
const LEVEL13: u8 = 13;

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>, symbols: SymbolTable, end: usize) -> Parser {
        let eof = Token { kind: TokenKind::Eof, value: String::new(), position: end };
        let slot_count = symbols.variables.len();
        Parser {
            tokens,
            next: 0,
            current: eof.clone(),
            eof,
            symbols,
            locals: Vec::new(),
            scope_depth: 0,
            loop_depth: 0,
            parsing_break: false,
            break_scopes: Vec::new(),
            slot_count,
            errors: Vec::new(),
            synthesis_error: String::new(),
            side_effect_present: false,
        }
    }

    pub(crate) fn error(&mut self, message: &str) {
        self.errors.push(message.to_string());
    }

    fn next_token(&mut self) {
        if self.next < self.tokens.len() {
            self.current = self.tokens[self.next].clone();
            self.next += 1;
        } else {
            self.current = self.eof.clone();
        }
    }

    fn finished(&self) -> bool {
        self.next >= self.tokens.len()
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.next).unwrap_or(&self.eof)
    }

    fn is(&self, kind: TokenKind) -> bool {
        self.current.kind == kind
    }

    /// `token_is(type)`: advances past a match.
    fn token_is(&mut self, kind: TokenKind) -> bool {
        if self.current.kind != kind {
            return false;
        }
        self.next_token();
        true
    }

    fn is_word(&self, word: &str) -> bool {
        self.current.value.eq_ignore_ascii_case(word)
    }

    /// `lexer().insert_front(type)`: a copy of the next token with another
    /// type, read next.
    fn insert_front(&mut self, kind: TokenKind) {
        if self.next >= self.tokens.len() {
            return;
        }
        let mut token = self.tokens[self.next].clone();
        token.kind = kind;
        self.tokens.insert(self.next, token);
    }

    /// `compile`: the whole expression.
    pub(crate) fn compile(&mut self) -> Option<Node> {
        self.next_token();
        let result = self.parse_corpus();
        if result.is_some() && self.is(TokenKind::Eof) {
            return result;
        }
        if self.errors.is_empty() {
            self.error("ERR003 - Invalid expression encountered");
        }
        None
    }

    fn parse_corpus(&mut self) -> Option<Node> {
        let mut statements = Vec::new();
        let mut side_effects = Vec::new();
        loop {
            self.side_effect_present = false;
            let Some(statement) = self.parse_expression(LEVEL00) else {
                if self.errors.is_empty() {
                    self.error("ERR009 - Invalid expression encountered");
                }
                return None;
            };
            statements.push(statement);
            side_effects.push(self.side_effect_present);
            if self.is(TokenKind::Eof) {
                if self.finished() {
                    break;
                }
                self.next_token();
            }
        }
        self.simplify(statements, side_effects)
    }

    /// `simplify`: drops statements without side effects, except the last.
    fn simplify(&mut self, statements: Vec<Node>, side_effects: Vec<bool>) -> Option<Node> {
        if statements.len() <= 1 {
            return statements.into_iter().next();
        }
        let last_side_effect = side_effects.last().copied().unwrap_or(false);
        let count = statements.len();
        let mut kept = Vec::new();
        let mut stopped = false;
        let mut statements = statements.into_iter();
        for &has_side_effect in side_effects.iter().take(count - 1) {
            let statement = statements.next()?;
            if statement.is_variable() {
                continue;
            }
            if statement.is_break_or_continue() {
                kept.push(statement);
                stopped = true;
                break;
            }
            if statement.is_constant() || statement.is_null() || !has_side_effect {
                continue;
            }
            kept.push(statement);
        }
        if !stopped {
            kept.push(statements.next()?);
        }
        if stopped || last_side_effect || kept.len() > 1 {
            self.side_effect_present = true;
        }
        if kept.len() == 1 {
            return kept.pop();
        }
        self.vararg_function(VarArgOp::Multi, kept)
    }

    /// The operator at the current token: (left level, right level,
    /// operation), `None` to end the expression.
    fn current_operator(&self) -> Option<(u8, u8, Operation)> {
        use TokenKind as K;
        let binary = |left: u8, right: u8, op: BinaryOp| Some((left, right, Operation::Binary(op)));
        match self.current.kind {
            K::Assign => Some((0, 0, Operation::Assign)),
            K::AddAssign => Some((0, 0, Operation::AssignOp(BinaryOp::Add))),
            K::SubAssign => Some((0, 0, Operation::AssignOp(BinaryOp::Sub))),
            K::MulAssign => Some((0, 0, Operation::AssignOp(BinaryOp::Mul))),
            K::DivAssign => Some((0, 0, Operation::AssignOp(BinaryOp::Div))),
            K::ModAssign => Some((0, 0, Operation::AssignOp(BinaryOp::Mod))),
            K::Swap => Some((0, 0, Operation::Swap)),
            K::Lt => binary(5, 6, BinaryOp::Lt),
            K::Lte => binary(5, 6, BinaryOp::Lte),
            K::Eq => binary(5, 6, BinaryOp::Eq),
            K::Ne => binary(5, 6, BinaryOp::Ne),
            K::Gte => binary(5, 6, BinaryOp::Gte),
            K::Gt => binary(5, 6, BinaryOp::Gt),
            K::Add => binary(7, 8, BinaryOp::Add),
            K::Sub => binary(7, 8, BinaryOp::Sub),
            K::Div => binary(10, 11, BinaryOp::Div),
            K::Mul => binary(10, 11, BinaryOp::Mul),
            K::Mod => binary(10, 11, BinaryOp::Mod),
            K::Pow => binary(12, 12, BinaryOp::Pow),
            K::Symbol => {
                let word = self.current.value.to_ascii_lowercase();
                match word.as_str() {
                    "and" => binary(3, 4, BinaryOp::And),
                    "&" => Some((3, 4, Operation::ScAnd)),
                    "nand" => binary(3, 4, BinaryOp::Nand),
                    "or" => binary(1, 2, BinaryOp::Or),
                    "|" => Some((1, 2, Operation::ScOr)),
                    "nor" => binary(1, 2, BinaryOp::Nor),
                    "xor" => binary(1, 2, BinaryOp::Xor),
                    "xnor" => binary(1, 2, BinaryOp::Xnor),
                    "in" | "like" | "ilike" => Some((4, 4, Operation::StringOp)),
                    "not" => Some((0, 0, Operation::Default)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(crate) fn parse_expression(&mut self, precedence: u8) -> Option<Node> {
        let mut expression = self.parse_branch(precedence)?;
        if self.is(TokenKind::Eof) {
            return Some(expression);
        }
        while let Some((left, right, operation)) = self.current_operator() {
            if left < precedence {
                break;
            }
            let previous = self.current.clone();
            self.next_token();
            let right_branch = self.parse_expression(right);
            let new_expression = match right_branch {
                Some(right_branch) => self.binary_node(operation, expression, right_branch),
                None => None,
            };
            let Some(new_expression) = new_expression else {
                if self.errors.is_empty() {
                    let message = if self.synthesis_error.is_empty() {
                        format!("ERR017 - General parsing error at token: '{}'", previous.value)
                    } else {
                        self.synthesis_error.clone()
                    };
                    self.error(&message);
                }
                return None;
            };
            expression = if self.is(TokenKind::Ternary) && precedence == LEVEL00 {
                self.parse_ternary(new_expression)?
            } else {
                new_expression
            };
        }
        Some(expression)
    }

    fn parse_branch(&mut self, precedence: u8) -> Option<Node> {
        use TokenKind as K;
        let branch = match self.current.kind {
            K::Number => {
                let Some(value) = string_to_real(&self.current.value) else {
                    let message = format!("ERR238 - Failed to convert '{}' to a number", self.current.value);
                    self.error(&message);
                    return None;
                };
                self.next_token();
                Node::Constant(value)
            }
            K::Symbol => self.parse_symbol()?,
            K::Str => {
                self.error("ERR - String expressions are not supported");
                return None;
            }
            K::LeftBracket | K::LeftSquare | K::LeftCurly => {
                let (closing, text) = match self.current.kind {
                    K::LeftBracket => (K::RightBracket, "ERR239 - Expected ')'"),
                    K::LeftSquare => (K::RightSquare, "ERR240 - Expected ']'"),
                    _ => (K::RightCurly, "ERR241 - Expected '}'"),
                };
                let round = self.is(K::LeftBracket);
                self.next_token();
                let inner = self.parse_expression(LEVEL00)?;
                if round {
                    self.token_is(K::Eof);
                }
                if !self.token_is(closing) {
                    let message = format!("{text} instead of: '{}'", self.current.value);
                    self.error(&message);
                    return None;
                }
                inner
            }
            K::Sub => {
                self.next_token();
                let operand = self.parse_expression(LEVEL11)?;
                match operand {
                    // -(-x) --> x
                    Node::Unary { op: UnaryOp::Neg, operand } => *operand,
                    other => self.unary_node(UnaryOp::Neg, other)?,
                }
            }
            K::Add => {
                self.next_token();
                self.parse_expression(LEVEL13)?
            }
            K::Eof => {
                self.error("ERR242 - Premature end of expression[1]");
                return None;
            }
            _ => {
                self.error("ERR243 - Premature end of expression[2]");
                return None;
            }
        };
        if precedence == LEVEL00 && self.is(K::Ternary) {
            return self.parse_ternary(branch);
        }
        Some(branch)
    }

    fn parse_ternary(&mut self, condition: Node) -> Option<Node> {
        if !self.token_is(TokenKind::Ternary) {
            self.error("ERR057 - Expected '?' after condition of ternary if-statement");
            return None;
        }
        let Some(consequent) = self.parse_expression(LEVEL00) else {
            self.error("ERR058 - Failed to parse consequent for ternary if-statement");
            return None;
        };
        if !self.token_is(TokenKind::Colon) {
            self.error("ERR059 - Expected ':' between ternary if-statement consequent and alternative");
            return None;
        }
        let Some(alternative) = self.parse_expression(LEVEL00) else {
            self.error("ERR060 - Failed to parse alternative for ternary if-statement");
            return None;
        };
        self.conditional(condition, consequent, Some(alternative))
    }

    /// `check_block_statement_closure`.
    fn check_closure(&mut self, expression: Option<Node>) -> Option<Node> {
        let expression = expression?;
        if self.is(TokenKind::Symbol) || self.is(TokenKind::Number) {
            let message = format!("ERR240 - Invalid syntax '{}' possible missing operator or context", self.current.value);
            self.error(&message);
            return None;
        }
        Some(expression)
    }

    fn parse_symbol(&mut self) -> Option<Node> {
        let symbol = self.current.value.to_ascii_lowercase();
        match symbol.as_str() {
            "sum" | "mul" | "avg" | "min" | "max" | "mand" | "mor" | "~" | "[*]" => return self.parse_vararg_function(),
            "not" => return self.parse_base_operation(),
            _ => {}
        }
        let base_operation = (3..=9).contains(&symbol.len()) && BaseOp::named(&symbol).is_some();
        if base_operation {
            return self.parse_base_operation();
        }
        match symbol.as_str() {
            "if" => self.parse_conditional_statement(),
            "while" => {
                let result = self.parse_while_loop();
                self.check_closure(result)
            }
            "repeat" => {
                let result = self.parse_repeat_until_loop();
                self.check_closure(result)
            }
            "for" => {
                let result = self.parse_for_loop();
                self.check_closure(result)
            }
            "switch" => {
                let result = self.parse_switch_statement();
                self.check_closure(result)
            }
            _ if is_special_function(&symbol) => self.parse_special_function(&symbol),
            "null" => {
                self.next_token();
                Some(Node::Null)
            }
            "break" => self.parse_break_statement(),
            "continue" => self.parse_continue_statement(),
            "var" => self.parse_define_var_statement(),
            "const" => self.parse_define_const_statement(),
            "swap" => self.parse_swap_statement(),
            "return" | "assert" => {
                let message = format!("ERR - '{symbol}' statements are not supported");
                self.error(&message);
                None
            }
            _ => self.parse_symtab_symbol(),
        }
    }

    /// `post_variable_process`: `x(...)` multiplies.
    fn post_variable_process(&mut self) {
        let next = self.peek().kind;
        if matches!(next, TokenKind::LeftBracket | TokenKind::LeftCurly | TokenKind::LeftSquare) {
            self.insert_front(TokenKind::Mul);
        }
    }

    fn parse_symtab_symbol(&mut self) -> Option<Node> {
        let symbol = self.current.value.clone();
        let mut found = self.symbols.variable(&symbol).map(Node::Variable);
        if found.is_none() {
            found = self.symbols.constant(&symbol).map(Node::Constant);
        }
        if let Some(node) = found {
            self.post_variable_process();
            self.next_token();
            return Some(node);
        }
        if let Some(local) = self.active_local(&symbol) {
            let node = match local.constant {
                Some(value) => Node::Constant(value),
                None => Node::Variable(local.slot),
            };
            self.post_variable_process();
            self.next_token();
            return Some(node);
        }
        if is_reserved_symbol(&symbol) {
            let message = format!("ERR231 - Invalid use of reserved symbol '{symbol}'");
            self.error(&message);
            return None;
        }
        let message = format!("ERR234 - Undefined symbol: '{symbol}'");
        self.error(&message);
        None
    }

    fn parse_vararg_function(&mut self) -> Option<Node> {
        let symbol = self.current.value.to_ascii_lowercase();
        let op = match symbol.as_str() {
            "~" => {
                self.next_token();
                let result = self.parse_multi_sequence("", false);
                return self.check_closure(result);
            }
            "[*]" => {
                let result = self.parse_multi_switch_statement();
                return self.check_closure(result);
            }
            "avg" => VarArgOp::Avg,
            "mand" => VarArgOp::Mand,
            "max" => VarArgOp::Max,
            "min" => VarArgOp::Min,
            "mor" => VarArgOp::Mor,
            "mul" => VarArgOp::Mul,
            _ => VarArgOp::Sum,
        };
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            let message = format!("ERR105 - Expected '(' for call to vararg function: {symbol}");
            self.error(&message);
            return None;
        }
        if self.token_is(TokenKind::RightBracket) {
            let message = format!("ERR106 - vararg function: {symbol} requires at least one input parameter");
            self.error(&message);
            return None;
        }
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.parse_expression(LEVEL00)?);
            if self.token_is(TokenKind::RightBracket) {
                break;
            }
            if !self.token_is(TokenKind::Comma) {
                let message = format!("ERR107 - Expected ',' for call to vararg function: {symbol}");
                self.error(&message);
                return None;
            }
        }
        self.vararg_function(op, arguments)
    }

    fn parse_base_operation(&mut self) -> Option<Node> {
        let name = self.current.value.to_ascii_lowercase();
        let operation = BaseOp::named(&name)?;
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            let message = format!("ERR029 - Expected a '(' at start of function call to '{name}', instead got: '{}'", self.current.value);
            self.error(&message);
            return None;
        }
        if self.is(TokenKind::RightBracket) {
            let message = format!("ERR030 - Expected at least one input parameter for function call '{name}'");
            self.error(&message);
            return None;
        }
        let mut parameters = Vec::new();
        let mut closed = false;
        while parameters.len() < 4 {
            parameters.push(self.parse_expression(LEVEL00)?);
            if self.token_is(TokenKind::RightBracket) {
                closed = true;
                break;
            }
            if self.token_is(TokenKind::Comma) {
                continue;
            }
            let message = format!("ERR031 - Expected a ',' between function input parameters, instead got: '{}'", self.current.value);
            self.error(&message);
            return None;
        }
        if !closed {
            let message = format!("ERR032 - Invalid number of input parameters passed to function '{name}'");
            self.error(&message);
            return None;
        }
        if parameters.len() != operation.parameter_count() {
            let message = format!("ERR034 - Invalid number of input parameters for call to function: '{name}'");
            self.error(&message);
            return None;
        }
        let mut parameters = parameters.into_iter();
        let first = parameters.next()?;
        match operation {
            BaseOp::Unary(op) => self.unary_node(op, first),
            BaseOp::Binary(op) => {
                let second = parameters.next()?;
                self.binary_node(Operation::Binary(op), first, second)
            }
            BaseOp::Trinary(op) => {
                let second = parameters.next()?;
                let third = parameters.next()?;
                self.trinary_node(op, [first, second, third])
            }
        }
    }

    fn parse_special_function(&mut self, name: &str) -> Option<Node> {
        let digits = &name.as_bytes()[2..4];
        let index = (digits[0] - b'0') * 10 + (digits[1] - b'0');
        let count = if index < 48 { 3 } else { 4 };
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            let message = format!("ERR146 - Expected '(' for special function '{name}'");
            self.error(&message);
            return None;
        }
        let mut arguments = Vec::new();
        for argument in 0..count {
            arguments.push(self.parse_expression(LEVEL00)?);
            if argument + 1 < count && !self.token_is(TokenKind::Comma) {
                let message = format!("ERR147 - Expected ',' before next parameter of special function '{name}'");
                self.error(&message);
                return None;
            }
        }
        if !self.token_is(TokenKind::RightBracket) {
            let message = format!("ERR148 - Invalid number of parameters for special function '{name}'");
            self.error(&message);
            return None;
        }
        self.special_function(index, arguments)
    }

    /// `parse_multi_sequence`: `{ a; b; c }` (or `( a, b, c )` unless curly
    /// brackets are required).
    fn parse_multi_sequence(&mut self, source: &str, curly_only: bool) -> Option<Node> {
        let (mut close, mut separator) = (TokenKind::RightCurly, TokenKind::Eof);
        if !self.token_is(TokenKind::LeftCurly) {
            if !curly_only && self.token_is(TokenKind::LeftBracket) {
                close = TokenKind::RightBracket;
                separator = TokenKind::Comma;
            } else {
                let section = if source.is_empty() { String::new() } else { format!(" section of {source}") };
                let message = format!("ERR111 - Expected '{{' for call to multi-sequence{section}");
                self.error(&message);
                return None;
            }
        } else if self.token_is(TokenKind::RightCurly) {
            return Some(Node::Null);
        }

        let outer_side_effect = self.side_effect_present;
        self.enter_scope();
        let result = self.parse_sequence_body(close, separator, source);
        self.exit_scope();
        self.side_effect_present = self.side_effect_present || outer_side_effect;
        result
    }

    fn parse_sequence_body(&mut self, close: TokenKind, separator: TokenKind, source: &str) -> Option<Node> {
        let mut statements = Vec::new();
        let mut side_effects = Vec::new();
        loop {
            self.side_effect_present = false;
            statements.push(self.parse_expression(LEVEL00)?);
            side_effects.push(self.side_effect_present);
            if self.token_is(close) {
                break;
            }
            let next_is_close = self.peek().kind == close;
            if !self.token_is(separator) && next_is_close {
                let message = format!("ERR112 - Expected separator for call to multi-sequence section of {source}");
                self.error(&message);
                return None;
            }
            if self.token_is(close) {
                break;
            }
        }
        self.simplify(statements, side_effects)
    }

    fn enter_scope(&mut self) {
        self.scope_depth += 1;
    }

    /// `scope_handler` going out of scope: its locals stop being visible.
    fn exit_scope(&mut self) {
        for local in self.locals.iter_mut() {
            if local.active && local.depth >= self.scope_depth {
                local.active = false;
            }
        }
        self.scope_depth -= 1;
    }

    /// `sem_.get_element(name)`: the shallowest visible element.
    fn local_index(&self, name: &str) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (index, local) in self.locals.iter().enumerate() {
            if local.depth > self.scope_depth || !local.name.eq_ignore_ascii_case(name) {
                continue;
            }
            let better = best.is_none_or(|current| local.depth < self.locals[current].depth);
            if better {
                best = Some(index);
            }
        }
        best
    }

    fn active_local(&self, name: &str) -> Option<&Local> {
        let mut best: Option<&Local> = None;
        for local in &self.locals {
            if local.depth > self.scope_depth || !local.active || !local.name.eq_ignore_ascii_case(name) {
                continue;
            }
            if best.is_none_or(|current| local.depth < current.depth) {
                best = Some(local);
            }
        }
        best
    }

    fn parse_conditional_statement(&mut self) -> Option<Node> {
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            let message = format!("ERR053 - Expected '(' at start of if-statement, instead got: '{}'", self.current.value);
            self.error(&message);
            return None;
        }
        let Some(condition) = self.parse_expression(LEVEL00) else {
            self.error("ERR054 - Failed to parse condition for if-statement");
            return None;
        };
        if self.is(TokenKind::Comma) {
            return self.parse_conditional_01(condition);
        }
        if self.token_is(TokenKind::RightBracket) {
            return self.parse_conditional_02(condition);
        }
        self.error("ERR055 - Invalid if-statement");
        None
    }

    /// `if (x, y, z)`.
    fn parse_conditional_01(&mut self, condition: Node) -> Option<Node> {
        if !self.token_is(TokenKind::Comma) {
            self.error("ERR035 - Expected ',' between if-statement condition and consequent");
            return None;
        }
        let Some(consequent) = self.parse_expression(LEVEL00) else {
            self.error("ERR036 - Failed to parse consequent for if-statement");
            return None;
        };
        if !self.token_is(TokenKind::Comma) {
            self.error("ERR037 - Expected ',' between if-statement consequent and alternative");
            return None;
        }
        let Some(alternative) = self.parse_expression(LEVEL00) else {
            self.error("ERR038 - Failed to parse alternative for if-statement");
            return None;
        };
        if !self.token_is(TokenKind::RightBracket) {
            self.error("ERR039 - Expected ')' at the end of if-statement");
            return None;
        }
        self.conditional(condition, consequent, Some(alternative))
    }

    /// `if (x) y; else z;` and the block forms.
    fn parse_conditional_02(&mut self, condition: Node) -> Option<Node> {
        let consequent = if self.is(TokenKind::LeftCurly) {
            let Some(consequent) = self.parse_multi_sequence("if-statement-01", false) else {
                self.error("ERR043 - Failed to parse body of consequent for if-statement");
                return None;
            };
            consequent
        } else {
            if self.is(TokenKind::Mul) {
                self.next_token();
            }
            let Some(consequent) = self.parse_expression(LEVEL00) else {
                self.error("ERR046 - Failed to parse body of consequent for if-statement");
                return None;
            };
            if !self.is(TokenKind::Eof) {
                self.error("ERR045 - Expected ';' at the end of the consequent for if-statement (2)");
                return None;
            }
            consequent
        };

        let mut alternative = None;
        let else_follows = self.is_word("else") || (self.is(TokenKind::Eof) && self.peek().value.eq_ignore_ascii_case("else"));
        if else_follows {
            self.next_token();
            if self.is_word("else") {
                self.next_token();
            }
            if self.is(TokenKind::LeftCurly) {
                let Some(body) = self.parse_multi_sequence("else-statement-01", false) else {
                    self.error("ERR047 - Failed to parse body of the 'else' for if-statement");
                    return None;
                };
                alternative = Some(body);
            } else if self.is_word("if") {
                let Some(body) = self.parse_conditional_statement() else {
                    self.error("ERR048 - Failed to parse body of if-else statement");
                    return None;
                };
                alternative = Some(body);
            } else {
                let Some(body) = self.parse_expression(LEVEL00) else {
                    self.error("ERR050 - Failed to parse body of the 'else' for if-statement");
                    return None;
                };
                if !self.is(TokenKind::Ternary) && !self.is(TokenKind::RightCurly) && !self.token_is(TokenKind::Eof) {
                    self.error("ERR049 - Expected ';' at the end of the 'else-if' for the if-statement");
                    return None;
                }
                alternative = Some(body);
            }
        }
        self.conditional(condition, consequent, alternative)
    }

    fn parse_while_loop(&mut self) -> Option<Node> {
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            self.error("ERR064 - Expected '(' at start of while-loop condition statement");
            return None;
        }
        let Some(condition) = self.parse_expression(LEVEL00) else {
            self.error("ERR065 - Failed to parse condition for while-loop");
            return None;
        };
        if !self.token_is(TokenKind::RightBracket) {
            self.error("ERR066 - Expected ')' at end of while-loop condition statement");
            return None;
        }
        self.break_scopes.push(false);
        self.loop_depth += 1;
        let body = self.parse_multi_sequence("while-loop", true);
        self.loop_depth -= 1;
        let uses_break = self.break_scopes.pop().unwrap_or(false);
        let Some(body) = body else {
            self.error("ERR067 - Failed to parse body of while-loop");
            return None;
        };
        if let Some(test) = condition.constant()
            && !uses_break {
                if test != 0.0 {
                    self.error("ERR260 - Infinite loop condition without 'break' or 'return' not allowed in while-loops");
                    return None;
                }
                return Some(Node::Null);
            }
        if condition.is_null() {
            return Some(body);
        }
        Some(Node::While { condition: Box::new(condition), body: Box::new(body) })
    }

    fn parse_repeat_until_loop(&mut self) -> Option<Node> {
        self.next_token();
        self.break_scopes.push(false);
        let body = if self.is_word("until") {
            self.next_token();
            Node::Null
        } else {
            let outer_side_effect = self.side_effect_present;
            self.enter_scope();
            self.loop_depth += 1;
            let body = self.parse_repeat_body();
            self.loop_depth -= 1;
            self.exit_scope();
            self.side_effect_present = self.side_effect_present || outer_side_effect;
            body?
        };
        if !self.token_is(TokenKind::LeftBracket) {
            self.error("ERR072 - Expected '(' before condition statement of repeat until loop");
            return None;
        }
        let Some(condition) = self.parse_expression(LEVEL00) else {
            self.error("ERR073 - Failed to parse condition for repeat until loop");
            return None;
        };
        if !self.token_is(TokenKind::RightBracket) {
            self.error("ERR074 - Expected ')' after condition of repeat until loop");
            return None;
        }
        let uses_break = self.break_scopes.pop().unwrap_or(false);
        if condition.is_constant() && !uses_break {
            let finishes = condition.constant().is_some_and(|test| test != 0.0);
            if finishes && body.is_constant() {
                return Some(body);
            }
            self.error("ERR075 - Failed to synthesize repeat until loop");
            return None;
        }
        if condition.is_null() {
            return Some(body);
        }
        Some(Node::Repeat { body: Box::new(body), condition: Box::new(condition) })
    }

    fn parse_repeat_body(&mut self) -> Option<Node> {
        let mut statements = Vec::new();
        let mut side_effects = Vec::new();
        loop {
            self.side_effect_present = false;
            statements.push(self.parse_expression(LEVEL00)?);
            side_effects.push(self.side_effect_present);
            if self.is_word("until") {
                self.next_token();
                break;
            }
            let next_is_until = self.peek().kind == TokenKind::Symbol && self.peek().value.eq_ignore_ascii_case("until");
            if !self.token_is(TokenKind::Eof) && next_is_until {
                self.error("ERR070 - Expected ';' in body of repeat until loop");
                return None;
            }
            if self.is_word("until") {
                self.next_token();
                break;
            }
        }
        let body = self.simplify(statements, side_effects);
        if body.is_none() {
            self.error("ERR071 - Failed to parse body of repeat until loop");
        }
        body
    }

    fn parse_for_loop(&mut self) -> Option<Node> {
        self.next_token();
        self.enter_scope();
        let result = self.parse_for_loop_parts();
        self.exit_scope();
        result
    }

    fn parse_for_loop_parts(&mut self) -> Option<Node> {
        if !self.token_is(TokenKind::LeftBracket) {
            self.error("ERR077 - Expected '(' at start of for-loop");
            return None;
        }
        let mut initialiser = None;
        if !self.token_is(TokenKind::Eof) {
            // ExprTk's own `var` handling here tests `!token_is(symbol) &&
            // current == "var"`, which never holds; `var i := 0` goes through
            // the variable definition statement instead.
            let Some(expression) = self.parse_expression(LEVEL00) else {
                self.error("ERR082 - Failed to parse initialiser of for-loop");
                return None;
            };
            if !self.token_is(TokenKind::Eof) {
                self.error("ERR083 - Expected ';' after initialiser of for-loop");
                return None;
            }
            initialiser = Some(expression);
        }
        let mut condition = None;
        if !self.token_is(TokenKind::Eof) {
            let Some(expression) = self.parse_expression(LEVEL00) else {
                self.error("ERR084 - Failed to parse condition of for-loop");
                return None;
            };
            if !self.token_is(TokenKind::Eof) {
                self.error("ERR085 - Expected ';' after condition section of for-loop");
                return None;
            }
            condition = Some(expression);
        }
        let mut incrementor = None;
        if !self.token_is(TokenKind::RightBracket) {
            let Some(expression) = self.parse_expression(LEVEL00) else {
                self.error("ERR086 - Failed to parse incrementor of for-loop");
                return None;
            };
            if !self.token_is(TokenKind::RightBracket) {
                self.error("ERR087 - Expected ')' after incrementor section of for-loop");
                return None;
            }
            incrementor = Some(expression);
        }
        self.break_scopes.push(false);
        self.loop_depth += 1;
        let body = self.parse_multi_sequence("for-loop", true);
        self.loop_depth -= 1;
        let uses_break = self.break_scopes.pop().unwrap_or(false);
        let Some(body) = body else {
            self.error("ERR088 - Failed to parse body of for-loop");
            return None;
        };
        // for_loop(): a constant condition and no break/continue.
        if let Some(test) = condition.as_ref().and_then(Node::constant)
            && !uses_break {
                if test != 0.0 {
                    self.error("ERR261 - Infinite loop condition without 'break' or 'return' not allowed in for-loop");
                    return None;
                }
                return Some(Node::Null);
            }
        let Some(condition) = condition.filter(|condition| !condition.is_null()) else {
            return Some(body);
        };
        Some(Node::For { initialiser: initialiser.map(Box::new), condition: Box::new(condition), incrementor: incrementor.map(Box::new), body: Box::new(body) })
    }

    fn parse_switch_statement(&mut self) -> Option<Node> {
        self.next_token();
        if !self.token_is(TokenKind::LeftCurly) {
            self.error("ERR091 - Expected '{' for call to switch statement");
            return None;
        }
        let mut cases = Vec::new();
        let mut default = None;
        loop {
            if self.is_word("case") {
                self.next_token();
                let condition = self.parse_expression(LEVEL00)?;
                if !self.token_is(TokenKind::Colon) {
                    self.error("ERR092 - Expected ':' for case of switch statement");
                    return None;
                }
                let consequent = if self.is(TokenKind::LeftCurly) { self.parse_multi_sequence("switch-consequent", false)? } else { self.parse_expression(LEVEL00)? };
                if !self.token_is(TokenKind::Eof) {
                    self.error("ERR093 - Expected ';' at end of case for switch statement");
                    return None;
                }
                let never = condition.constant().is_some_and(|test| test == 0.0);
                if !never {
                    cases.push((condition, consequent));
                }
            } else if self.is_word("default") {
                if default.is_some() {
                    self.error("ERR094 - Multiple default cases for switch statement");
                    return None;
                }
                self.next_token();
                if !self.token_is(TokenKind::Colon) {
                    self.error("ERR095 - Expected ':' for default of switch statement");
                    return None;
                }
                let statement = if self.is(TokenKind::LeftCurly) { self.parse_multi_sequence("switch-default", false)? } else { self.parse_expression(LEVEL00)? };
                if !self.token_is(TokenKind::Eof) {
                    self.error("ERR096 - Expected ';' at end of default for switch statement");
                    return None;
                }
                default = Some(statement);
            } else if self.token_is(TokenKind::RightCurly) {
                break;
            } else {
                self.error("ERR097 - Expected '}' at end of switch statement");
                return None;
            }
        }
        let has_default = default.is_some();
        let default = default.unwrap_or(Node::Constant(f32::NAN));
        if cases.is_empty() && !has_default {
            return None;
        }
        // const_optimise_switch: every node constant.
        let all_constant = cases.iter().all(|(condition, consequent)| condition.is_constant() && consequent.is_constant()) && default.is_constant();
        if all_constant {
            for (condition, consequent) in cases {
                if condition.constant().is_some_and(|test| test != 0.0) {
                    return Some(consequent);
                }
            }
            return Some(default);
        }
        Some(Node::Switch { cases, default: Box::new(default) })
    }

    fn parse_multi_switch_statement(&mut self) -> Option<Node> {
        self.next_token();
        if !self.token_is(TokenKind::LeftCurly) {
            self.error("ERR099 - Expected '{' for call to [*] statement");
            return None;
        }
        let mut cases = Vec::new();
        loop {
            if !self.is_word("case") {
                self.error("ERR100 - Expected a 'case' statement for multi-switch");
                return None;
            }
            self.next_token();
            let condition = self.parse_expression(LEVEL00)?;
            if !self.token_is(TokenKind::Colon) {
                self.error("ERR101 - Expected ':' for case of [*] statement");
                return None;
            }
            let consequent = if self.is(TokenKind::LeftCurly) { self.parse_multi_sequence("multi-switch-consequent", false)? } else { self.parse_expression(LEVEL00)? };
            if !self.token_is(TokenKind::Eof) {
                self.error("ERR102 - Expected ';' at end of case for [*] statement");
                return None;
            }
            let never = condition.constant().is_some_and(|test| test == 0.0);
            if !never {
                cases.push((condition, consequent));
            }
            if self.is(TokenKind::RightCurly) {
                break;
            }
        }
        if !self.token_is(TokenKind::RightCurly) {
            self.error("ERR103 - Expected '}' at end of [*] statement");
            return None;
        }
        // const_optimise_mswitch: the last true case, else 0.
        let all_constant = cases.iter().all(|(condition, consequent)| condition.is_constant() && consequent.is_constant());
        if all_constant {
            let mut result = Node::Constant(0.0);
            for (condition, consequent) in cases {
                if condition.constant().is_some_and(|test| test != 0.0) {
                    result = consequent;
                }
            }
            return Some(result);
        }
        Some(Node::MultiSwitch { cases })
    }

    fn parse_break_statement(&mut self) -> Option<Node> {
        if self.parsing_break {
            self.error("ERR151 - Invoking 'break' within a break call is not allowed");
            return None;
        }
        if self.loop_depth == 0 || self.break_scopes.is_empty() {
            self.error("ERR152 - Invalid use of 'break', allowed only in the scope of a loop");
            return None;
        }
        self.parsing_break = true;
        self.next_token();
        if let Some(scope) = self.break_scopes.last_mut() {
            *scope = true;
        }
        let mut value = None;
        if self.token_is(TokenKind::LeftSquare) {
            let Some(expression) = self.parse_expression(LEVEL00) else {
                self.parsing_break = false;
                self.error("ERR153 - Failed to parse return expression for 'break' statement");
                return None;
            };
            if !self.token_is(TokenKind::RightSquare) {
                self.parsing_break = false;
                self.error("ERR154 - Expected ']' at the completion of break's return expression");
                return None;
            }
            value = Some(Box::new(expression));
        }
        self.parsing_break = false;
        self.side_effect_present = true;
        Some(Node::Break(value))
    }

    fn parse_continue_statement(&mut self) -> Option<Node> {
        if self.loop_depth == 0 {
            self.error("ERR156 - Invalid use of 'continue', allowed only in the scope of a loop");
            return None;
        }
        self.next_token();
        if let Some(scope) = self.break_scopes.last_mut() {
            *scope = true;
        }
        self.side_effect_present = true;
        Some(Node::Continue)
    }

    /// Adds (or reactivates) a local in the current scope; its slot.
    fn define_local(&mut self, name: &str, constant: Option<f32>) -> usize {
        if let Some(index) = self.local_index(name) {
            let local = &mut self.locals[index];
            if !local.active && local.constant.is_none() == constant.is_none() {
                local.active = true;
                local.depth = self.scope_depth;
                local.constant = constant;
                return local.slot;
            }
        }
        let slot = self.slot_count;
        self.slot_count += 1;
        let local = Local { name: name.to_string(), slot, depth: self.scope_depth, active: true, constant };
        self.locals.push(local);
        slot
    }

    fn check_new_symbol(&mut self, name: &str, kind: &str) -> bool {
        if is_reserved_symbol(name) {
            let message = format!("ERR179 - Illegal redefinition of reserved keyword: '{name}'");
            self.error(&message);
            return false;
        }
        if self.symbols.exists(name) {
            let message = format!("ERR180 - Illegal redefinition of {kind} '{name}'");
            self.error(&message);
            return false;
        }
        let shadowed = self.local_index(name).is_some_and(|index| self.locals[index].active && self.locals[index].name == name);
        if shadowed {
            let message = format!("ERR181 - Illegal redefinition of local variable: '{name}'");
            self.error(&message);
            return false;
        }
        true
    }

    fn parse_define_var_statement(&mut self) -> Option<Node> {
        self.next_token();
        let name = self.current.value.clone();
        if !self.token_is(TokenKind::Symbol) {
            self.error("ERR178 - Expected a symbol for variable definition");
            return None;
        }
        if !self.check_new_symbol(&name, "variable") {
            return None;
        }
        if self.is(TokenKind::LeftSquare) {
            self.error("ERR - Vector definitions are not supported");
            return None;
        }
        let mut initialiser = None;
        if self.is(TokenKind::LeftCurly) {
            // `var x{};`
            let empty = self.token_is(TokenKind::LeftCurly) && self.token_is(TokenKind::RightCurly);
            if !empty {
                self.error("ERR200 - Expected a '{}' for uninitialised var definition");
                return None;
            }
            if !self.is(TokenKind::Eof) {
                self.error("ERR201 - Expected ';' after uninitialised variable definition");
                return None;
            }
            self.define_local(&name, None);
            self.side_effect_present = true;
            return Some(Node::Constant(0.0));
        }
        if self.token_is(TokenKind::Assign) {
            let Some(expression) = self.parse_expression(LEVEL00) else {
                let message = format!("ERR182 - Failed to parse initialisation expression for variable '{name}'");
                self.error(&message);
                return None;
            };
            initialiser = Some(expression);
        }
        let closes = self.is(TokenKind::RightBracket) || self.is(TokenKind::RightCurly) || self.is(TokenKind::RightSquare);
        if !closes && !self.is(TokenKind::Eof) {
            let message = format!("ERR183 - Expected ';' after variable '{name}' definition");
            self.error(&message);
            return None;
        }
        let slot = self.define_local(&name, None);
        self.side_effect_present = true;
        let value = initialiser.unwrap_or(Node::Constant(0.0));
        self.binary_node(Operation::Assign, Node::Variable(slot), value)
    }

    fn parse_define_const_statement(&mut self) -> Option<Node> {
        self.next_token();
        if !self.is_word("var") {
            self.error("ERR189 - Expected 'var' keyword for const-variable definition");
            return None;
        }
        self.next_token();
        let name = self.current.value.clone();
        if !self.token_is(TokenKind::Symbol) {
            self.error("ERR190 - Expected a symbol for const-variable definition");
            return None;
        }
        if !self.check_new_symbol(&name, "variable") {
            return None;
        }
        if !self.token_is(TokenKind::Assign) {
            let message = format!("ERR194 - Expected assignment operator after const-variable: '{name}' definition");
            self.error(&message);
            return None;
        }
        let Some(initialiser) = self.parse_expression(LEVEL00) else {
            let message = format!("ERR195 - Failed to parse initialisation expression for const-variable: '{name}'");
            self.error(&message);
            return None;
        };
        let Some(value) = initialiser.constant() else {
            let message = format!("ERR196 - initialisation expression for const-variable: '{name}' must be a constant/literal");
            self.error(&message);
            return None;
        };
        self.define_local(&name, Some(value));
        self.side_effect_present = true;
        Some(Node::Constant(value))
    }

    fn parse_swap_statement(&mut self) -> Option<Node> {
        self.next_token();
        if !self.token_is(TokenKind::LeftBracket) {
            self.error("ERR205 - Expected '(' at start of swap statement");
            return None;
        }
        let first = self.swap_variable()?;
        if !self.token_is(TokenKind::Comma) {
            self.error("ERR210 - Expected ',' between parameters to swap");
            return None;
        }
        let second = self.swap_variable()?;
        if !self.token_is(TokenKind::RightBracket) {
            self.error("ERR214 - Expected ')' at end of swap statement");
            return None;
        }
        self.side_effect_present = true;
        Some(Node::Swap { first, second })
    }

    fn swap_variable(&mut self) -> Option<usize> {
        let name = self.current.value.clone();
        if !self.is(TokenKind::Symbol) {
            self.error("ERR206 - Expected a symbol for variable or vector element definition");
            return None;
        }
        let slot = self.symbols.variable(&name).or_else(|| self.active_local(&name).filter(|local| local.constant.is_none()).map(|local| local.slot));
        let Some(slot) = slot else {
            let message = format!("ERR209 - First parameter to swap is an invalid variable: '{name}'");
            self.error(&message);
            return None;
        };
        self.next_token();
        Some(slot)
    }
}

/// `is_valid_sf_symbol`: `$fNN`.
fn is_special_function(symbol: &str) -> bool {
    let bytes = symbol.as_bytes();
    bytes.len() == 4 && bytes[0] == b'$' && bytes[1].eq_ignore_ascii_case(&b'f') && bytes[2].is_ascii_digit() && bytes[3].is_ascii_digit()
}
