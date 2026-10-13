//! The Delaunay triangulation `MeshRasterizer_RenderPointCloud` runs on the
//! host: delaunator-cpp (delfrrr, October 2018) as
//! clang compiled it with fast-math, and MSVC's `std::sort`.
//!
//! The arithmetic follows the binary, not the source: products fuse into
//! FMAs differently at each site, the circumradius and circumcentre multiply
//! by a reciprocal, and the in-circle test is rearranged. The sort's
//! distance comparison is inlined with three different FMA patterns, so the
//! comparator is not perfectly consistent and MSVC's introsort has to be
//! reproduced step by step.

const INVALID: usize = usize::MAX;

/// The triangles (three point indices each), or `None` where delaunator
/// throws "not triangulation".
pub fn triangulate(coords: &[f64]) -> Option<Vec<usize>> {
    if coords.len() < 2 {
        return None;
    }
    let mut triangulation = Triangulation::new(coords);
    triangulation.run()?;
    Some(triangulation.triangles)
}

/// `std::llround` as UCRT evaluates it: values outside `i64` (and NaN) give
/// `i64::MIN`.
fn llround(value: f64) -> i64 {
    let in_range = (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&value);
    if !in_range {
        return i64::MIN;
    }
    value.round() as i64
}

fn fast_mod(value: usize, modulus: usize) -> usize {
    if value >= modulus { value % modulus } else { value }
}

/// `MAXSD`/`MINSD`: the first operand only when strictly greater/smaller.
fn maxsd(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

fn minsd(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

/// The sort's view of a point: its offset from the seed circumcentre.
struct SortKey<'a> {
    coords: &'a [f64],
    center_x: f64,
    center_y: f64,
}

#[derive(PartialEq)]
enum Comparison {
    /// The inserted (or middle) element's distance as `dx² + dy²` with
    /// `dx²` fused, the other's with `dy²` fused (insertion sort, the first
    /// and third tests of the median of three).
    Asymmetric,
    /// Both distances with `dx²` fused (partitioning, the second
    /// median-of-three test).
    Symmetric,
    /// `dxa² + (dya² − db)` in two fused steps (heap sort).
    Heap,
}

impl SortKey<'_> {
    fn offset(&self, index: usize) -> (f64, f64) {
        let dx = self.coords[2 * index] - self.center_x;
        let dy = self.coords[2 * index + 1] - self.center_y;
        (dx, dy)
    }

    fn distance_x_fused(&self, index: usize) -> f64 {
        let (dx, dy) = self.offset(index);
        dx.mul_add(dx, dy * dy)
    }

    fn distance_y_fused(&self, index: usize) -> f64 {
        let (dx, dy) = self.offset(index);
        dy.mul_add(dy, dx * dx)
    }

    /// delaunator's `compare` (`a` before `b`): nearer the centre first,
    /// then smaller x, then smaller y.
    fn less(&self, a: usize, b: usize, comparison: Comparison) -> bool {
        let difference = match comparison {
            Comparison::Asymmetric => self.distance_x_fused(a) - self.distance_y_fused(b),
            Comparison::Symmetric => self.distance_x_fused(a) - self.distance_x_fused(b),
            Comparison::Heap => {
                let (dx, dy) = self.offset(a);
                let inner = dy.mul_add(dy, -self.distance_x_fused(b));
                dx.mul_add(dx, inner)
            }
        };
        // UCOMISD: an unordered difference counts as a tie.
        let decided = difference != 0.0 && !difference.is_nan();
        if decided {
            return difference < 0.0;
        }
        let x_difference = self.coords[2 * a] - self.coords[2 * b];
        let x_decided = x_difference != 0.0 && !x_difference.is_nan();
        if x_decided {
            return x_difference < 0.0;
        }
        let ya = self.coords[2 * a + 1];
        let yb = self.coords[2 * b + 1];
        !(ya >= yb)
    }
}

/// MSVC's `std::sort` (`_Sort_unchecked`): introsort with insertion sort up
/// to 32 elements, Tukey's ninther above 40 and heap sort once the depth
/// budget (×¾ per level) runs out.
fn msvc_sort(ids: &mut [usize], key: &SortKey) {
    let length = ids.len() as isize;
    sort_unchecked(ids, 0, length, length, key);
}

