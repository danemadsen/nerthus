//! ExprTk's `expression_generator`: how the parser turns an operator and
//! its operand nodes into a node. It folds constants and rewrites operator
//! patterns into specialised nodes (`(c0 + v) + c1` into `(c0 + c1) + v`,
//! `(v0 / v1) / v2` into `v0 / (v1 * v2)`, `x ^ 3` into `x * x * x`, …).
//! Those rewrites change float rounding, so this follows ExprTk 2024-01-01
//! (the release closest to the ported snapshot) rule for rule.

use crate::exprtk::functions::{self, sf3_lookup, sf4_lookup, SF3_PATTERNS};
use crate::exprtk::node::{leaf_letters, BinaryOp, BranchKind, FunctionOp, Leaf, Node, UnaryOp, VarArgOp};
use crate::exprtk::parser::Parser;

/// An operation the parser hands the generator (`details::operator_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Binary(BinaryOp),
    Assign,
    /// `+=`, `-=`, `*=`, `/=`, `%=`.
    AssignOp(BinaryOp),
    Swap,
    /// `&`.
    ScAnd,
    /// `|`.
    ScOr,
    /// `in`, `like`, `ilike`: string operators, which fail on numbers.
    StringOp,
    /// What the parser passes for a `not` between operands.
    Default,
}

impl Operation {
    /// `details::to_str(operation)` for diagnostics.
    pub fn text(self) -> &'static str {
        match self {
            Operation::Binary(op) => op.symbol(),
            _ => "UNKNOWN",
        }
    }
}

/// The 4-leaf patterns ExprTk registers T4 synthesizers for.
const T4_TYPES: [&str; 9] = ["vvvv", "vvvc", "vvcv", "vcvv", "cvvv", "cvcv", "vcvc", "cvvc", "vccv"];

/// `is_sf3ext_node` followed by the `dynamic_cast` to `sf3ext_type_node`:
/// an sf3ext node over `vovov`, `vovoc`, `vocov`, `covov` or `covoc`.
fn sf3ext_parts(node: &Node) -> Option<(u8, [Leaf; 3])> {
    let Node::Sf3Ext { index, leaves } = node else {
        return None;
    };
    let letters = leaf_letters(leaves);
    let typed = matches!(letters.as_str(), "vvv" | "vvc" | "vcv" | "cvv" | "cvc");
    typed.then_some((*index, *leaves))
}

/// `synthesize_sf3ext_expression::compile`.
fn sf3ext(id: &str, leaves: [Leaf; 3]) -> Option<Node> {
    let index = sf3_lookup(id)?;
    Some(Node::Sf3Ext { index, leaves })
}

/// `synthesize_sf4ext_expression::compile`.
fn sf4ext(id: &str, leaves: [Leaf; 4]) -> Option<Node> {
    let index = sf4_lookup(id)?;
    Some(Node::Sf4Ext { index, leaves })
}

/// `cardinal_pow_optimisable`: `^` by a whole number up to 60.
fn cardinal_pow_optimisable(op: BinaryOp, c: f32) -> bool {
    op == BinaryOp::Pow && functions::abs(c) <= 60.0 && c % 1.0 == 0.0
}

fn boxed(node: Node) -> Box<Node> {
    Box::new(node)
}

fn binary(op: BinaryOp, left: Node, right: Node, kind: BranchKind) -> Node {
    Node::Binary { op, left: boxed(left), right: boxed(right), kind }
}

/// Which registered synthesizer `synthesize_map_` holds for two branches.
enum Mapped {
    Vov(usize, usize),
    Cov(f32, usize),
    Voc(usize, f32),
    /// `(a o0 b) o1 x`.
    Expression0 { pair: (BinaryOp, Leaf, Leaf), leaf: Leaf },
    /// `x o0 (a o1 b)`.
    Expression1 { leaf: Leaf, pair: (BinaryOp, Leaf, Leaf) },
    /// `(a o0 b) o1 (c o2 d)`.
    Quad0 { left: (BinaryOp, Leaf, Leaf), right: (BinaryOp, Leaf, Leaf) },
    /// Families 1 to 4: a leaf and a plain T0oT1oT2 node.
    QuadN { family: u8, leaf: Leaf, leaves: [Leaf; 3], ops: [BinaryOp; 2] },
}

fn as_pair(node: &Node) -> Option<(BinaryOp, Leaf, Leaf)> {
    match node {
        Node::Pair { op, left, right } => Some((*op, *left, *right)),
        _ => None,
    }
}

fn as_triple(node: &Node) -> Option<([Leaf; 3], [BinaryOp; 2], u8)> {
    match node {
        Node::Triple { leaves, ops, mode } => Some((*leaves, *ops, *mode)),
        _ => None,
    }
}

/// `synthesize_expression(operation, branch, result)`: the map lookup by
/// `branch_to_id`.
fn mapped(left: &Node, right: &Node) -> Option<Mapped> {
    match (left, right) {
        (Node::Variable(v0), Node::Variable(v1)) => return Some(Mapped::Vov(*v0, *v1)),
        (Node::Constant(c), Node::Variable(v)) => return Some(Mapped::Cov(*c, *v)),
        (Node::Variable(v), Node::Constant(c)) => return Some(Mapped::Voc(*v, *c)),
        _ => {}
    }
    if let (Some(pair), Some(leaf)) = (as_pair(left), right.as_leaf()) {
        return Some(Mapped::Expression0 { pair, leaf });
    }
    if let (Some(leaf), Some(pair)) = (left.as_leaf(), as_pair(right)) {
        return Some(Mapped::Expression1 { leaf, pair });
    }
    if let (Some(left_pair), Some(right_pair)) = (as_pair(left), as_pair(right)) {
        return Some(Mapped::Quad0 { left: left_pair, right: right_pair });
    }
    let (leaf, (leaves, ops, mode), leaf_first) = if let (Some(leaf), Some(triple)) = (left.as_leaf(), as_triple(right)) {
        (leaf, triple, true)
    } else if let (Some(triple), Some(leaf)) = (as_triple(left), right.as_leaf()) {
        (leaf, triple, false)
    } else {
        return None;
    };
    let letters = if leaf_first {
        format!("{}{}", leaf.letter(), leaf_letters(&leaves))
    } else {
        format!("{}{}", leaf_letters(&leaves), leaf.letter())
    };
    let family = match (leaf_first, mode) {
        (true, 1) => 1,
        (true, _) => 2,
        (false, 0) => 3,
        (false, _) => 4,
    };
    // Families 2 and 4 have no `vococov` synthesizer.
    let registered = T4_TYPES.contains(&letters.as_str()) && !(letters == "vccv" && (family == 2 || family == 4));
    if !registered {
        return None;
    }
    Some(Mapped::QuadN { family, leaf, leaves, ops })
}

