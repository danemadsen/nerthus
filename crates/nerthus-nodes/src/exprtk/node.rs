//! ExprTk's expression nodes as the port keeps them: each variant is one of
//! ExprTk's node classes (which later synthesis matches on), and evaluates
//! the way that class does, in `float`.

use crate::exprtk::functions;

/// A binary operator (`details::operator_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Lte,
    Eq,
    Ne,
    Gte,
    Gt,
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    // The two-argument functions, which ExprTk also parses into binary
    // nodes (none of its specialised nodes take them).
    Atan2,
    Hypot,
    Logn,
    Root,
    Roundn,
    Equal,
    NotEqual,
    Shr,
    Shl,
}

impl BinaryOp {
    /// `operation_optimisable`: the operators with specialised nodes.
    pub fn is_optimisable(self) -> bool {
        !matches!(
            self,
            BinaryOp::Atan2
                | BinaryOp::Hypot
                | BinaryOp::Logn
                | BinaryOp::Root
                | BinaryOp::Roundn
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Shr
                | BinaryOp::Shl
        )
    }

    /// The two-argument functions by name.
    pub fn function_named(name: &str) -> Option<BinaryOp> {
        let op = match name {
            "atan2" => BinaryOp::Atan2,
            "mod" => BinaryOp::Mod,
            "logn" => BinaryOp::Logn,
            "pow" => BinaryOp::Pow,
            "root" => BinaryOp::Root,
            "roundn" => BinaryOp::Roundn,
            "equal" => BinaryOp::Equal,
            "not_equal" => BinaryOp::NotEqual,
            "hypot" => BinaryOp::Hypot,
            "shr" => BinaryOp::Shr,
            "shl" => BinaryOp::Shl,
            _ => return None,
        };
        Some(op)
    }

    /// `expression_generator::to_str`, which builds the sf3/sf4 ids.
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Pow => "^",
            BinaryOp::Lt => "<",
            BinaryOp::Lte => "<=",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Gte => ">=",
            BinaryOp::Gt => ">",
            BinaryOp::And => "and",
            BinaryOp::Nand => "nand",
            BinaryOp::Or => "or",
            BinaryOp::Nor => "nor",
            BinaryOp::Xor => "xor",
            BinaryOp::Xnor => "xnor",
            _ => "UNKNOWN",
        }
    }

    /// `apply` where MSVC's code puts the right operand first: for `+`
    /// and `*` a NaN on the right then wins over one on the left (only the
    /// NaN's sign can differ).
    pub fn apply_right_first(self, left: f32, right: f32) -> f32 {
        match self {
            BinaryOp::Add | BinaryOp::Mul if left.is_nan() && right.is_nan() => functions::nan1(right, right),
            _ => self.apply(left, right),
        }
    }

    pub fn apply(self, left: f32, right: f32) -> f32 {
        use functions as f;
        let truth = |condition: bool| if condition { 1.0 } else { 0.0 };
        let is_true = |value: f32| value != 0.0;
        match self {
            BinaryOp::Add => f::add(left, right),
            BinaryOp::Sub => f::sub(left, right),
            BinaryOp::Mul => f::mul(left, right),
            BinaryOp::Div => f::div(left, right),
            BinaryOp::Mod => f::fmod(left, right),
            // x86's powf gives the exponent's NaN when both are NaN.
            BinaryOp::Pow => f::nan2(left.powf(right), right, left),
            BinaryOp::Lt => truth(left < right),
            BinaryOp::Lte => truth(left <= right),
            BinaryOp::Eq => truth(left == right),
            BinaryOp::Ne => truth(left != right),
            BinaryOp::Gte => truth(left >= right),
            BinaryOp::Gt => truth(left > right),
            BinaryOp::And => truth(is_true(left) && is_true(right)),
            BinaryOp::Nand => truth(!is_true(left) || !is_true(right)),
            BinaryOp::Or => truth(is_true(left) || is_true(right)),
            BinaryOp::Nor => truth(!is_true(left) && !is_true(right)),
            BinaryOp::Xor => truth(is_true(left) != is_true(right)),
            BinaryOp::Xnor => truth(is_true(left) == is_true(right)),
            BinaryOp::Atan2 => f::nan2(left.atan2(right), left, right),
            BinaryOp::Hypot => {
                let sum = f::add(f::mul(left, left), f::mul(right, right));
                f::nan1(sum.sqrt(), sum)
            }
            BinaryOp::Logn => f::div(f::nan1(left.ln(), left), f::nan1(right.ln(), right)),
            BinaryOp::Root => f::root(left, right),
            BinaryOp::Roundn => f::roundn(left, right),
            BinaryOp::Equal => f::equal(left, right),
            BinaryOp::NotEqual => f::not_equal(left, right),
            BinaryOp::Shr => {
                let power = f::truncate_to_i32(right) as f32;
                f::mul(left, 1.0 / 2.0f32.powf(power))
            }
            BinaryOp::Shl => {
                let power = f::truncate_to_i32(right) as f32;
                f::mul(left, 2.0f32.powf(power))
            }
        }
    }
}

