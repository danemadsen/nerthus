//! `Spline1DRegular`: a cubic spline through evenly spaced values,
//! by default the constrained form (no overshoot between monotonic nodes).

#[derive(Clone, Copy, Debug, Default)]
struct Segment {
    value: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
}

impl Segment {
    fn evaluate(&self, t: f32) -> f32 {
        ((self.a * t + self.b) * t + self.c) * t + self.d
    }
}

/// Coefficients of the cubic through (`x0`, `y0`) and (`x1`, `y1`) with end
/// slopes `slope0` and `slope1`, as `(a, b, c, d)`.
fn cubic_coefficients(x0: f32, x1: f32, y0: f32, y1: f32, slope0: f32, slope1: f32) -> (f32, f32, f32, f32) {
    let inverse = 1.0 / (x1 - x0);
    let inverse_squared = inverse * inverse;
    let x1_squared = x1 * x1;
    let x0_squared = x0 * x0;
    let second_derivative0 = -2.0 * (slope1 + 2.0 * slope0) * inverse + 6.0 * (y1 - y0) * inverse_squared;
    let second_derivative1 = 2.0 * (2.0 * slope1 + slope0) * inverse - 6.0 * (y1 - y0) * inverse_squared;
    let a = (second_derivative1 - second_derivative0) / 6.0 * inverse;
    let b = (x1 * second_derivative0 - x0 * second_derivative1) / 2.0 * inverse;
    let c = (y1 - y0 - b * (x1_squared - x0_squared) - a * (x1 * x1_squared - x0 * x0_squared)) * inverse;
    let d = y0 - c * x0 - b * x0_squared - a * x0_squared * x0;
    (a, b, c, d)
}

/// A constrained cubic spline over nodes at 0, 1, 2, …
#[derive(Clone, Debug, Default)]
pub struct RegularSpline {
    segments: Vec<Segment>,
}

impl RegularSpline {
    /// `Rebuild(data)` with the default constrained coefficients.
    pub fn constrained(values: &[f32]) -> Self {
        let mut segments: Vec<Segment> = values.iter().map(|&value| Segment { value, ..Segment::default() }).collect();
        let count = segments.len();
        for segment in 0..count.saturating_sub(1) {
            let previous = segment.saturating_sub(1);
            let next = (segment + 1).min(count - 1);
            let after_next = (segment + 2).min(count - 1);
            let value0 = segments[previous].value;
            let value1 = segments[segment].value;
            let value2 = segments[next].value;
            let value3 = segments[after_next].value;

            let (slope1, slope2) = if count > 2 {
                let rising_or_falling1 = (value1 > value0 && value1 < value2) || (value1 < value0 && value1 > value2);
                let rising_or_falling2 = (value2 > value1 && value2 < value3) || (value2 < value1 && value2 > value3);
                let mut slope1 = if rising_or_falling1 { 2.0 / (1.0 / (value2 - value1) + 1.0 / (value1 - value0)) } else { 0.0 };
                let mut slope2 = if rising_or_falling2 { 2.0 / (1.0 / (value3 - value2) + 1.0 / (value2 - value1)) } else { 0.0 };
                if segment == 0 {
                    slope1 = 1.5 * (value2 - value1) - 0.5 * slope2;
                }
                if segment == count - 2 {
                    slope2 = 1.5 * (value2 - value1) - 0.5 * slope1;
                }
                (slope1, slope2)
            } else {
                let slope = value2 - value1;
                (slope, slope)
            };

            let (a, b, c, d) = cubic_coefficients(0.0, 1.0, value1, value2, slope1, slope2);
            let target = &mut segments[segment];
            target.a = a;
            target.b = b;
            target.c = c;
            target.d = d;
        }
        Self { segments }
    }

    /// The spline at `position` (in node units), clamped to the last
    /// segment (`Interpolate`).
    pub fn interpolate(&self, position: f32) -> f32 {
        let last_segment = self.segments.len() as f32 - 2.0;
        let clamped = position.min(last_segment).max(0.0);
        let index = clamped as i32 as usize;
        let t = position - index as f32;
        self.segments[index].evaluate(t)
    }
}