impl Parser {
    /// `operator()(operation, branch[1])`.
    pub(crate) fn unary_node(&mut self, op: UnaryOp, branch: Node) -> Option<Node> {
        if branch.is_null() {
            return Some(branch);
        }
        if branch.is_break_or_continue() {
            let message = if matches!(branch, Node::Break(_)) { "ERR249 - Invalid use of break statement" } else { "ERR250 - Invalid use of continue statement" };
            self.error(message);
            return None;
        }
        if let Some(value) = branch.constant() {
            return Some(Node::Constant(op.apply(value)));
        }
        Some(Node::Unary { op, operand: boxed(branch) })
    }

    /// `operator()(operation, branch[2])`.
    pub(crate) fn binary_node(&mut self, operation: Operation, left: Node, right: Node) -> Option<Node> {
        let invalid_assignment = matches!(operation, Operation::AssignOp(_)) && !left.is_variable();
        if invalid_assignment {
            self.error(&format!("ERR251 - Invalid branch pair for assignment operator '{}'", operation.text()));
            return None;
        }
        if left.is_break_or_continue() || right.is_break_or_continue() {
            self.error(&format!("ERR252 - Invalid branch pair for break/continue operator '{}'", operation.text()));
            return None;
        }
        let op = match operation {
            Operation::Assign => return self.assignment(left, right),
            Operation::Swap => return self.swap(left, right),
            Operation::AssignOp(op) => {
                let slot = left.variable()?;
                self.side_effect_present = true;
                return Some(Node::Assign { slot, op: Some(op), value: boxed(right) });
            }
            Operation::ScAnd | Operation::ScOr => return Some(short_circuit(operation == Operation::ScOr, left, right)),
            Operation::StringOp | Operation::Default => return None,
            Operation::Binary(op) => op,
        };

        if left.is_null() || right.is_null() {
            return Some(null_expression(op, left, right));
        }

        // is_constpow_operation: a branch to a whole power.
        if let Some(c) = right.constant() {
            let branch_base = !left.is_constant() && !left.is_variable();
            if branch_base && cardinal_pow_optimisable(op, c) {
                return Some(branch_power(left, c));
            }
        }

        if op.is_optimisable()
            && let Some(found) = mapped(&left, &right) {
                return self.run_mapped(op, found);
            }

        let both_kinds = (left.is_constant(), right.is_constant());
        let simple_op = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div);
        let cocob = simple_op && ((both_kinds.0 && right.is_cob()) || (both_kinds.1 && left.is_cob()));
        let coboc = simple_op && ((both_kinds.0 && right.is_boc()) || (both_kinds.1 && left.is_boc()));
        if cocob {
            if let Some(result) = cocob_expression(op, &left, &right) {
                return Some(result);
            }
        } else if coboc
            && let Some(result) = coboc_expression(op, &left, &right) {
                return Some(result);
            }

        if op.is_optimisable() {
            if let (Some(u0), Some(u1)) = (left.as_uv(), right.as_uv()) {
                return self.uvouv(op, u0, u1);
            }
            if left.is_variable() && !right.is_variable() {
                return self.vob(op, left, right);
            }
            if !left.is_variable() && right.is_variable() {
                return self.bov(op, left, right);
            }
            if left.is_constant() && !right.is_constant() {
                return self.cob(op, left, right);
            }
            if !left.is_constant() && right.is_constant() {
                return self.boc(op, left, right);
            }
            if !left.is_constant() || !right.is_constant() {
                return self.binary_ext(op, left, right);
            }
        }