/// A one-argument operation (`unary_opr_switch_statements`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Ceil,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Log,
    Log10,
    Log2,
    Log1p,
    Neg,
    Pos,
    Round,
    Sin,
    Sinc,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Cot,
    Sec,
    Csc,
    R2d,
    D2r,
    D2g,
    G2d,
    Notl,
    Sgn,
    Erf,
    Erfc,
    Ncdf,
    Frac,
    Trunc,
}

impl UnaryOp {
    /// The one-argument base functions by name (`not` included).
    pub fn named(name: &str) -> Option<UnaryOp> {
        let op = match name {
            "abs" => UnaryOp::Abs,
            "acos" => UnaryOp::Acos,
            "acosh" => UnaryOp::Acosh,
            "asin" => UnaryOp::Asin,
            "asinh" => UnaryOp::Asinh,
            "atan" => UnaryOp::Atan,
            "atanh" => UnaryOp::Atanh,
            "ceil" => UnaryOp::Ceil,
            "cos" => UnaryOp::Cos,
            "cosh" => UnaryOp::Cosh,
            "exp" => UnaryOp::Exp,
            "expm1" => UnaryOp::Expm1,
            "floor" => UnaryOp::Floor,
            "log" => UnaryOp::Log,
            "log10" => UnaryOp::Log10,
            "log2" => UnaryOp::Log2,
            "log1p" => UnaryOp::Log1p,
            "round" => UnaryOp::Round,
            "sin" => UnaryOp::Sin,
            "sinc" => UnaryOp::Sinc,
            "sinh" => UnaryOp::Sinh,
            "sec" => UnaryOp::Sec,
            "csc" => UnaryOp::Csc,
            "sqrt" => UnaryOp::Sqrt,
            "tan" => UnaryOp::Tan,
            "tanh" => UnaryOp::Tanh,
            "cot" => UnaryOp::Cot,
            "rad2deg" => UnaryOp::R2d,
            "deg2rad" => UnaryOp::D2r,
            "deg2grad" => UnaryOp::D2g,
            "grad2deg" => UnaryOp::G2d,
            "sgn" => UnaryOp::Sgn,
            "not" => UnaryOp::Notl,
            "erf" => UnaryOp::Erf,
            "erfc" => UnaryOp::Erfc,
            "ncdf" => UnaryOp::Ncdf,
            "frac" => UnaryOp::Frac,
            "trunc" => UnaryOp::Trunc,
            _ => return None,
        };
        Some(op)
    }

    /// Whether the operator is in the `unary_op_map_` (ExprTk
    /// 2024-01-01 forgot `atan`, so `uv o uv` with an `atan` fails to
    /// compile).
    pub fn in_unary_op_map(self) -> bool {
        self != UnaryOp::Atan
    }

    pub fn apply(self, value: f32) -> f32 {
        let result = self.raw(value);
        // Negation and abs set the sign themselves; log1p's NaN (below -1
        // and for NaN) is ExprTk's own quiet NaN; ncdf negates inside.
        if matches!(self, UnaryOp::Neg | UnaryOp::Abs | UnaryOp::Log1p | UnaryOp::Ncdf) {
            return result;
        }
        // x86's coshf and tanhf return a NaN input without its sign.
        if matches!(self, UnaryOp::Cosh | UnaryOp::Tanh) && value.is_nan() {
            return f32::from_bits(functions::abs(value).to_bits() | 0x0040_0000);
        }
        functions::nan1(result, value)
    }