/// A node of [`IrregularSpline`] with the cubic of the segment it starts.
#[derive(Clone, Copy, Debug, Default)]
struct IrregularSegment {
    x: f32,
    y: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
}

impl IrregularSegment {
    fn evaluate(&self, position: f32) -> f32 {
        let t = position - self.x;
        ((self.a * t + self.b) * t + self.c) * t + self.d
    }
}

/// [`cubic_coefficients`] in double
/// precision.
fn cubic_coefficients_f64(x0: f64, x1: f64, y0: f64, y1: f64, slope0: f64, slope1: f64) -> (f32, f32, f32, f32) {
    let inverse = 1.0 / (x1 - x0);
    let inverse_squared = inverse * inverse;
    let x1_squared = x1 * x1;
    let x0_squared = x0 * x0;
    let second_derivative0 = -2.0 * (slope1 + 2.0 * slope0) * inverse + 6.0 * (y1 - y0) * inverse_squared;
    let second_derivative1 = 2.0 * (2.0 * slope1 + slope0) * inverse - 6.0 * (y1 - y0) * inverse_squared;
    let a = (second_derivative1 - second_derivative0) / 6.0 * inverse;
    let b = (x1 * second_derivative0 - x0 * second_derivative1) / 2.0 * inverse;
    let c = (y1 - y0 - b * (x1_squared - x0_squared) - a * (x1 * x1_squared - x0 * x0_squared)) * inverse;
    let d = y0 - c * x0 - b * x0_squared - a * x0_squared * x0;
    (a as f32, b as f32, c as f32, d as f32)
}

/// Whether `y1` lies strictly between `y0` and `y2`.
fn is_monotonic(y0: f32, y1: f32, y2: f32) -> bool {
    (y1 > y0 && y1 < y2) || (y1 < y0 && y1 > y2)
}

/// The constrained slope at the middle of three nodes, or 0 at a turn.
fn constrained_slope(left: (f32, f32), middle: (f32, f32), right: (f32, f32)) -> f64 {
    if !is_monotonic(left.1, middle.1, right.1) {
        return 0.0;
    }
    let sum = (right.0 - middle.0) / (right.1 - middle.1) + (middle.0 - left.0) / (middle.1 - left.1);
    2.0 / sum as f64
}

/// `Spline1DIrregular` with its default `Spline1dConstrained`
/// segments: a constrained cubic through arbitrary (x, y) nodes.
#[derive(Clone, Debug, Default)]
pub struct IrregularSpline {
    segments: Vec<IrregularSegment>,
}

impl IrregularSpline {
    /// `RebuildFromUnsortedData`: sorts the nodes by x and fits.
    pub fn from_unsorted(points: &[(f32, f32)]) -> Self {
        let mut segments: Vec<IrregularSegment> = points.iter().map(|&(x, y)| IrregularSegment { x, y, ..IrregularSegment::default() }).collect();
        segments.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let mut spline = Self { segments };
        spline.calculate_coefficients();
        spline
    }

    fn calculate_coefficients(&mut self) {
        let count = self.segments.len();
        for index in 0..count.saturating_sub(1) {
            let point = |at: usize| (self.segments[at].x, self.segments[at].y);
            let previous = point(index.saturating_sub(1));
            let current = point(index);
            let next = point((index + 1).min(count - 1));
            let after = point((index + 2).min(count - 1));
            let (slope0, slope1) = if count > 2 {
                let mut slope0 = constrained_slope(previous, current, next);
                let mut slope1 = constrained_slope(current, next, after);
                let end_slope = 1.5 * (next.1 - current.1) as f64 / (next.0 - current.0) as f64;
                if index == 0 {
                    slope0 = end_slope - 0.5 * slope1;
                }
                if index == count - 2 {
                    slope1 = end_slope - 0.5 * slope0;
                }
                (slope0, slope1)
            } else {
                let slope = ((next.1 - current.1) / (next.0 - current.0)) as f64;
                (slope, slope)
            };
            let width = next.0 - current.0;
            // `x[i] - x[i]` as the spline has it: 0, or NaN for an infinite position.
            #[allow(clippy::eq_op)]
            let start = current.0 - current.0;
            let (a, b, c, d) = cubic_coefficients_f64(start as f64, width as f64, current.1 as f64, next.1 as f64, slope0, slope1);
            let segment = &mut self.segments[index];
            segment.a = a;
            segment.b = b;
            segment.c = c;
            segment.d = d;
        }
    }