fn sort_unchecked(ids: &mut [usize], mut first: isize, mut last: isize, mut ideal: isize, key: &SortKey) {
    loop {
        if last - first <= 32 {
            insertion_sort(ids, first, last, key);
            return;
        }
        if ideal <= 0 {
            make_heap(ids, first, last, key);
            sort_heap(ids, first, last, key);
            return;
        }
        let (middle_first, middle_last) = partition_by_median_guess(ids, first, last, key);
        ideal = (ideal >> 1) + (ideal >> 2);
        if middle_first - first < last - middle_last {
            sort_unchecked(ids, first, middle_first, ideal, key);
            first = middle_last;
        } else {
            sort_unchecked(ids, middle_last, last, ideal, key);
            last = middle_first;
        }
    }
}

fn at(ids: &[usize], index: isize) -> usize {
    ids[index as usize]
}

fn insertion_sort(ids: &mut [usize], first: isize, last: isize, key: &SortKey) {
    if first == last {
        return;
    }
    for middle in first + 1..last {
        let value = at(ids, middle);
        if key.less(value, at(ids, first), Comparison::Asymmetric) {
            ids.copy_within(first as usize..middle as usize, first as usize + 1);
            ids[first as usize] = value;
            continue;
        }
        let mut hole = middle;
        while key.less(value, at(ids, hole - 1), Comparison::Asymmetric) {
            ids[hole as usize] = ids[hole as usize - 1];
            hole -= 1;
        }
        ids[hole as usize] = value;
    }
}

fn median_of_three(ids: &mut [usize], first: isize, middle: isize, last: isize, key: &SortKey) {
    if key.less(at(ids, middle), at(ids, first), Comparison::Asymmetric) {
        ids.swap(middle as usize, first as usize);
    }
    if key.less(at(ids, last), at(ids, middle), Comparison::Symmetric) {
        ids.swap(last as usize, middle as usize);
        if key.less(at(ids, middle), at(ids, first), Comparison::Asymmetric) {
            ids.swap(middle as usize, first as usize);
        }
    }
}

fn guess_median(ids: &mut [usize], first: isize, middle: isize, last: isize, key: &SortKey) {
    let count = last - first;
    if 40 < count {
        let step = (count + 1) >> 3;
        let two_steps = step << 1;
        median_of_three(ids, first, first + step, first + two_steps, key);
        median_of_three(ids, middle - step, middle, middle + step, key);
        median_of_three(ids, last - two_steps, last - step, last, key);
        median_of_three(ids, first + step, middle, last - step, key);
    } else {
        median_of_three(ids, first, middle, last, key);
    }
}

fn partition_by_median_guess(ids: &mut [usize], first: isize, last: isize, key: &SortKey) -> (isize, isize) {
    let middle = first + ((last - first) >> 1);
    guess_median(ids, first, middle, last - 1, key);
    let mut pivot_first = middle;
    let mut pivot_last = pivot_first + 1;
    let less = |ids: &[usize], a: isize, b: isize| key.less(at(ids, a), at(ids, b), Comparison::Symmetric);

    while first < pivot_first && !less(ids, pivot_first - 1, pivot_first) && !less(ids, pivot_first, pivot_first - 1) {
        pivot_first -= 1;
    }
    while pivot_last < last && !less(ids, pivot_last, pivot_first) && !less(ids, pivot_first, pivot_last) {
        pivot_last += 1;
    }

    let mut greater_first = pivot_last;
    let mut greater_last = pivot_first;
    loop {
        while greater_first < last {
            if less(ids, pivot_first, greater_first) {
                greater_first += 1;
                continue;
            }
            if less(ids, greater_first, pivot_first) {
                break;
            }
            if pivot_last != greater_first {
                ids.swap(pivot_last as usize, greater_first as usize);
            }
            pivot_last += 1;
            greater_first += 1;
        }
        while first < greater_last {
            let previous = greater_last - 1;
            if less(ids, previous, pivot_first) {
                greater_last -= 1;
                continue;
            }
            if less(ids, pivot_first, previous) {
                break;
            }
            pivot_first -= 1;
            if pivot_first != previous {
                ids.swap(pivot_first as usize, previous as usize);
            }
            greater_last -= 1;
        }

        if greater_last == first && greater_first == last {
            return (pivot_first, pivot_last);
        }
        if greater_last == first {
            if pivot_last != greater_first {
                ids.swap(pivot_first as usize, pivot_last as usize);
            }
            pivot_last += 1;
            ids.swap(pivot_first as usize, greater_first as usize);
            pivot_first += 1;
            greater_first += 1;
        } else if greater_first == last {
            greater_last -= 1;
            pivot_first -= 1;
            if greater_last != pivot_first {
                ids.swap(greater_last as usize, pivot_first as usize);
            }
            pivot_last -= 1;
            ids.swap(pivot_first as usize, pivot_last as usize);
        } else {
            greater_last -= 1;
            ids.swap(greater_first as usize, greater_last as usize);
            greater_first += 1;
        }
    }
}