    fn raw(self, value: f32) -> f32 {
        use functions as f;
        match self {
            UnaryOp::Abs => f::abs(value),
            UnaryOp::Acos => value.acos(),
            UnaryOp::Acosh => (value + ((value * value) - 1.0).sqrt()).ln(),
            UnaryOp::Asin => value.asin(),
            UnaryOp::Asinh => (value + ((value * value) + 1.0).sqrt()).ln(),
            UnaryOp::Atan => value.atan(),
            UnaryOp::Atanh => ((1.0 + value).ln() - (1.0 - value).ln()) / 2.0,
            UnaryOp::Ceil => value.ceil(),
            UnaryOp::Cos => value.cos(),
            UnaryOp::Cosh => value.cosh(),
            UnaryOp::Exp => value.exp(),
            UnaryOp::Expm1 => f::expm1(value),
            UnaryOp::Floor => value.floor(),
            UnaryOp::Log => value.ln(),
            UnaryOp::Log10 => value.log10(),
            UnaryOp::Log2 => value.ln() / f::LOG2,
            UnaryOp::Log1p => f::log1p(value),
            UnaryOp::Neg => -value,
            UnaryOp::Pos => value,
            UnaryOp::Round => f::round(value),
            UnaryOp::Sin => value.sin(),
            UnaryOp::Sinc => f::sinc(value),
            UnaryOp::Sinh => value.sinh(),
            UnaryOp::Sqrt => value.sqrt(),
            UnaryOp::Tan => value.tan(),
            UnaryOp::Tanh => value.tanh(),
            UnaryOp::Cot => 1.0 / value.tan(),
            UnaryOp::Sec => 1.0 / value.cos(),
            UnaryOp::Csc => 1.0 / value.sin(),
            UnaryOp::R2d => value * f::_180_PI,
            UnaryOp::D2r => value * f::PI_180,
            UnaryOp::D2g => value * f::TEN_NINTHS,
            UnaryOp::G2d => value * f::NINE_TENTHS,
            UnaryOp::Notl => {
                if value != 0.0 {
                    0.0
                } else {
                    1.0
                }
            }
            UnaryOp::Sgn => f::sgn(value),
            UnaryOp::Erf => f::erf(value),
            UnaryOp::Erfc => f::erfc(value),
            UnaryOp::Ncdf => f::ncdf(value),
            UnaryOp::Frac => value - f::trunc(value),
            UnaryOp::Trunc => f::trunc(value),
        }
    }
}

/// The operands of ExprTk's specialised nodes: a variable or a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leaf {
    Variable(usize),
    Constant(f32),
}

impl Leaf {
    pub fn value(self, variables: &[f32]) -> f32 {
        match self {
            Leaf::Variable(slot) => variables[slot],
            Leaf::Constant(value) => value,
        }
    }

    /// `exprtk_crtype`: `v` or `c`.
    pub fn letter(self) -> char {
        match self {
            Leaf::Variable(_) => 'v',
            Leaf::Constant(_) => 'c',
        }
    }

    pub fn is_variable(self) -> bool {
        matches!(self, Leaf::Variable(_))
    }
}

/// The leaf pattern of a list of leaves (`"vovoc"` is `vvc`).
pub fn leaf_letters(leaves: &[Leaf]) -> String {
    leaves.iter().map(|leaf| leaf.letter()).collect()
}

/// Which class a binary node over branches is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
    /// `binary_node` (and the generic constant-foldable node).
    Plain,
    /// `binary_ext_node`.
    Ext,
    Vob,
    Bov,
    Cob,
    Boc,
}

/// The three-argument functions (`trinary_node`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionOp {
    Clamp,
    Iclamp,
    Inrange,
}

impl FunctionOp {
    pub fn named(name: &str) -> Option<FunctionOp> {
        match name {
            "clamp" => Some(FunctionOp::Clamp),
            "iclamp" => Some(FunctionOp::Iclamp),
            "inrange" => Some(FunctionOp::Inrange),
            _ => None,
        }
    }
}