        // synthesize_expression<binary_node_t, 2>: folds constants.
        if let (Some(c0), Some(c1)) = (left.constant(), right.constant()) {
            return Some(Node::Constant(op.apply(c0, c1)));
        }
        Some(binary(op, left, right, BranchKind::Plain))
    }

    fn assignment(&mut self, left: Node, right: Node) -> Option<Node> {
        if let Some(slot) = left.variable() {
            self.side_effect_present = true;
            return Some(Node::Assign { slot, op: None, value: boxed(right) });
        }
        if left.is_constant() {
            self.error("ERR274 - Cannot assign value to const variable");
        } else {
            self.error("ERR275 - Invalid branches for assignment operator ':='");
        }
        None
    }

    fn swap(&mut self, left: Node, right: Node) -> Option<Node> {
        match (left.variable(), right.variable()) {
            (Some(first), Some(second)) => {
                self.side_effect_present = true;
                Some(Node::Swap { first, second })
            }
            _ => {
                self.synthesis_error = "Invalid branches for swap".to_string();
                None
            }
        }
    }

    fn run_mapped(&mut self, op: BinaryOp, found: Mapped) -> Option<Node> {
        match found {
            Mapped::Vov(v0, v1) => Some(Node::Pair { op, left: Leaf::Variable(v0), right: Leaf::Variable(v1) }),
            Mapped::Cov(c, v) => Some(cov(op, c, v)),
            Mapped::Voc(v, c) => Some(voc(op, v, c)),
            Mapped::Expression0 { pair, leaf } => Some(expression0(pair, op, leaf)),
            Mapped::Expression1 { leaf, pair } => Some(expression1(leaf, op, pair)),
            Mapped::Quad0 { left, right } => Some(quad0(left, op, right)),
            Mapped::QuadN { family, leaf, leaves, ops } => Some(quad_family(family, leaf, leaves, ops, op)),
        }
    }

    /// `synthesize_uvouv_expression`.
    fn uvouv(&mut self, op: BinaryOp, u0: (UnaryOp, usize), u1: (UnaryOp, usize)) -> Option<Node> {
        if !u0.0.in_unary_op_map() || !u1.0.in_unary_op_map() {
            return None;
        }
        let (o0, v0) = u0;
        let (o1, v1) = u1;
        if o0 == UnaryOp::Neg && o1 == UnaryOp::Neg {
            let pair = |op: BinaryOp, a: usize, b: usize| Node::Pair { op, left: Leaf::Variable(a), right: Leaf::Variable(b) };
            match op {
                // (-v0 + -v1) --> -(v0 + v1)
                BinaryOp::Add => return self.unary_node(UnaryOp::Neg, pair(BinaryOp::Add, v0, v1)),
                // (-v0 - -v1) --> (v1 - v0)
                BinaryOp::Sub => return Some(pair(BinaryOp::Sub, v1, v0)),
                // (-v0 * -v1) --> (v0 * v1)
                BinaryOp::Mul => return Some(pair(BinaryOp::Mul, v0, v1)),
                // (-v0 / -v1) --> (v0 / v1)
                BinaryOp::Div => return Some(pair(BinaryOp::Div, v0, v1)),
                _ => {}
            }
        }
        Some(Node::Uvouv { op, left: u0, right: u1 })
    }

    /// `synthesize_vob_expression`.
    fn vob(&mut self, op: BinaryOp, left: Node, right: Node) -> Option<Node> {
        let v = left.variable()?;
        if let Some(result) = compile_right(Leaf::Variable(v), op, &right) {
            return Some(result);
        }
        if matches!(op, BinaryOp::Mul | BinaryOp::Div)
            && let Some((UnaryOp::Neg, v1)) = right.as_uv() {
                let pair = Node::Pair { op, left: Leaf::Variable(v), right: Leaf::Variable(v1) };
                return self.unary_node(UnaryOp::Neg, pair);
            }
        Some(binary(op, left, right, BranchKind::Vob))
    }

    /// `synthesize_bov_expression`.
    fn bov(&mut self, op: BinaryOp, left: Node, right: Node) -> Option<Node> {
        let v = right.variable()?;
        if let Some(result) = compile_left(&left, op, Leaf::Variable(v)) {
            return Some(result);
        }
        if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div)
            && let Some((UnaryOp::Neg, v0)) = left.as_uv() {
                let pair = |op: BinaryOp, a: usize, b: usize| Node::Pair { op, left: Leaf::Variable(a), right: Leaf::Variable(b) };
                return match op {
                    // -v0 + v --> v - v0
                    BinaryOp::Add => Some(pair(BinaryOp::Sub, v, v0)),
                    BinaryOp::Sub => self.unary_node(UnaryOp::Neg, pair(BinaryOp::Add, v0, v)),
                    BinaryOp::Mul => self.unary_node(UnaryOp::Neg, pair(BinaryOp::Mul, v0, v)),
                    _ => self.unary_node(UnaryOp::Neg, pair(BinaryOp::Div, v0, v)),
                };
            }
        Some(binary(op, left, right, BranchKind::Bov))
    }

    /// `synthesize_cob_expression`.
    fn cob(&mut self, op: BinaryOp, left: Node, right: Node) -> Option<Node> {
        let c = left.constant()?;
        if c == 0.0 && (op == BinaryOp::Mul || op == BinaryOp::Div) {
            return Some(Node::Constant(0.0));
        }
        if (c == 0.0 && op == BinaryOp::Add) || (c == 1.0 && op == BinaryOp::Mul) {
            return Some(right);
        }
        if let Node::Binary { op: cob_op, left: cob_c, right: inner, kind: BranchKind::Cob } = &right {
            let c1 = cob_c.constant()?;
            let rebuilt = |new_op: BinaryOp, new_c: f32| binary(new_op, Node::Constant(new_c), (**inner).clone(), BranchKind::Cob);
            if (op == BinaryOp::Mul || op == BinaryOp::Add) && op == *cob_op {
                let combined = if op == BinaryOp::Add { c + c1 } else { c * c1 };
                return Some(rebuilt(*cob_op, combined));
            }
            if op == BinaryOp::Mul {
                match cob_op {
                    // c * (c1 / b) --> (c * c1) / b
                    BinaryOp::Div => return Some(rebuilt(BinaryOp::Div, c * c1)),
                    BinaryOp::Mul => return Some(rebuilt(BinaryOp::Mul, c1 / c)),
                    _ => {}
                }
            } else if op == BinaryOp::Div {
                match cob_op {
                    // c / (c1 / b) --> (c / c1) * b
                    BinaryOp::Div => return Some(rebuilt(BinaryOp::Mul, c / c1)),
                    // c / (c1 * b) --> (c / c1) / b
                    BinaryOp::Mul => return Some(rebuilt(BinaryOp::Div, c / c1)),
                    _ => {}
                }
            }
        } else if let Some(result) = compile_right(Leaf::Constant(c), op, &right) {
            return Some(result);
        }
        Some(binary(op, Node::Constant(c), right, BranchKind::Cob))
    }

    /// `synthesize_boc_expression`.
    fn boc(&mut self, op: BinaryOp, left: Node, right: Node) -> Option<Node> {
        let c = right.constant()?;
        if c == 0.0 && op == BinaryOp::Mul {
            return Some(Node::Constant(0.0));
        }
        if c == 0.0 && op == BinaryOp::Div {
            return Some(Node::Constant(f32::NAN));
        }
        if (c == 0.0 && op == BinaryOp::Add) || (c == 1.0 && op == BinaryOp::Mul) {
            return Some(left);
        }
        if let Node::Binary { op: boc_op, left: inner, right: boc_c, kind: BranchKind::Boc } = &left {
            let c1 = boc_c.constant()?;
            let rebuilt = |new_c: f32| binary(*boc_op, (**inner).clone(), Node::Constant(new_c), BranchKind::Boc);
            if op == BinaryOp::Mul || op == BinaryOp::Add {
                if op == *boc_op {
                    let combined = if op == BinaryOp::Add { c + c1 } else { c * c1 };
                    return Some(rebuilt(combined));
                }
            } else if op == BinaryOp::Div {
                match boc_op {
                    // (b / c1) / c --> b / (c * c1)
                    BinaryOp::Div => return Some(rebuilt(c * c1)),
                    // (b * c1) / c --> b * (c1 / c)
                    BinaryOp::Mul => return Some(rebuilt(c1 / c)),
                    _ => {}
                }
            } else if op == BinaryOp::Pow && *boc_op == BinaryOp::Pow {
                // (v ^ c0) ^ c1 --> v ^ (c0 * c1)
                return Some(rebuilt(c1 * c));
            }
        }
        if let Some(result) = compile_left(&left, op, Leaf::Constant(c)) {
            return Some(result);
        }
        Some(binary(op, left, Node::Constant(c), BranchKind::Boc))
    }

    /// `synthesize_binary_ext_expression`.
    fn binary_ext(&mut self, op: BinaryOp, left: Node, right: Node) -> Option<Node> {
        let simple_op = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div);
        let strip = |node: Node| match node {
            Node::Unary { op: UnaryOp::Neg, operand } => *operand,
            other => other,
        };
        let left_neg = left.is_negation();
        let right_neg = right.is_negation();
        let ext = |op: BinaryOp, a: Node, b: Node| binary(op, a, b, BranchKind::Ext);
        if left_neg && right_neg {
            if !simple_op {
                return Some(ext(op, left, right));
            }
            let left = strip(left);
            let right = strip(right);
            return match op {
                // -f + -g --> -(f + g)
                BinaryOp::Add => self.unary_node(UnaryOp::Neg, ext(BinaryOp::Add, left, right)),
                // -f - -g --> g - f
                BinaryOp::Sub => Some(ext(BinaryOp::Sub, right, left)),
                _ => Some(ext(op, left, right)),
            };
        }
        if left_neg && simple_op {
            let left = strip(left);
            return match op {
                // -f + g --> g - f
                BinaryOp::Add => Some(ext(BinaryOp::Sub, right, left)),
                // -f - g --> -(f + g)
                BinaryOp::Sub => self.unary_node(UnaryOp::Neg, ext(BinaryOp::Add, left, right)),
                _ => self.unary_node(UnaryOp::Neg, ext(op, left, right)),
            };
        }
        if right_neg && simple_op {
            let right = strip(right);
            return match op {
                // f + -g --> f - g
                BinaryOp::Add => Some(ext(BinaryOp::Sub, left, right)),
                // f - -g --> f + g
                BinaryOp::Sub => Some(ext(BinaryOp::Add, left, right)),
                _ => self.unary_node(UnaryOp::Neg, ext(op, left, right)),
            };
        }
        Some(ext(op, left, right))
    }

    /// `operator()(operation, branch[3])`: clamp, iclamp, inrange.
    pub(crate) fn trinary_node(&mut self, op: FunctionOp, arguments: [Node; 3]) -> Option<Node> {
        if let (Some(lower), Some(value), Some(upper)) = (arguments[0].constant(), arguments[1].constant(), arguments[2].constant()) {
            return Some(Node::Constant(functions::function(op, lower, value, upper)));
        }
        Some(Node::Trinary { op, arguments: Box::new(arguments) })
    }

    /// `special_function`: `$fNN(...)`.
    pub(crate) fn special_function(&mut self, index: u8, arguments: Vec<Node>) -> Option<Node> {
        if arguments.iter().all(Node::is_constant) {
            let mut values = [0.0f32; 4];
            for (target, argument) in values.iter_mut().zip(&arguments) {
                *target = argument.constant().unwrap_or(0.0);
            }
            return Some(Node::Constant(functions::special(index, values)));
        }
        Some(Node::Special { index, arguments })
    }

    /// `vararg_function`.
    pub(crate) fn vararg_function(&mut self, op: VarArgOp, mut arguments: Vec<Node>) -> Option<Node> {
        if arguments.iter().all(Node::is_constant) {
            let values: Vec<f32> = arguments.iter().filter_map(Node::constant).collect();
            return Some(Node::Constant(functions::vararg_values(op, &values)));
        }
        let single = matches!(op, VarArgOp::Sum | VarArgOp::Mul | VarArgOp::Avg | VarArgOp::Min | VarArgOp::Max);
        if arguments.len() == 1 && single {
            return arguments.pop();
        }
        Some(Node::VarArg { op, arguments })
    }

    /// `conditional`.
    pub(crate) fn conditional(&mut self, condition: Node, consequent: Node, alternative: Option<Node>) -> Option<Node> {
        if let Some(test) = condition.constant() {
            if test != 0.0 {
                return Some(consequent);
            }
            return Some(alternative.unwrap_or(Node::Null));
        }
        Some(Node::Conditional { condition: boxed(condition), consequent: boxed(consequent), alternative: alternative.map(boxed) })
    }
}