fn make_heap(ids: &mut [usize], first: isize, last: isize, key: &SortKey) {
    let bottom = last - first;
    let mut hole = bottom >> 1;
    while hole > 0 {
        hole -= 1;
        let value = at(ids, first + hole);
        pop_heap_hole_by_index(ids, first, hole, bottom, value, key);
    }
}

fn sort_heap(ids: &mut [usize], first: isize, mut last: isize, key: &SortKey) {
    while last - first >= 2 {
        last -= 1;
        let value = at(ids, last);
        ids[last as usize] = at(ids, first);
        pop_heap_hole_by_index(ids, first, 0, last - first, value, key);
    }
}

fn pop_heap_hole_by_index(ids: &mut [usize], first: isize, mut hole: isize, bottom: isize, value: usize, key: &SortKey) {
    let top = hole;
    let mut index = hole;
    let max_non_leaf = (bottom - 1) >> 1;
    while index < max_non_leaf {
        index = 2 * index + 2;
        if key.less(at(ids, first + index), at(ids, first + index - 1), Comparison::Heap) {
            index -= 1;
        }
        ids[(first + hole) as usize] = at(ids, first + index);
        hole = index;
    }
    if index == max_non_leaf && bottom % 2 == 0 {
        ids[(first + hole) as usize] = at(ids, first + bottom - 1);
        hole = bottom - 1;
    }
    push_heap_by_index(ids, first, hole, top, value, key);
}

fn push_heap_by_index(ids: &mut [usize], first: isize, mut hole: isize, top: isize, value: usize, key: &SortKey) {
    let mut index = (hole - 1) >> 1;
    while top < hole && key.less(at(ids, first + index), value, Comparison::Heap) {
        ids[(first + hole) as usize] = at(ids, first + index);
        hole = index;
        index = (hole - 1) >> 1;
    }
    ids[(first + hole) as usize] = value;
}

/// `check_pts_equal`.
fn points_equal(x1: f64, y1: f64, x2: f64, y2: f64) -> bool {
    (x1 - x2).abs() <= f64::EPSILON && (y1 - y2).abs() <= f64::EPSILON
}

/// `orient(p, q, r)` as compiled: the two products compared directly.
fn orient(px: f64, py: f64, qx: f64, qy: f64, rx: f64, ry: f64) -> bool {
    let first = (rx - qx) * (qy - py);
    let second = (qx - px) * (ry - qy);
    first < second
}

struct Triangulation<'a> {
    coords: &'a [f64],
    triangles: Vec<usize>,
    halfedges: Vec<usize>,
    hull_prev: Vec<usize>,
    hull_next: Vec<usize>,
    hull_tri: Vec<usize>,
    hull_start: usize,
    hash: Vec<usize>,
    center_x: f64,
    center_y: f64,
    hash_size: usize,
    edge_stack: Vec<usize>,
}

impl<'a> Triangulation<'a> {
    fn new(coords: &'a [f64]) -> Self {
        Triangulation {
            coords,
            triangles: Vec::new(),
            halfedges: Vec::new(),
            hull_prev: Vec::new(),
            hull_next: Vec::new(),
            hull_tri: Vec::new(),
            hull_start: 0,
            hash: Vec::new(),
            center_x: 0.0,
            center_y: 0.0,
            hash_size: 0,
            edge_stack: Vec::new(),
        }
    }

    fn x(&self, index: usize) -> f64 {
        self.coords[2 * index]
    }

    fn y(&self, index: usize) -> f64 {
        self.coords[2 * index + 1]
    }