/// `min`, `max`, `avg`, `sum`, `mul`, `mand`, `mor`, `~` (multi).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarArgOp {
    Sum,
    Mul,
    Avg,
    Min,
    Max,
    Mand,
    Mor,
    Multi,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// `null`.
    Null,
    Constant(f32),
    Variable(usize),
    /// `unary_variable_node` (an operation on a bare variable) and
    /// `unary_branch_node`.
    Unary { op: UnaryOp, operand: Box<Node> },
    /// `vov`, `cov`, `voc` (two leaves).
    Pair { op: BinaryOp, left: Leaf, right: Leaf },
    /// `T0oT1oT2`: mode 0 is `(t0 o0 t1) o1 t2`, mode 1 `t0 o0 (t1 o1 t2)`.
    Triple { leaves: [Leaf; 3], ops: [BinaryOp; 2], mode: u8 },
    /// `T0oT1oT2_sf3ext`: `sf00`..`sf30` on three leaves.
    Sf3Ext { index: u8, leaves: [Leaf; 3] },
    /// `T0oT1oT2oT3` (modes as ExprTk numbers them).
    Quad { leaves: [Leaf; 4], ops: [BinaryOp; 3], mode: u8 },
    /// `T0oT1oT2oT3_sf4ext`: an entry of `functions::SF4_PATTERNS`.
    Sf4Ext { index: u8, leaves: [Leaf; 4] },
    /// `x ^ n` for a whole `n` by repeated multiplication (`ipow_node`,
    /// `bipow_node`), `1 / x ^ n` when inverse.
    Power { base: Box<Node>, power: u32, inverse: bool },
    Binary { op: BinaryOp, left: Box<Node>, right: Box<Node>, kind: BranchKind },
    /// `uvouv_node`: `op(u0(v0), u1(v1))`.
    Uvouv { op: BinaryOp, left: (UnaryOp, usize), right: (UnaryOp, usize) },
    /// `trinary_node`: clamp, iclamp, inrange.
    Trinary { op: FunctionOp, arguments: Box<[Node; 3]> },
    /// `sf3_node`/`sf4_node` from `$fNN(...)`.
    Special { index: u8, arguments: Vec<Node> },
    VarArg { op: VarArgOp, arguments: Vec<Node> },
    /// `if`/ternary: `cons_conditional_node` when there is no alternative.
    Conditional { condition: Box<Node>, consequent: Box<Node>, alternative: Option<Box<Node>> },
    /// `scand_node` (`&`) and `scor_node` (`|`).
    ShortCircuit { or: bool, left: Box<Node>, right: Box<Node> },
    /// `null_eq_node`: whether the operand is NaN (`equality`) or not.
    NullEquality { operand: Box<Node>, equality: bool },
    /// `v := e` and `v += e` and friends.
    Assign { slot: usize, op: Option<BinaryOp>, value: Box<Node> },
    /// `swap(v0, v1)` / `v0 <=> v1`.
    Swap { first: usize, second: usize },
    While { condition: Box<Node>, body: Box<Node> },
    Repeat { body: Box<Node>, condition: Box<Node> },
    For { initialiser: Option<Box<Node>>, condition: Box<Node>, incrementor: Option<Box<Node>>, body: Box<Node> },
    /// `switch { case c: e; ... default: e; }`: pairs of condition and
    /// consequent, then the default.
    Switch { cases: Vec<(Node, Node)>, default: Box<Node> },
    /// `[*] { case c: e; ... }`.
    MultiSwitch { cases: Vec<(Node, Node)> },
    Break(Option<Box<Node>>),
    Continue,
}

/// How a statement ended: normally, or by `break`/`continue` unwinding to
/// the closest loop.
pub enum Flow {
    Normal(f32),
    Break(f32),
    Continue,
}

impl Node {
    pub fn is_constant(&self) -> bool {
        matches!(self, Node::Constant(_))
    }