/// `synthesize_shortcircuit_expression`.
fn short_circuit(or: bool, left: Node, right: Node) -> Node {
    for branch in [&left, &right] {
        if let Some(value) = branch.constant() {
            if !or && value == 0.0 {
                return Node::Constant(0.0);
            }
            if or && value != 0.0 {
                return Node::Constant(1.0);
            }
        }
    }
    if let (Some(a), Some(b)) = (left.constant(), right.constant()) {
        let truth = if or { a != 0.0 || b != 0.0 } else { a != 0.0 && b != 0.0 };
        return Node::Constant(if truth { 1.0 } else { 0.0 });
    }
    Node::ShortCircuit { or, left: boxed(left), right: boxed(right) }
}

/// `synthesize_null_expression`.
fn null_expression(op: BinaryOp, left: Node, right: Node) -> Node {
    let left_null = left.is_null();
    let right_null = right.is_null();
    if left_null && right_null {
        return match op {
            BinaryOp::Eq => Node::Constant(1.0),
            BinaryOp::Ne => Node::Constant(0.0),
            _ => left,
        };
    }
    // ExprTk keeps the null branch itself here, so `x == null` is always 1
    // and `x != null` always 0.
    match op {
        BinaryOp::Eq => return Node::NullEquality { operand: boxed(Node::Null), equality: true },
        BinaryOp::Ne => return Node::NullEquality { operand: boxed(Node::Null), equality: false },
        _ => {}
    }
    let kept = if left_null { right } else { left };
    match op {
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => kept,
        BinaryOp::Lt
        | BinaryOp::Lte
        | BinaryOp::Gt
        | BinaryOp::Gte
        | BinaryOp::And
        | BinaryOp::Nand
        | BinaryOp::Or
        | BinaryOp::Nor
        | BinaryOp::Xor
        | BinaryOp::Xnor => Node::Constant(0.0),
        _ => Node::Null,
    }
}

/// `cardinal_pow_optimisation(branch)`: `bipow_node`/`bipowinv_node`.
fn branch_power(base: Node, c: f32) -> Node {
    let power = functions::truncate_to_i32(functions::abs(c)) as u32;
    if power == 0 {
        return Node::Constant(1.0);
    }
    Node::Power { base: boxed(base), power, inverse: c < 0.0 }
}

