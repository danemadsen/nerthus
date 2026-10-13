# Allowed Functions

The following operators, functions, and language features are available in [Expressions](expressions.md) and the [Math Node](math-node.md).

Use this page as a reference when you need to transform a value, constrain a variable, or build a procedural calculation inside the Math node.

## Common Operators

| Type | Available syntax |
| --- | --- |
| Arithmetic | `+`, `-`, `*`, `/`, `%`, `^` |
| Equality and comparison | `=`, `==`, `<>`, `!=`, `<`, `<=`, `>`, `>=` |
| Assignment | `:=`, `+=`, `-=`, `*=`, `/=`, `%=` |
| Logic | `and`, `nand`, `nor`, `not`, `or`, `xor`, `xnor`, `mand`, `mor` |

## Numeric Functions

| Use | Functions |
| --- | --- |
| Basic values | `abs`, `sgn`, `floor`, `ceil`, `round`, `roundn`, `trunc`, `frac` |
| Bounds and ranges | `clamp`, `iclamp`, `inrange`, `min`, `max` |
| Aggregation | `avg`, `sum`, `mul` |
| Powers and roots | `pow`, `sqrt`, `root`, `hypot` |
| Logs and exponentials | `exp`, `expm1`, `log`, `log10`, `log1p`, `log2`, `logn` |
| Remainders | `mod` |
| Probability and error functions | `erf`, `erfc`, `ncdf` |
| Comparison helpers | `equal`, `not_equal` |
| Value exchange | `swap` |

## Trigonometry

| Use | Functions |
| --- | --- |
| Standard trig | `sin`, `cos`, `tan`, `cot`, `sec`, `csc`, `sinc` |
| Inverse trig | `asin`, `acos`, `atan`, `atan2` |
| Hyperbolic trig | `sinh`, `cosh`, `tanh` |
| Inverse hyperbolic trig | `asinh`, `acosh`, `atanh` |
| Unit conversion | `deg2rad`, `rad2deg`, `deg2grad`, `grad2deg` |

## Flow Control

| Feature | Use it for |
| --- | --- |
| `if-then-else` | Branching between values or calculations. |
| Ternary conditional | Compact conditional expressions. |
| `switch` / `case` | Choosing between several possible branches. |
| `return` | Ending a calculation and returning a value. |
| `while` | Repeating while a condition remains true. |
| `for` | Repeating with a counter or bounded iteration. |
| `repeat until` | Repeating until a condition becomes true. |
| `break` and `continue` | Controlling loop execution. |

## Variables, Strings, and Composition

| Feature | Notes |
| --- | --- |
| Expression local variables | Store intermediate values inside an expression. |
| User-defined variables | Use values exposed through Nerthus variables. |
| Vectors and strings | Create and manipulate vector or string values. |
| String operations | Equality, inequality, logical operations, concatenation, and sub-ranges. |
| Constants and user functions | Define reusable values and helper functions. |
| Multivariate function composition | Build functions that depend on multiple inputs. |
| Multiple sequence points | Use multiple sub-expressions in a single calculation. |

## Advanced Facilities

These features are available for more complex Math node and expression work. Use them when a simple value transform is not enough.

| Area | Facilities |
| --- | --- |
| Optimization | Constant folding, strength reduction, operator coupling, special functions, and dead code elimination. |
| Runtime checks | Vector bounds, string bounds, loop iteration and execution-time bounds, and assert statements. |
| Compile-time checks | Function parameter type checking, compilation stack overflow checks, evaluation stack overflow checks, and compilation running time checks. |
| Numerical methods | Numerical integration and differentiation. |

## Vector Processing

| Area | Facilities |
| --- | --- |
| BLAS-L1 style operations | `axpy`, `axpby`, `axpb` |
| Boolean checks | All true, any true, all false, any false |
| Vector manipulation | `assign`, `copy`, `reverse`, `rotate-left`, `rotate-right`, `shift-left`, `shift-right`, `sort`, `nth_element`, `iota` |
| Vector analysis | `count`, `sum`, `kahan-sum`, `dot-product`, `diff`, `thresholding` |

## File I/O

The File I/O package includes routines such as `open`, `close`, `read`, `write`, and `getline`, with support for binary and text modes.

For typical parameter work, prefer [Variables](variables.md) and [Expressions](expressions.md) before reaching for File I/O or vector-processing features.