    pub fn is_variable(&self) -> bool {
        matches!(self, Node::Variable(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Node::Null)
    }

    pub fn constant(&self) -> Option<f32> {
        match self {
            Node::Constant(value) => Some(*value),
            _ => None,
        }
    }

    pub fn variable(&self) -> Option<usize> {
        match self {
            Node::Variable(slot) => Some(*slot),
            _ => None,
        }
    }

    pub fn as_leaf(&self) -> Option<Leaf> {
        match self {
            Node::Constant(value) => Some(Leaf::Constant(*value)),
            Node::Variable(slot) => Some(Leaf::Variable(*slot)),
            _ => None,
        }
    }

    /// `is_uv_node`: a unary operation on a bare variable.
    pub fn as_uv(&self) -> Option<(UnaryOp, usize)> {
        match self {
            Node::Unary { op, operand } => operand.variable().map(|slot| (*op, slot)),
            _ => None,
        }
    }

    /// `is_neg_unary_node`.
    pub fn is_negation(&self) -> bool {
        matches!(self, Node::Unary { op: UnaryOp::Neg, .. })
    }

    pub fn is_cob(&self) -> bool {
        matches!(self, Node::Binary { kind: BranchKind::Cob, .. })
    }

    pub fn is_boc(&self) -> bool {
        matches!(self, Node::Binary { kind: BranchKind::Boc, .. })
    }

    pub fn is_break_or_continue(&self) -> bool {
        matches!(self, Node::Break(_) | Node::Continue)
    }

    pub fn evaluate(&self, variables: &mut [f32]) -> f32 {
        match self.run(variables) {
            Flow::Normal(value) | Flow::Break(value) => value,
            Flow::Continue => f32::NAN,
        }
    }

    /// Evaluates the node, passing `break`/`continue` up to the closest
    /// loop (ExprTk throws them as exceptions).
    fn run(&self, variables: &mut [f32]) -> Flow {
        match self {
            Node::Break(value) => {
                let result = match value {
                    Some(value) => value.evaluate(variables),
                    None => f32::NAN,
                };
                Flow::Break(result)
            }
            Node::Continue => Flow::Continue,
            Node::VarArg { op: VarArgOp::Multi, arguments } => {
                let mut last = f32::NAN;
                for argument in arguments {
                    match argument.run(variables) {
                        Flow::Normal(value) => last = value,
                        other => return other,
                    }
                }
                Flow::Normal(last)
            }
            Node::Conditional { condition, consequent, alternative } => {
                let test = condition.evaluate(variables);
                if test != 0.0 {
                    consequent.run(variables)
                } else {
                    match alternative {
                        Some(alternative) => alternative.run(variables),
                        None => Flow::Normal(f32::NAN),
                    }
                }
            }
            Node::While { condition, body } => {
                let mut result = 0.0;
                while condition.evaluate(variables) != 0.0 {
                    match body.run(variables) {
                        Flow::Normal(value) => result = value,
                        Flow::Break(value) => return Flow::Normal(value),
                        Flow::Continue => {}
                    }
                }
                Flow::Normal(result)
            }
            Node::Repeat { body, condition } => {
                let mut result = 0.0;
                loop {
                    match body.run(variables) {
                        Flow::Normal(value) => result = value,
                        Flow::Break(value) => return Flow::Normal(value),
                        Flow::Continue => {}
                    }
                    if condition.evaluate(variables) != 0.0 {
                        break;
                    }
                }
                Flow::Normal(result)
            }
            Node::For { initialiser, condition, incrementor, body } => {
                let mut result = 0.0;
                if let Some(initialiser) = initialiser {
                    initialiser.evaluate(variables);
                }
                while condition.evaluate(variables) != 0.0 {
                    match body.run(variables) {
                        Flow::Normal(value) => result = value,
                        Flow::Break(value) => return Flow::Normal(value),
                        Flow::Continue => {}
                    }
                    if let Some(incrementor) = incrementor {
                        incrementor.evaluate(variables);
                    }
                }
                Flow::Normal(result)
            }
            Node::Switch { cases, default } => {
                for (condition, consequent) in cases {
                    if condition.evaluate(variables) != 0.0 {
                        return consequent.run(variables);
                    }
                }
                default.run(variables)
            }
            Node::MultiSwitch { cases } => {
                let mut result = 0.0;
                for (condition, consequent) in cases {
                    if condition.evaluate(variables) != 0.0 {
                        match consequent.run(variables) {
                            Flow::Normal(value) => result = value,
                            other => return other,
                        }
                    }
                }
                Flow::Normal(result)
            }
            _ => Flow::Normal(self.value(variables)),
        }
    }

    /// The value of a node that cannot break out of a loop.
    fn value(&self, variables: &mut [f32]) -> f32 {
        match self {
            Node::Null => f32::NAN,
            Node::Constant(value) => *value,
            Node::Variable(slot) => variables[*slot],
            Node::Unary { op, operand } => {
                let value = operand.evaluate(variables);
                op.apply(value)
            }
            Node::Pair { op, left, right } => op.apply_right_first(left.value(variables), right.value(variables)),
            // T0oT1oT2 and T0oT1oT2oT3 call their operators through function
            // pointers, so the left operand's NaN wins a tie.
            Node::Triple { leaves, ops, mode } => {
                let [t0, t1, t2] = leaves.map(|leaf| leaf.value(variables));
                if *mode == 0 {
                    ops[1].apply(ops[0].apply(t0, t1), t2)
                } else {
                    ops[0].apply(t0, ops[1].apply(t1, t2))
                }
            }
            Node::Sf3Ext { index, leaves } => {
                let [x, y, z] = leaves.map(|leaf| leaf.value(variables));
                functions::sf3(*index, x, y, z)
            }
            Node::Quad { leaves, ops, mode } => {
                let [t0, t1, t2, t3] = leaves.map(|leaf| leaf.value(variables));
                let [f0, f1, f2] = *ops;
                match mode {
                    0 => f1.apply(f0.apply(t0, t1), f2.apply(t2, t3)),
                    1 => f0.apply(t0, f1.apply(t1, f2.apply(t2, t3))),
                    2 => f0.apply(t0, f2.apply(f1.apply(t1, t2), t3)),
                    3 => f2.apply(f1.apply(f0.apply(t0, t1), t2), t3),
                    _ => f2.apply(f0.apply(t0, f1.apply(t1, t2)), t3),
                }
            }
            Node::Sf4Ext { index, leaves } => {
                let [x, y, z, w] = leaves.map(|leaf| leaf.value(variables));
                functions::sf4(*index, x, y, z, w)
            }
            Node::Power { base, power, inverse } => {
                let value = base.evaluate(variables);
                let result = functions::fast_exp(value, *power);
                if *inverse {
                    functions::div(1.0, result)
                } else {
                    result
                }
            }
            Node::Binary { op, left, right, kind } => {
                let left_value = left.evaluate(variables);
                let right_value = right.evaluate(variables);
                // The computed branch (in a register) wins a NaN tie.
                match kind {
                    BranchKind::Bov | BranchKind::Boc => op.apply(left_value, right_value),
                    _ => op.apply_right_first(left_value, right_value),
                }
            }
            Node::Uvouv { op, left, right } => {
                let left_value = left.0.apply(variables[left.1]);
                let right_value = right.0.apply(variables[right.1]);
                // Called through a function pointer: the left NaN wins.
                op.apply(left_value, right_value)
            }
            Node::Trinary { op, arguments } => {
                let lower = arguments[0].evaluate(variables);
                let value = arguments[1].evaluate(variables);
                let upper = arguments[2].evaluate(variables);
                functions::function(*op, lower, value, upper)
            }
            Node::Special { index, arguments } => {
                let mut values = [0.0f32; 4];
                for (target, argument) in values.iter_mut().zip(arguments) {
                    *target = argument.evaluate(variables);
                }
                functions::special(*index, values)
            }
            Node::VarArg { op, arguments } => functions::vararg(*op, arguments, variables),
            Node::ShortCircuit { or, left, right } => {
                let first = left.evaluate(variables) != 0.0;
                if *or {
                    if first || right.evaluate(variables) != 0.0 {
                        1.0
                    } else {
                        0.0
                    }
                } else if first && right.evaluate(variables) != 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Node::NullEquality { operand, equality } => {
                let is_nan = operand.evaluate(variables).is_nan();
                if is_nan == *equality {
                    1.0
                } else {
                    0.0
                }
            }
            Node::Assign { slot, op, value } => {
                let assigned = value.evaluate(variables);
                let result = match op {
                    Some(op) => op.apply(variables[*slot], assigned),
                    None => assigned,
                };
                variables[*slot] = result;
                result
            }
            Node::Swap { first, second } => {
                variables.swap(*first, *second);
                variables[*first]
            }
            _ => self.evaluate(variables),
        }
    }
}