/// `synthesize_cov_expression`.
fn cov(op: BinaryOp, c: f32, v: usize) -> Node {
    if c == 0.0 && (op == BinaryOp::Mul || op == BinaryOp::Div) {
        return Node::Constant(0.0);
    }
    if (c == 0.0 && op == BinaryOp::Add) || (c == 1.0 && op == BinaryOp::Mul) {
        return Node::Variable(v);
    }
    Node::Pair { op, left: Leaf::Constant(c), right: Leaf::Variable(v) }
}

/// `synthesize_voc_expression`.
fn voc(op: BinaryOp, v: usize, c: f32) -> Node {
    if cardinal_pow_optimisable(op, c) {
        if c == 1.0 {
            return Node::Variable(v);
        }
        // cardinal_pow_optimisation(v, c)
        let power = functions::truncate_to_i32(functions::abs(c)) as u32;
        if power == 0 {
            return Node::Constant(1.0);
        }
        if c == 2.0 {
            return Node::Pair { op: BinaryOp::Mul, left: Leaf::Variable(v), right: Leaf::Variable(v) };
        }
        return Node::Power { base: boxed(Node::Variable(v)), power, inverse: c < 0.0 };
    }
    if c == 0.0 && op == BinaryOp::Mul {
        return Node::Constant(0.0);
    }
    if c == 0.0 && op == BinaryOp::Div {
        return Node::Constant(f32::NAN);
    }
    let identity = (c == 0.0 && op == BinaryOp::Add) || (c == 1.0 && (op == BinaryOp::Mul || op == BinaryOp::Div));
    if identity {
        return Node::Variable(v);
    }
    Node::Pair { op, left: Leaf::Variable(v), right: Leaf::Constant(c) }
}

fn cov_pair(op: BinaryOp, c: f32, v: Leaf) -> Node {
    Node::Pair { op, left: Leaf::Constant(c), right: v }
}

fn voc_pair(op: BinaryOp, v: Leaf, c: f32) -> Node {
    Node::Pair { op, left: v, right: Leaf::Constant(c) }
}

fn leaf_constant(leaf: Leaf) -> f32 {
    match leaf {
        Leaf::Constant(value) => value,
        Leaf::Variable(_) => f32::NAN,
    }
}

/// The T3 synthesizers for `(a o0 b) o1 x` (`synthesize_*_expression0`).
fn expression0(pair: (BinaryOp, Leaf, Leaf), o1: BinaryOp, x: Leaf) -> Node {
    use BinaryOp::{Add, Div, Mul, Pow, Sub};
    let (o0, a, b) = pair;
    let letters = format!("{}{}{}", a.letter(), b.letter(), x.letter());
    match letters.as_str() {
        // (v0 / v1) / v2 --> v0 / (v1 * v2), likewise with a constant.
        "vvv" | "vvc" | "cvv" if o0 == Div && o1 == Div => return sf3ext("t/(t*t)", [a, b, x]).unwrap_or(Node::Null),
        // (v0 / c) / v1 --> v0 / (v1 * c)
        "vcv" if o0 == Div && o1 == Div => return sf3ext("t/(t*t)", [a, x, b]).unwrap_or(Node::Null),
        "cvc" => {
            let (c0, v, c1) = (leaf_constant(a), b, leaf_constant(x));
            let folded = match (o0, o1) {
                (Add, Add) => Some(cov_pair(Add, c0 + c1, v)),
                (Add, Sub) => Some(cov_pair(Add, c0 - c1, v)),
                (Sub, Add) => Some(cov_pair(Sub, c0 + c1, v)),
                (Sub, Sub) => Some(cov_pair(Sub, c0 - c1, v)),
                (Mul, Mul) => Some(cov_pair(Mul, c0 * c1, v)),
                (Mul, Div) => Some(cov_pair(Mul, c0 / c1, v)),
                (Div, Mul) => Some(cov_pair(Div, c0 * c1, v)),
                (Div, Div) => Some(cov_pair(Div, c0 / c1, v)),
                _ => None,
            };
            if let Some(folded) = folded {
                return folded;
            }
        }
        "vcc" => {
            let (v, c0, c1) = (a, leaf_constant(b), leaf_constant(x));
            let folded = match (o0, o1) {
                (Add, Add) => Some(voc_pair(Add, v, c0 + c1)),
                (Add, Sub) => Some(voc_pair(Add, v, c0 - c1)),
                (Sub, Add) => Some(voc_pair(Add, v, c1 - c0)),
                (Sub, Sub) => Some(voc_pair(Sub, v, c0 + c1)),
                (Mul, Mul) => Some(voc_pair(Mul, v, c0 * c1)),
                (Mul, Div) => Some(voc_pair(Mul, v, c0 / c1)),
                (Div, Mul) => Some(voc_pair(Mul, v, c1 / c0)),
                (Div, Div) => Some(voc_pair(Div, v, c0 * c1)),
                (Pow, Pow) => Some(voc_pair(Pow, v, c0 * c1)),
                _ => None,
            };
            if let Some(folded) = folded {
                return folded;
            }
        }
        _ => {}
    }
    let id = format!("(t{}t){}t", o0.symbol(), o1.symbol());
    let leaves = [a, b, x];
    sf3ext(&id, leaves).unwrap_or(Node::Triple { leaves, ops: [o0, o1], mode: 0 })
}