    /// `hash_key`: the pseudo-angle about the centre, scaled to the table.
    fn hash_key(&self, x: f64, y: f64) -> usize {
        let dx = x - self.center_x;
        let dy = y - self.center_y;
        let p = dx / (dy.abs() + dx.abs());
        let angle = if 0.0 < dy { 3.0 - p } else { p + 1.0 };
        let scaled = (self.hash_size as f64 * 0.25) * angle;
        let key = llround(scaled.floor()) as usize;
        fast_mod(key, self.hash_size)
    }

    fn link(&mut self, a: usize, b: usize) {
        let size = self.halfedges.len();
        if a == size {
            self.halfedges.push(b);
        } else if a < size {
            self.halfedges[a] = b;
        } else {
            panic!("Cannot link edge");
        }
        if b != INVALID {
            let size = self.halfedges.len();
            if b == size {
                self.halfedges.push(a);
            } else if b < size {
                self.halfedges[b] = a;
            } else {
                panic!("Cannot link edge");
            }
        }
    }

    fn add_triangle(&mut self, i0: usize, i1: usize, i2: usize, a: usize, b: usize, c: usize) -> usize {
        let t = self.triangles.len();
        self.triangles.push(i0);
        self.triangles.push(i1);
        self.triangles.push(i2);
        self.link(t, a);
        self.link(t + 1, b);
        self.link(t + 2, c);
        t
    }

    /// The in-circle test as compiled: `p1` inside the circle through
    /// `p0`, `pr`, `pl`.
    fn illegal(&self, p0: usize, pr: usize, pl: usize, p1: usize) -> bool {
        let px = self.x(p1);
        let py = self.y(p1);
        let dx = self.x(p0) - px;
        let dy = self.y(p0) - py;
        let ex = self.x(pr) - px;
        let ey = self.y(pr) - py;
        let fx = self.x(pl) - px;
        let fy = self.y(pl) - py;
        let ap = dx.mul_add(dx, dy * dy);
        let bp = ex.mul_add(ex, ey * ey);
        let cp = fx.mul_add(fx, fy * fy);
        let flipped_first = (-cp).mul_add(ey, bp * fy);
        let second = ex.mul_add(cp, -(bp * fx));
        let third = ex.mul_add(fy, -(ey * fx));
        let left = (-dx).mul_add(flipped_first, ap * third);
        let right = second * dy;
        left < right
    }

    fn legalize(&mut self, mut a: usize) -> usize {
        let mut depth = 0;
        let mut ar;
        self.edge_stack.clear();
        loop {
            let b = self.halfedges[a];
            let a0 = 3 * (a / 3);
            ar = a0 + (a + 2) % 3;
            let flip = b != INVALID && {
                let b0 = 3 * (b / 3);
                let al = a0 + (a + 1) % 3;
                let bl = b0 + (b + 2) % 3;
                self.illegal(self.triangles[ar], self.triangles[a], self.triangles[al], self.triangles[bl])
            };
            if !flip {
                if depth > 0 {
                    depth -= 1;
                    a = self.edge_stack[depth];
                    continue;
                }
                break;
            }
            let b0 = 3 * (b / 3);
            let bl = b0 + (b + 2) % 3;
            let p0 = self.triangles[ar];
            let p1 = self.triangles[bl];
            self.triangles[a] = p1;
            self.triangles[b] = p0;
            let hbl = self.halfedges[bl];
            if hbl == INVALID {
                let mut e = self.hull_start;
                loop {
                    if self.hull_tri[e] == bl {
                        self.hull_tri[e] = a;
                        break;
                    }
                    e = self.hull_next[e];
                    if e == self.hull_start {
                        break;
                    }
                }
            }
            self.link(a, hbl);
            let har = self.halfedges[ar];
            self.link(b, har);
            self.link(ar, bl);
            let br = b0 + (b + 1) % 3;
            if depth < self.edge_stack.len() {
                self.edge_stack[depth] = br;
            } else {
                self.edge_stack.push(br);
            }
            depth += 1;
        }
        ar
    }