    /// The index of the segment the vector `Interpolate` uses for an
    /// already clamped position.
    fn segment_index(&self, position: f32) -> usize {
        let count = self.segments.len();
        if count < 50 {
            let mut index = 0;
            while self.segments[index + 1].x < position {
                index += 1;
            }
            return index;
        }
        let mut low = 0usize;
        let mut high = count - 1;
        let mut middle = (low + high) >> 1;
        loop {
            if position < self.segments[middle].x {
                high = middle;
            } else {
                low = middle;
            }
            middle = (low + high) >> 1;
            if middle == low {
                return middle;
            }
        }
    }

    /// `Interpolate(Vector<float>)`, one lane: the position clamped to the
    /// nodes' range (SSE min/max order), then the segment's cubic.
    pub fn interpolate(&self, position: f32) -> f32 {
        let first = self.segments[0].x;
        let last = self.segments[self.segments.len() - 1].x;
        let below_last = if position < last { position } else { last };
        let clamped = if below_last > first { below_last } else { first };
        let index = self.segment_index(clamped);
        self.segments[index].evaluate(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_its_nodes() {
        let spline = RegularSpline::constrained(&[0.0, 0.5, 0.75, 1.0]);
        for (index, expected) in [0.0f32, 0.5, 0.75].iter().enumerate() {
            let value = spline.interpolate(index as f32);
            assert!((value - expected).abs() < 1e-6, "node {index}: {value}");
        }
    }
}

/// `Polyline1DIrregular`: piecewise-linear interpolation through points
/// sorted by position (with .NET's unstable sort).
#[derive(Clone, Debug)]
pub struct Polyline {
    points: Vec<[f32; 2]>,
}

impl Polyline {
    /// `RebuildFromUnsortedData`.
    pub fn from_unsorted(points: &[[f32; 2]]) -> Self {
        let mut sorted = points.to_vec();
        let by_position = |a: &[f32; 2], b: &[f32; 2]| {
            if a[0] > b[0] {
                std::cmp::Ordering::Greater
            } else if a[0] < b[0] {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        };
        crate::dotnet_sort::introspective_sort(&mut sorted, &by_position);
        Self { points: sorted }
    }

    /// Linear interpolation on segment `index`.
    fn segment(&self, index: usize, position: f32) -> f32 {
        let start = self.points[index];
        let end = self.points[index + 1];
        let t = (position - start[0]) / (end[0] - start[0] + 1e-24);
        start[1] * (1.0 - t) + end[1] * t
    }

    /// `Interpolate(Vector<float>)` for one lane: clamped to the end points,
    /// then a linear search (fewer than 50 points) or a binary search.
    pub fn interpolate(&self, position: f32) -> f32 {
        let count = self.points.len();
        let last = self.points[count - 1][0];
        let first = self.points[0][0];
        let lower = if position < last { position } else { last };
        let position = if lower > first { lower } else { first };
        if count < 50 {
            let mut index = 0usize;
            while self.points[index + 1][0] < position {
                index += 1;
            }
            return self.segment(index, position);
        }
        let mut low = 0usize;
        let mut high = count - 1;
        let mut middle = (low + high) >> 1;
        while middle != low {
            if position < self.points[middle][0] {
                high = middle;
            } else {
                low = middle;
            }
            middle = (low + high) >> 1;
        }
        self.segment(middle, position)
    }
}