/// The T3 synthesizers for `x o0 (a o1 b)` (`synthesize_*_expression1`).
fn expression1(x: Leaf, o0: BinaryOp, pair: (BinaryOp, Leaf, Leaf)) -> Node {
    use BinaryOp::{Add, Div, Mul, Sub};
    let (o1, a, b) = pair;
    let letters = format!("{}{}{}", x.letter(), a.letter(), b.letter());
    match letters.as_str() {
        // v0 / (v1 / v2) --> (v0 * v2) / v1
        "vvv" | "cvv" if o0 == Div && o1 == Div => return sf3ext("(t*t)/t", [x, b, a]).unwrap_or(Node::Null),
        // v0 / (v1 / c) --> (v0 * c) / v1
        "vvc" if o0 == Div && o1 == Div => return sf3ext("(t*t)/t", [x, b, a]).unwrap_or(Node::Null),
        // v0 / (c / v1) --> (v0 * v1) / c
        "vcv" if o0 == Div && o1 == Div => return sf3ext("(t*t)/t", [x, b, a]).unwrap_or(Node::Null),
        "cvc" => {
            let (c0, v, c1) = (leaf_constant(x), a, leaf_constant(b));
            let folded = match (o0, o1) {
                (Add, Add) => Some(cov_pair(Add, c0 + c1, v)),
                (Add, Sub) => Some(cov_pair(Add, c0 - c1, v)),
                (Sub, Add) => Some(cov_pair(Sub, c0 - c1, v)),
                (Sub, Sub) => Some(cov_pair(Sub, c0 + c1, v)),
                (Mul, Mul) => Some(cov_pair(Mul, c0 * c1, v)),
                (Mul, Div) => Some(cov_pair(Mul, c0 / c1, v)),
                (Div, Mul) => Some(cov_pair(Div, c0 / c1, v)),
                (Div, Div) => Some(cov_pair(Div, c0 * c1, v)),
                _ => None,
            };
            if let Some(folded) = folded {
                return folded;
            }
        }
        "ccv" => {
            let (c0, c1, v) = (leaf_constant(x), leaf_constant(a), b);
            let folded = match (o0, o1) {
                (Add, Add) => Some(cov_pair(Add, c0 + c1, v)),
                (Add, Sub) => Some(cov_pair(Sub, c0 + c1, v)),
                (Sub, Add) => Some(cov_pair(Sub, c0 - c1, v)),
                (Sub, Sub) => Some(cov_pair(Add, c0 - c1, v)),
                (Mul, Mul) => Some(cov_pair(Mul, c0 * c1, v)),
                (Mul, Div) => Some(cov_pair(Div, c0 * c1, v)),
                (Div, Mul) => Some(cov_pair(Div, c0 / c1, v)),
                (Div, Div) => Some(cov_pair(Mul, c0 / c1, v)),
                _ => None,
            };
            if let Some(folded) = folded {
                return folded;
            }
        }
        _ => {}
    }
    let id = format!("t{}(t{}t)", o0.symbol(), o1.symbol());
    let leaves = [x, a, b];
    sf3ext(&id, leaves).unwrap_or(Node::Triple { leaves, ops: [o0, o1], mode: 1 })
}

/// `(c0 o c1) o (v0, v1)` as an sf3ext over (constant, v0, v1).
fn cvv(id: &str, c: f32, v0: Leaf, v1: Leaf) -> Node {
    sf3ext(id, [Leaf::Constant(c), v0, v1]).unwrap_or(Node::Null)
}