    fn run(&mut self) -> Option<()> {
        let n = self.coords.len() >> 1;

        let mut max_x = f64::MIN_POSITIVE;
        let mut max_y = f64::MIN_POSITIVE;
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        for i in 0..n {
            let x = self.x(i);
            let y = self.y(i);
            max_y = maxsd(y, max_y);
            max_x = maxsd(x, max_x);
            min_y = minsd(y, min_y);
            min_x = minsd(x, min_x);
        }
        let cx = (max_x + min_x) * 0.5;
        let cy = (max_y + min_y) * 0.5;

        // The seed: the point nearest the bounds' centre.
        let mut i0 = INVALID;
        let mut min_distance = f64::MAX;
        for i in 0..n {
            let dx = cx - self.x(i);
            let dy = cy - self.y(i);
            let distance = dx.mul_add(dx, dy * dy);
            if distance < min_distance {
                i0 = i;
                min_distance = distance;
            }
        }
        let i0x = self.x(i0);
        let i0y = self.y(i0);

        // The point nearest the seed.
        let mut i1 = INVALID;
        min_distance = f64::MAX;
        for i in 0..n {
            if i == i0 {
                continue;
            }
            let dx = i0x - self.x(i);
            let dy = i0y - self.y(i);
            let distance = dx.mul_add(dx, dy * dy);
            if distance < min_distance && distance > 0.0 {
                i1 = i;
                min_distance = distance;
            }
        }
        if i1 == INVALID {
            return None;
        }
        let mut i1x = self.x(i1);
        let mut i1y = self.y(i1);

        // The third point: the smallest circumcircle with the first two.
        let dx = i1x - i0x;
        let dy = i1y - i0y;
        let bl = dy.mul_add(dy, dx * dx);
        let mut i2 = INVALID;
        let mut min_radius = f64::MAX;
        for i in 0..n {
            if i == i0 || i == i1 {
                continue;
            }
            let ex = self.x(i) - i0x;
            let ey = self.y(i) - i0y;
            let cl = ex.mul_add(ex, ey * ey);
            let d = ey.mul_add(dx, -(ex * dy));
            let mut radius = f64::MAX;
            if d != 0.0 && !d.is_nan() {
                let inverse = 1.0 / d;
                let x = inverse * (bl.mul_add(ey, -(cl * dy)) * 0.5);
                let y = inverse * (dx.mul_add(cl, -(ex * bl)) * 0.5);
                radius = x.mul_add(x, y * y);
            }
            if !(cl > 0.0) {
                radius = f64::MAX;
            }
            if bl <= 0.0 {
                radius = f64::MAX;
            }
            if radius < min_radius {
                i2 = i;
                min_radius = radius;
            }
        }
        if !(min_radius < f64::MAX) {
            return None;
        }
        let mut i2x = self.x(i2);
        let mut i2y = self.y(i2);

        if orient(i0x, i0y, i1x, i1y, i2x, i2y) {
            std::mem::swap(&mut i1, &mut i2);
            std::mem::swap(&mut i1x, &mut i2x);
            std::mem::swap(&mut i1y, &mut i2y);
        }

        // The seed triangle's circumcentre.
        let dx = i1x - i0x;
        let dy = i1y - i0y;
        let ex = i2x - i0x;
        let ey = i2y - i0y;
        let bl = dx.mul_add(dx, dy * dy);
        let cl = ex.mul_add(ex, ey * ey);
        let negated_d = (-dx).mul_add(ey, dy * ex);
        let inverse = -1.0 / negated_d;
        self.center_x = (bl.mul_add(ey, -(cl * dy)) * 0.5).mul_add(inverse, i0x);
        self.center_y = (dx.mul_add(cl, -(bl * ex)) * 0.5).mul_add(inverse, i0y);

        let mut ids: Vec<usize> = (0..n).collect();
        let key = SortKey { coords: self.coords, center_x: self.center_x, center_y: self.center_y };
        msvc_sort(&mut ids, &key);

        self.hash_size = llround((n as i64 as f64).sqrt().ceil()) as usize;
        self.hash = vec![INVALID; self.hash_size];
        self.hull_prev = vec![0; n];
        self.hull_next = vec![0; n];
        self.hull_tri = vec![0; n];
        self.hull_start = i0;
        self.hull_next[i0] = i1;
        self.hull_prev[i2] = i1;
        self.hull_next[i1] = i2;
        self.hull_prev[i0] = i2;
        self.hull_next[i2] = i0;
        self.hull_prev[i1] = i0;
        self.hull_tri[i0] = 0;
        self.hull_tri[i1] = 1;
        self.hull_tri[i2] = 2;
        let key0 = self.hash_key(i0x, i0y);
        self.hash[key0] = i0;
        let key1 = self.hash_key(i1x, i1y);
        self.hash[key1] = i1;
        let key2 = self.hash_key(i2x, i2y);
        self.hash[key2] = i2;

        self.add_triangle(i0, i1, i2, INVALID, INVALID, INVALID);
        let mut xp = f64::NAN;
        let mut yp = f64::NAN;
        for (k, &i) in ids.iter().enumerate().take(n) {
            let x = self.x(i);
            let y = self.y(i);
            if k > 0 && points_equal(x, y, xp, yp) {
                continue;
            }
            xp = x;
            yp = y;
            let seed_point = points_equal(x, y, i0x, i0y) || points_equal(x, y, i1x, i1y) || points_equal(x, y, i2x, i2y);
            if seed_point {
                continue;
            }

            // A visible edge of the hull, found through the hash.
            let mut start = 0;
            let key = self.hash_key(x, y);
            for j in 0..self.hash_size {
                start = self.hash[fast_mod(key + j, self.hash_size)];
                if start != INVALID && start != self.hull_next[start] {
                    break;
                }
            }
            start = self.hull_prev[start];
            let mut e = start;
            loop {
                let q = self.hull_next[e];
                if orient(x, y, self.x(e), self.y(e), self.x(q), self.y(q)) {
                    break;
                }
                e = q;
                if e == start {
                    e = INVALID;
                    break;
                }
            }
            if e == INVALID {
                continue;
            }

            let next_of_e = self.hull_next[e];
            let tri_of_e = self.hull_tri[e];
            let t = self.add_triangle(e, i, next_of_e, INVALID, INVALID, tri_of_e);
            self.hull_tri[i] = self.legalize(t + 2);
            self.hull_tri[e] = t;

            // Forward along the hull.
            let mut next = self.hull_next[e];
            loop {
                let q = self.hull_next[next];
                if !orient(x, y, self.x(next), self.y(next), self.x(q), self.y(q)) {
                    break;
                }
                let tri_of_i = self.hull_tri[i];
                let tri_of_next = self.hull_tri[next];
                let t = self.add_triangle(next, i, q, tri_of_i, INVALID, tri_of_next);
                self.hull_tri[i] = self.legalize(t + 2);
                self.hull_next[next] = next;
                next = q;
            }

            // Backward from the other side.
            if e == start {
                loop {
                    let q = self.hull_prev[e];
                    if !orient(x, y, self.x(q), self.y(q), self.x(e), self.y(e)) {
                        break;
                    }
                    let tri_of_e = self.hull_tri[e];
                    let tri_of_q = self.hull_tri[q];
                    let t = self.add_triangle(q, i, e, INVALID, tri_of_e, tri_of_q);
                    self.legalize(t + 2);
                    self.hull_tri[q] = t;
                    self.hull_next[e] = e;
                    e = q;
                }
            }

            self.hull_prev[i] = e;
            self.hull_start = e;
            self.hull_prev[next] = i;
            self.hull_next[e] = i;
            self.hull_next[i] = next;
            let key_i = self.hash_key(x, y);
            self.hash[key_i] = i;
            let key_e = self.hash_key(self.x(e), self.y(e));
            self.hash[key_e] = e;
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_splits_into_two_triangles() {
        let coords = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let triangles = triangulate(&coords).unwrap();
        assert_eq!(triangles.len(), 6);
    }

    #[test]
    fn collinear_points_fail() {
        let coords = [0.0, 0.0, 1.0, 0.0, 2.0, 0.0];
        assert!(triangulate(&coords).is_none());
    }

    #[test]
    fn sort_orders_by_distance() {
        let coords: Vec<f64> = (0..200).flat_map(|i| [((i * 37) % 101) as f64 + 0.5, ((i * 53) % 97) as f64 + 0.5]).collect();
        let key = SortKey { coords: &coords, center_x: 50.25, center_y: 48.75 };
        let mut ids: Vec<usize> = (0..200).collect();
        msvc_sort(&mut ids, &key);
        for pair in ids.windows(2) {
            assert!(!key.less(pair[1], pair[0], Comparison::Symmetric));
        }
    }
}