/// The T4 synthesizers for `(a o0 b) o1 (c o2 d)`
/// (`synthesize_*_expression0`).
fn quad0(left: (BinaryOp, Leaf, Leaf), o1: BinaryOp, right: (BinaryOp, Leaf, Leaf)) -> Node {
    use BinaryOp::{Add, Div, Mul, Sub};
    let (o0, a, b) = left;
    let (o2, c, d) = right;
    let letters = format!("{}{}{}{}", a.letter(), b.letter(), c.letter(), d.letter());
    let ops = (o0, o1, o2);
    let quad = |id: &str, leaves: [Leaf; 4]| sf4ext(id, leaves).unwrap_or(Node::Null);
    match letters.as_str() {
        "vvvv" => {
            let (v0, v1, v2, v3) = (a, b, c, d);
            match ops {
                (Div, Mul, Div) => return quad("(t*t)/(t*t)", [v0, v2, v1, v3]),
                (Div, Div, Div) => return quad("(t*t)/(t*t)", [v0, v3, v1, v2]),
                (Add, Div, Div) => return quad("(t+t)*(t/t)", [v0, v1, v3, v2]),
                (Sub, Div, Div) => return quad("(t-t)*(t/t)", [v0, v1, v3, v2]),
                (Mul, Div, Div) => return quad("((t*t)*t)/t", [v0, v1, v3, v2]),
                _ => {}
            }
        }
        "vvvc" => {
            let (v0, v1, v2, k) = (a, b, c, d);
            match ops {
                (Div, Mul, Div) => return quad("(t*t)/(t*t)", [v0, v2, v1, k]),
                (Div, Div, Div) => return quad("(t*t)/(t*t)", [v0, k, v1, v2]),
                _ => {}
            }
        }
        "vvcv" => {
            let (v0, v1, k, v2) = (a, b, c, d);
            match ops {
                (Div, Mul, Div) => return quad("(t*t)/(t*t)", [v0, k, v1, v2]),
                (Div, Div, Div) => return quad("(t*t)/(t*t)", [v0, v2, v1, k]),
                _ => {}
            }
        }
        "vcvv" => {
            let (v0, k, v1, v2) = (a, b, c, d);
            match ops {
                (Div, Mul, Div) => return quad("(t*t)/(t*t)", [v0, v1, k, v2]),
                (Div, Div, Div) => return quad("(t*t)/(t*t)", [v0, v2, k, v1]),
                _ => {}
            }
        }
        "cvvv" => {
            let (k, v0, v1, v2) = (a, b, c, d);
            match ops {
                (Div, Mul, Div) => return quad("(t*t)/(t*t)", [k, v1, v0, v2]),
                (Div, Div, Div) => return quad("(t*t)/(t*t)", [k, v2, v0, v1]),
                _ => {}
            }
        }
        "cvcv" => {
            let (c0, v0, c1, v1) = (leaf_constant(a), b, leaf_constant(c), d);
            match ops {
                (Add, Add, Add) => return cvv("(t+t)+t", c0 + c1, v0, v1),
                (Add, Sub, Add) => return cvv("(t+t)-t", c0 - c1, v0, v1),
                (Sub, Sub, Sub) => return cvv("(t-t)+t", c0 - c1, v0, v1),
                (Mul, Mul, Mul) => return cvv("(t*t)*t", c0 * c1, v0, v1),
                (Mul, Div, Mul) => return cvv("(t*t)/t", c0 / c1, v0, v1),
                (Div, Mul, Div) => return cvv("t/(t*t)", c0 * c1, v0, v1),
                (Div, Div, Div) => return cvv("(t*t)/t", c0 / c1, v1, v0),
                (Mul, Div, Div) => return cvv("t*(t*t)", c0 / c1, v0, v1),
                (Div, Div, Mul) => return cvv("t/(t*t)", c0 / c1, v0, v1),
                _ => {}
            }
            if c0 == c1 && o0 == Mul && o2 == Mul {
                match o1 {
                    Add => return cvv("t*(t+t)", c0, v0, v1),
                    Sub => return cvv("t*(t-t)", c0, v0, v1),
                    _ => {}
                }
            }
        }
        "vcvc" => {
            let (v0, c0, v1, c1) = (a, leaf_constant(b), c, leaf_constant(d));
            match ops {
                (Add, Add, Add) => return cvv("(t+t)+t", c0 + c1, v0, v1),
                (Add, Sub, Add) => return cvv("(t+t)-t", c0 - c1, v0, v1),
                (Sub, Sub, Sub) => return cvv("(t+t)-t", c1 - c0, v0, v1),
                (Mul, Mul, Mul) => return cvv("(t*t)*t", c0 * c1, v0, v1),
                (Mul, Div, Mul) => return cvv("(t*t)/t", c0 / c1, v0, v1),
                (Div, Mul, Div) => return cvv("(t*t)*t", 1.0 / (c0 * c1), v0, v1),
                (Div, Div, Div) => return cvv("(t*t)/t", c1 / c0, v0, v1),
                (Mul, Div, Div) => return cvv("t*(t/t)", c0 * c1, v0, v1),
                (Div, Div, Mul) => return cvv("t*(t/t)", 1.0 / (c0 * c1), v0, v1),
                (Div, Mul, Add) => return quad("(t*t)*(t+t)", [v0, Leaf::Constant(1.0 / c0), v1, Leaf::Constant(c1)]),
                (Div, Mul, Sub) => return quad("(t*t)*(t-t)", [v0, Leaf::Constant(1.0 / c0), v1, Leaf::Constant(c1)]),
                _ => {}
            }
            if c0 == c1 && o0 == Mul && o2 == Mul {
                match o1 {
                    Add => return cvv("t*(t+t)", c0, v0, v1),
                    Sub => return cvv("t*(t-t)", c0, v0, v1),
                    _ => {}
                }
            }
            if c0 == c1 && o0 == Div && o2 == Div {
                let constant = Leaf::Constant(c0);
                match o1 {
                    Add => return sf3ext("(t+t)/t", [v0, v1, constant]).unwrap_or(Node::Null),
                    Sub => return sf3ext("(t-t)/t", [v0, v1, constant]).unwrap_or(Node::Null),
                    _ => {}
                }
            }
        }
        "cvvc" => {
            let (c0, v0, v1, c1) = (leaf_constant(a), b, c, leaf_constant(d));
            match ops {
                (Add, Add, Add) => return cvv("(t+t)+t", c0 + c1, v0, v1),
                (Add, Sub, Add) => return cvv("(t+t)-t", c0 - c1, v0, v1),
                (Sub, Sub, Sub) => return cvv("t-(t+t)", c0 + c1, v0, v1),
                (Mul, Mul, Mul) => return cvv("(t*t)*t", c0 * c1, v0, v1),
                (Mul, Div, Mul) => return cvv("(t*t)/t", c0 / c1, v0, v1),
                (Div, Mul, Div) => return cvv("t*(t/t)", c0 / c1, v1, v0),
                (Div, Div, Div) => return cvv("t/(t*t)", c0 * c1, v0, v1),
                (Mul, Div, Div) => return cvv("(t*t)/t", c0 * c1, v0, v1),
                (Div, Div, Mul) => return cvv("t/(t*t)", c0 / c1, v0, v1),
                _ => {}
            }
            if c0 == c1 && o0 == Mul && o2 == Mul {
                match o1 {
                    Add => return cvv("t*(t+t)", c0, v0, v1),
                    Sub => return cvv("t*(t-t)", c0, v0, v1),
                    _ => {}
                }
            }
        }
        "vccv" => {
            let (v0, c0, c1, v1) = (a, leaf_constant(b), leaf_constant(c), d);
            match ops {
                (Add, Add, Add) => return cvv("(t+t)+t", c0 + c1, v0, v1),
                (Add, Sub, Add) => return cvv("(t+t)-t", c0 - c1, v0, v1),
                (Sub, Sub, Sub) => return sf3ext("(t+t)-t", [v0, v1, Leaf::Constant(c1 + c0)]).unwrap_or(Node::Null),
                (Mul, Mul, Mul) => return cvv("(t*t)*t", c0 * c1, v0, v1),
                (Mul, Div, Mul) => return cvv("(t*t)/t", c0 / c1, v0, v1),
                (Div, Mul, Div) => return cvv("(t*t)/t", c1 / c0, v0, v1),
                (Mul, Div, Div) => return cvv("(t*t)*t", c0 / c1, v0, v1),
                (Div, Div, Mul) => return cvv("(t*t)/t", 1.0 / (c0 * c1), v0, v1),
                (Div, Div, Div) => return sf3ext("(t*t)*t", [v0, v1, Leaf::Constant(1.0 / (c0 * c1))]).unwrap_or(Node::Null),
                _ => {}
            }
            if c0 == c1 && o0 == Mul && o2 == Mul {
                match o1 {
                    Add => return cvv("t*(t+t)", c0, v0, v1),
                    Sub => return cvv("t*(t-t)", c0, v0, v1),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let id = format!("(t{}t){}(t{}t)", o0.symbol(), o1.symbol(), o2.symbol());
    let leaves = [a, b, c, d];
    sf4ext(&id, leaves).unwrap_or(Node::Quad { leaves, ops: [o0, o1, o2], mode: 0 })
}

/// T4 families 1 to 4 (a leaf beside a plain T0oT1oT2 node): only the
/// sf4ext lookup, then the plain node.
fn quad_family(family: u8, leaf: Leaf, leaves: [Leaf; 3], triple_ops: [BinaryOp; 2], op: BinaryOp) -> Node {
    let [t0, t1, t2] = leaves;
    let (four, ops) = if family <= 2 { ([leaf, t0, t1, t2], [op, triple_ops[0], triple_ops[1]]) } else { ([t0, t1, t2, leaf], [triple_ops[0], triple_ops[1], op]) };
    let [s0, s1, s2] = ops.map(BinaryOp::symbol);
    // Family 4's id is unbalanced in ExprTk ("(t+(t*t)-t"), so it never
    // names an sf4 function.
    let id = match family {
        1 => format!("t{s0}(t{s1}(t{s2}t))"),
        2 => format!("t{s0}((t{s1}t){s2}t)"),
        3 => format!("((t{s0}t){s1}t){s2}t"),
        _ => format!("(t{s0}(t{s1}t){s2}t"),
    };
    sf4ext(&id, four).unwrap_or(Node::Quad { leaves: four, ops, mode: family })
}

/// `compile_right`: `t o (sf3ext)` as an sf4ext.
fn compile_right(t: Leaf, op: BinaryOp, node: &Node) -> Option<Node> {
    let (index, [t0, t1, t2]) = sf3ext_parts(node)?;
    let id = format!("t{}({})", op.symbol(), SF3_PATTERNS[usize::from(index)]);
    sf4ext(&id, [t, t0, t1, t2])
}

/// `compile_left`: `(sf3ext) o t` as an sf4ext.
fn compile_left(node: &Node, op: BinaryOp, t: Leaf) -> Option<Node> {
    let (index, [t0, t1, t2]) = sf3ext_parts(node)?;
    let id = format!("({}){}t", SF3_PATTERNS[usize::from(index)], op.symbol());
    sf4ext(&id, [t0, t1, t2, t])
}

/// The parts of a cob node: operator, constant, branch.
fn cob_parts(node: &Node) -> Option<(BinaryOp, f32, &Node)> {
    match node {
        Node::Binary { op, left, right, kind: BranchKind::Cob } => Some((*op, left.constant()?, right)),
        _ => None,
    }
}

/// The parts of a boc node: operator, branch, constant.
fn boc_parts(node: &Node) -> Option<(BinaryOp, &Node, f32)> {
    match node {
        Node::Binary { op, left, right, kind: BranchKind::Boc } => Some((*op, left, right.constant()?)),
        _ => None,
    }
}

fn cob_node(op: BinaryOp, c: f32, branch: &Node) -> Node {
    binary(op, Node::Constant(c), branch.clone(), BranchKind::Cob)
}

fn boc_node(op: BinaryOp, branch: &Node, c: f32) -> Node {
    binary(op, branch.clone(), Node::Constant(c), BranchKind::Boc)
}

/// `synthesize_cocob_expression`: a constant against a cob node.
fn cocob_expression(op: BinaryOp, left: &Node, right: &Node) -> Option<Node> {
    use BinaryOp::{Add, Div, Mul, Sub};
    if let (Some((cob_op, c0, inner)), Some(c)) = (cob_parts(left), right.constant()) {
        // (cob) o c --> cob
        if c == 0.0 && op == Mul {
            return Some(Node::Constant(0.0));
        }
        if c == 0.0 && op == Div {
            return Some(Node::Constant(f32::NAN));
        }
        if (c == 0.0 && op == Add) || (c == 1.0 && (op == Mul || op == Div)) {
            return Some(left.clone());
        }
        return match (cob_op, op) {
            (Add | Sub, Add) => Some(cob_node(cob_op, c0 + c, inner)),
            (Add | Sub, Sub) => Some(cob_node(cob_op, c0 - c, inner)),
            (Mul, Mul) => Some(cob_node(Mul, c0 * c, inner)),
            (Mul, Div) => Some(cob_node(Mul, c0 / c, inner)),
            (Div, Mul) => Some(cob_node(Div, c0 * c, inner)),
            (Div, Div) => Some(cob_node(Div, c0 / c, inner)),
            _ => None,
        };
    }
    // c o (cob) --> cob
    let (cob_op, c0, inner) = cob_parts(right)?;
    let c = left.constant()?;
    if c == 0.0 && (op == Mul || op == Div) {
        return Some(Node::Constant(0.0));
    }
    if (c == 0.0 && op == Add) || (c == 1.0 && op == Mul) {
        return Some(right.clone());
    }
    match (cob_op, op) {
        (Add, Add) => Some(cob_node(Add, c + c0, inner)),
        (Add, Sub) => Some(cob_node(Sub, c - c0, inner)),
        (Sub, Add) => Some(cob_node(Sub, c + c0, inner)),
        (Sub, Sub) => Some(cob_node(Add, c - c0, inner)),
        (Mul, Mul) => Some(cob_node(Mul, c * c0, inner)),
        (Mul, Div) => Some(cob_node(Div, c / c0, inner)),
        (Div, Mul) => Some(cob_node(Div, c * c0, inner)),
        (Div, Div) => Some(cob_node(Mul, c / c0, inner)),
        _ => None,
    }
}

/// `synthesize_coboc_expression`: a constant against a boc node.
fn coboc_expression(op: BinaryOp, left: &Node, right: &Node) -> Option<Node> {
    use BinaryOp::{Add, Div, Mul, Sub};
    if let (Some((boc_op, inner, c0)), Some(c)) = (boc_parts(left), right.constant()) {
        // (boc) o c --> boc
        return match (boc_op, op) {
            (Add, Add) => Some(boc_node(Add, inner, c0 + c)),
            (Add, Sub) => Some(boc_node(Add, inner, c0 - c)),
            (Mul, Mul) => Some(boc_node(Mul, inner, c0 * c)),
            (Mul, Div) => Some(boc_node(Mul, inner, c0 / c)),
            (Sub, Add) => Some(boc_node(Add, inner, c - c0)),
            (Sub, Sub) => Some(boc_node(Sub, inner, c0 + c)),
            (Div, Div) => Some(boc_node(Div, inner, c0 * c)),
            (Div, Mul) => Some(boc_node(Div, inner, c0 / c)),
            _ => None,
        };
    }
    // c o (boc) --> boc/cob
    let (boc_op, inner, c0) = boc_parts(right)?;
    let c = left.constant()?;
    match (boc_op, op) {
        (Add, Add) => Some(boc_node(Add, inner, c + c0)),
        (Add, Sub) => Some(cob_node(Sub, c - c0, inner)),
        (Sub, Add) => Some(boc_node(Add, inner, c - c0)),
        (Sub, Sub) => Some(cob_node(Sub, c + c0, inner)),
        (Mul, Mul) => Some(boc_node(Mul, inner, c * c0)),
        (Mul, Div) => Some(cob_node(Div, c / c0, inner)),
        (Div, Mul) => Some(boc_node(Div, inner, c0 / c)),
        (Div, Div) => Some(cob_node(Div, c * c0, inner)),
        _ => None,
    }
}
