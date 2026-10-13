//! ImageMagick 7.1.2's colour quantizer (quantize.c) on images of quanta:
//! an octree classifies the colours (red, green and blue, with alpha as a
//! fourth dimension when the image has it), is pruned to the colours
//! wanted, and every pixel takes the nearest remaining colour, the error
//! dithered away Floyd–Steinberg style or along a Hilbert curve
//! (Riemersma).
//!
//! The pixels come in the quantize colourspace already. `QuantizeImage`'s
//! grey and colourspace bookkeeping (`IdentifyImageGray`,
//! `SetGrayscaleImage`, the transforms around classification and
//! assignment) is left to the caller, as it differs from caller to caller.
//! Callers always dither, so `NoDitherMethod`'s plain assignment is not
//! here, nor are the two-colour grey cases (`SetAssociatedAlpha` and the
//! monochrome colormap fix-up), which they never reach.

/// `MaxTreeDepth`.
pub const MAX_TREE_DEPTH: usize = 8;
/// `MaxColormapSize`.
pub const MAX_COLORMAP_SIZE: usize = 65536;
/// `MaxQNodes`.
const MAX_NODES: usize = 266817;
/// `ErrorQueueLength`.
const ERROR_QUEUE_LENGTH: usize = 16;
/// `ErrorRelativeWeight` (`MagickSafeReciprocal(16)`).
const ERROR_RELATIVE_WEIGHT: f64 = 1.0 / 16.0;
/// `exp(log(1/ErrorRelativeWeight)/(ErrorQueueLength-1))`, by which each
/// Riemersma error weight shrinks from the last. Windows' and Apple's C
/// libraries both round it correctly to this value.
const ERROR_WEIGHT_GROWTH: f64 = 1.2030250360821166;
/// `cube_info->diffusion` without a `dither:diffusion-amount` artifact.
const DIFFUSION: f64 = 1.0;
const QUANTUM_RANGE: f64 = 65535.0;
/// `QuantumScale`.
const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
/// `OpaqueAlpha` as a float quantum.
const OPAQUE_ALPHA: f32 = 65535.0;
const MAGICK_EPSILON: f64 = 1.0e-12;
const NO_CHILD: u32 = u32::MAX;
/// The cube's root node.
const ROOT: usize = 0;

/// How the quantization error is spread to neighbouring pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dither {
    FloydSteinberg,
    Riemersma,
}

/// How a pixel's quanta sit in the image data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One grey sample, read as red, green and blue alike.
    Gray,
    Rgb,
    Rgba,
}

impl Layout {
    pub fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }

    pub fn has_alpha(self) -> bool {
        self == Layout::Rgba
    }

    /// A pixel's red, green, blue and alpha (`OpaqueAlpha` without an
    /// alpha channel).
    fn pixel(self, data: &[f32], pixel: usize) -> [f32; 4] {
        match self {
            Layout::Gray => {
                let gray = data[pixel];
                [gray, gray, gray, OPAQUE_ALPHA]
            }
            Layout::Rgb => {
                let start = pixel * 3;
                [data[start], data[start + 1], data[start + 2], OPAQUE_ALPHA]
            }
            Layout::Rgba => {
                let start = pixel * 4;
                [data[start], data[start + 1], data[start + 2], data[start + 3]]
            }
        }
    }

    /// `SetPixelRed`, `SetPixelGreen` and `SetPixelBlue` (and
    /// `SetPixelAlpha` when `with_alpha`) with a colormap colour. A grey
    /// image's colours share one sample, which keeps the last (blue).
    fn set_pixel(self, data: &mut [f32], pixel: usize, color: &Color, with_alpha: bool) {
        match self {
            Layout::Gray => data[pixel] = clamp_to_quantum(color.blue),
            Layout::Rgb | Layout::Rgba => {
                let start = pixel * self.channels();
                data[start] = clamp_to_quantum(color.red);
                data[start + 1] = clamp_to_quantum(color.green);
                data[start + 2] = clamp_to_quantum(color.blue);
                if with_alpha && self == Layout::Rgba {
                    data[start + 3] = clamp_to_quantum(color.alpha);
                }
            }
        }
    }
}

/// A colour in quanta: IM's `DoublePixelPacket`, and the four values of
/// a colormap `PixelInfo` the quantizer uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

/// What `QuantizeImage` works with once it has settled its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantizeOptions {
    /// At most this many colours (see [`maximum_colors`]).
    pub maximum_colors: usize,
    /// The colour tree's depth, clamped to 2..=8 (see
    /// [`automatic_tree_depth`]).
    pub tree_depth: usize,
    pub dither: Dither,
    /// `CacheShift`, the coarseness of the dither colour cache: 2 in
    /// Windows builds of ImageMagick, 3 in Apple ones.
    pub cache_shift: u32,
}

/// `QuantizeImage`'s colour count: 0, or more than `MaxColormapSize`,
/// means `MaxColormapSize`.
pub fn maximum_colors(number_colors: usize) -> usize {
    if number_colors == 0 || number_colors > MAX_COLORMAP_SIZE {
        return MAX_COLORMAP_SIZE;
    }
    number_colors
}

/// `QuantizeImage`'s tree depth when it is asked for none: Log4 of the
/// colours plus 2, one less as it dithers and one less again (above 5)
/// with alpha; grey images always take the full depth.
pub fn automatic_tree_depth(maximum_colors: usize, has_alpha: bool, gray: bool) -> usize {
    let mut colors = maximum_colors;
    let mut depth = 1;
    while colors != 0 {
        colors >>= 2;
        depth += 1;
    }
    if depth > 2 {
        depth -= 1;
    }
    if has_alpha && depth > 5 {
        depth -= 1;
    }
    if gray {
        depth = MAX_TREE_DEPTH;
    }
    depth
}

/// `MagickSafeReciprocal`.
fn safe_reciprocal(value: f64) -> f64 {
    if value > -MAGICK_EPSILON && value < MAGICK_EPSILON {
        return 1.0 / MAGICK_EPSILON;
    }
    1.0 / value
}

/// `ClampPixel`: a value clamped to a float quantum.
fn clamp_pixel(value: f64) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value >= QUANTUM_RANGE {
        return QUANTUM_RANGE as f32;
    }
    value as f32
}

/// `ClampToQuantum`, which an HDRI build leaves unclamped.
fn clamp_to_quantum(value: f64) -> f32 {
    value as f32
}

/// `ScaleQuantumToChar` (Q16 HDRI), in single precision.
fn quantum_to_char(quantum: f32) -> usize {
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    let scaled = quantum / 257.0;
    if scaled >= 255.0 {
        return 255;
    }
    (scaled + 0.5) as usize
}

/// `IsPixelEquivalent` between a run's first pixel and another: colours
/// within `MagickEpsilon`, and any two transparent pixels alike.
fn equivalent(layout: Layout, first: &[f32; 4], other: &[f32; 4]) -> bool {
    let (alpha, beta) = if layout.has_alpha() {
        (first[3] as f64, other[3] as f64)
    } else {
        (QUANTUM_RANGE, QUANTUM_RANGE)
    };
    if (alpha - beta).abs() >= MAGICK_EPSILON {
        return false;
    }
    if alpha.abs() < MAGICK_EPSILON || beta.abs() < MAGICK_EPSILON {
        return true;
    }
    let red_close = (first[0] as f64 - other[0] as f64).abs() < MAGICK_EPSILON;
    let green_close = (first[1] as f64 - other[1] as f64).abs() < MAGICK_EPSILON;
    let blue_close = (first[2] as f64 - other[2] as f64).abs() < MAGICK_EPSILON;
    red_close && green_close && blue_close
}

/// How many pixels from `x` on in a row of `width` starting at
/// `row_start` match the first (the runs classification takes in one go).
fn run_length(data: &[f32], layout: Layout, row_start: usize, x: usize, width: usize) -> usize {
    let first = layout.pixel(data, row_start + x);
    let mut count = 1;
    while x + count < width {
        let other = layout.pixel(data, row_start + x + count);
        if !equivalent(layout, &first, &other) {
            break;
        }
        count += 1;
    }
    count
}

/// One cube of the colour octree (`QNodeInfo`).
struct Node {
    parent: usize,
    children: [u32; 16],
    number_unique: u64,
    total_color: Color,
    quantize_error: f64,
    color_number: usize,
    id: usize,
    level: usize,
}

/// `ClosestColor`'s state: the colour sought, the smallest distance so
/// far and its colormap entry. The entry carries over from search to
/// search as the cube's `color_number` does.
struct Search {
    target: Color,
    distance: f64,
    color_number: usize,
}

/// A step along the Hilbert curve (IM's gravities).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    West,
    East,
    North,
    South,
    Forget,
}

/// The colour cube (`QCubeInfo`) of an image, from classification to
/// assignment.
pub struct Quantizer {
    width: usize,
    height: usize,
    layout: Layout,
    associate_alpha: bool,
    dither: Dither,
    cache_shift: u32,
    nodes: Vec<Node>,
    /// `cube_info->nodes`: nodes in the tree (pruned ones leave it).
    live_nodes: usize,
    colors: usize,
    maximum_colors: usize,
    depth: usize,
    pruning_threshold: f64,
    next_threshold: f64,
    colormap: Vec<Color>,
}

/// `QuantizeImage` once its options are settled, on pixels in the
/// quantize colourspace: returns the colormap, every pixel holding its
/// colormap colour (as `SyncImage` leaves them).
pub fn quantize(data: &mut [f32], width: usize, height: usize, layout: Layout, options: &QuantizeOptions) -> Vec<Color> {
    let quantizer = Quantizer::classify(data, width, height, layout, options);
    quantizer.assign(data)
}

/// `QuantizeImage` on a one-channel image of quanta already in the
/// quantize colourspace: at most `number_colors` (0 or more than 65536
/// meaning 65536) grey levels from a tree of `tree_depth` levels (0 for
/// IM's choice, which for grey images is the full depth), dithered with
/// `dither`.
pub fn quantize_gray(data: &mut [f32], width: usize, height: usize, number_colors: usize, tree_depth: usize, dither: Dither, cache_shift: u32) {
    let maximum_colors = maximum_colors(number_colors);
    let tree_depth = if tree_depth == 0 { MAX_TREE_DEPTH } else { tree_depth };
    let options = QuantizeOptions {
        maximum_colors,
        tree_depth,
        dither,
        cache_shift,
    };
    quantize(data, width, height, Layout::Gray, &options);
}

impl Quantizer {
    /// `GetQCubeInfo`, `ClassifyImageColors` and, when there are too many
    /// colours, `ReduceImageColors`. `data` is read, not changed.
    pub fn classify(data: &[f32], width: usize, height: usize, layout: Layout, options: &QuantizeOptions) -> Self {
        assert_eq!(data.len(), width * height * layout.channels(), "image data does not match its size");
        let mut quantizer = Self {
            width,
            height,
            layout,
            associate_alpha: layout.has_alpha(),
            dither: options.dither,
            cache_shift: options.cache_shift,
            nodes: Vec::new(),
            live_nodes: 0,
            colors: 0,
            maximum_colors: options.maximum_colors,
            depth: options.tree_depth.clamp(2, MAX_TREE_DEPTH),
            pruning_threshold: 0.0,
            next_threshold: 0.0,
            colormap: Vec::new(),
        };
        // The root is its own parent.
        quantizer.add_node(0, 0, ROOT);
        quantizer.classify_image(data);
        if quantizer.colors > quantizer.maximum_colors {
            quantizer.reduce_colors();
        }
        quantizer
    }

    /// `AssignImageColors` (after any colourspace change the caller makes
    /// first): `SetImageColormap`, then every pixel set to its colormap
    /// colour as the dither carries the error on. Returns the colormap.
    pub fn assign(mut self, data: &mut [f32]) -> Vec<Color> {
        assert_eq!(data.len(), self.width * self.height * self.layout.channels(), "image data does not match its size");
        self.define_colormap(ROOT);
        match self.dither {
            Dither::FloydSteinberg => self.floyd_steinberg(data),
            Dither::Riemersma => self.riemersma(data),
        }
        self.colormap
    }

    fn child_count(&self) -> usize {
        if self.associate_alpha { 16 } else { 8 }
    }

    /// `GetQNodeInfo`.
    fn add_node(&mut self, id: usize, level: usize, parent: usize) -> usize {
        self.live_nodes += 1;
        self.nodes.push(Node {
            parent,
            children: [NO_CHILD; 16],
            number_unique: 0,
            total_color: Color::default(),
            quantize_error: 0.0,
            color_number: 0,
            id,
            level,
        });
        self.nodes.len() - 1
    }

    fn child(&self, node: usize, id: usize) -> Option<usize> {
        let child = self.nodes[node].children[id];
        (child != NO_CHILD).then_some(child as usize)
    }

    /// `AssociateAlphaPixel`: the colour premultiplied by its alpha when
    /// the cube classifies alpha and the pixel is not opaque.
    fn associate_alpha_pixel(&self, pixel: [f32; 4]) -> Color {
        let [red, green, blue, alpha] = pixel;
        if !self.associate_alpha || alpha == OPAQUE_ALPHA {
            return Color {
                red: red as f64,
                green: green as f64,
                blue: blue as f64,
                alpha: alpha as f64,
            };
        }
        let scale = QUANTUM_SCALE * alpha as f64;
        Color {
            red: scale * red as f64,
            green: scale * green as f64,
            blue: scale * blue as f64,
            alpha: alpha as f64,
        }
    }

    /// `AssociateAlphaPixelInfo` for a colormap colour.
    fn associate_alpha_color(&self, color: &Color) -> Color {
        if !self.associate_alpha || color.alpha == QUANTUM_RANGE {
            return *color;
        }
        let scale = QUANTUM_SCALE * color.alpha;
        Color {
            red: scale * color.red,
            green: scale * color.green,
            blue: scale * color.blue,
            alpha: color.alpha,
        }
    }

    /// `ColorToQNodeId`: bit `index` of each 8-bit sample.
    fn node_id(&self, pixel: &Color, index: usize) -> usize {
        let red = (quantum_to_char(clamp_pixel(pixel.red)) >> index) & 1;
        let green = (quantum_to_char(clamp_pixel(pixel.green)) >> index) & 1;
        let blue = (quantum_to_char(clamp_pixel(pixel.blue)) >> index) & 1;
        let mut id = red | (green << 1) | (blue << 2);
        if self.associate_alpha {
            id |= ((quantum_to_char(clamp_pixel(pixel.alpha)) >> index) & 1) << 3;
        }
        id
    }

    /// `ClassifyImageColors`: runs of equal pixels down the tree, first to
    /// the full depth, then (once there are too many colours) to the
    /// cube's depth.
    fn classify_image(&mut self, data: &[f32]) {
        let mut full_depth = true;
        for y in 0..self.height {
            if self.live_nodes > MAX_NODES {
                // Prune one level if the colour tree is too large.
                self.prune_level(ROOT);
                self.depth -= 1;
            }
            let row_start = y * self.width;
            let mut x = 0;
            while x < self.width {
                let count = run_length(data, self.layout, row_start, x, self.width);
                let first = self.layout.pixel(data, row_start + x);
                let pixel = self.associate_alpha_pixel(first);
                let levels = if full_depth { MAX_TREE_DEPTH } else { self.depth };
                self.classify_run(&pixel, count, levels);
                x += count;
            }
            if full_depth && self.colors > self.maximum_colors {
                self.prune_to_cube_depth(ROOT);
                full_depth = false;
            }
        }
    }

    /// One run of `count` equal pixels down the tree to `levels`: each
    /// node on the way gains the run's distance from its centre, the leaf
    /// its colour.
    fn classify_run(&mut self, pixel: &Color, count: usize, levels: usize) {
        let run = count as f64;
        let half = QUANTUM_RANGE / 2.0;
        let mut mid = Color {
            red: half,
            green: half,
            blue: half,
            alpha: half,
        };
        let mut bisect = (QUANTUM_RANGE + 1.0) / 2.0;
        let mut index = MAX_TREE_DEPTH - 1;
        let mut error_alpha = 0.0f64;
        let mut node = ROOT;
        for level in 1..=levels {
            bisect *= 0.5;
            let id = self.node_id(pixel, index);
            mid.red += if id & 1 != 0 { bisect } else { -bisect };
            mid.green += if id & 2 != 0 { bisect } else { -bisect };
            mid.blue += if id & 4 != 0 { bisect } else { -bisect };
            mid.alpha += if id & 8 != 0 { bisect } else { -bisect };
            node = match self.child(node, id) {
                Some(child) => child,
                None => {
                    let child = self.add_node(id, level, node);
                    self.nodes[node].children[id] = child as u32;
                    if level == levels {
                        self.colors += 1;
                    }
                    child
                }
            };
            // Approximate the quantization error this node represents.
            let error_red = QUANTUM_SCALE * (pixel.red - mid.red);
            let error_green = QUANTUM_SCALE * (pixel.green - mid.green);
            let error_blue = QUANTUM_SCALE * (pixel.blue - mid.blue);
            if self.associate_alpha {
                error_alpha = QUANTUM_SCALE * (pixel.alpha - mid.alpha);
            }
            let mut distance = error_red * error_red + error_green * error_green + error_blue * error_blue + error_alpha * error_alpha;
            if distance.is_nan() {
                distance = 0.0;
            }
            self.nodes[node].quantize_error += run * distance.sqrt();
            let node_error = self.nodes[node].quantize_error;
            self.nodes[ROOT].quantize_error += node_error;
            index = index.wrapping_sub(1);
        }

        // Sum the leaf's colour for its mean later.
        let alpha = if self.associate_alpha { clamp_pixel(pixel.alpha) } else { clamp_pixel(QUANTUM_RANGE) };
        let leaf = &mut self.nodes[node];
        leaf.number_unique += count as u64;
        leaf.total_color.red += run * QUANTUM_SCALE * clamp_pixel(pixel.red) as f64;
        leaf.total_color.green += run * QUANTUM_SCALE * clamp_pixel(pixel.green) as f64;
        leaf.total_color.blue += run * QUANTUM_SCALE * clamp_pixel(pixel.blue) as f64;
        leaf.total_color.alpha += run * QUANTUM_SCALE * alpha as f64;
    }

    /// `PruneChild`: folds a subtree into its parent while there are more
    /// nodes than colours wanted.
    fn prune_child(&mut self, node: usize) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.prune_child(child);
            }
        }
        if self.live_nodes > self.maximum_colors {
            let parent = self.nodes[node].parent;
            let unique = self.nodes[node].number_unique;
            let total = self.nodes[node].total_color;
            let id = self.nodes[node].id;
            let parent_node = &mut self.nodes[parent];
            parent_node.number_unique += unique;
            parent_node.total_color.red += total.red;
            parent_node.total_color.green += total.green;
            parent_node.total_color.blue += total.blue;
            parent_node.total_color.alpha += total.alpha;
            parent_node.children[id] = NO_CHILD;
            self.live_nodes -= 1;
        }
    }

    /// `PruneLevel`: folds the bottom level into its parents.
    fn prune_level(&mut self, node: usize) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.prune_level(child);
            }
        }
        if self.nodes[node].level == self.depth {
            self.prune_child(node);
        }
    }

    /// `PruneToCubeDepth`: folds everything below the cube's depth.
    fn prune_to_cube_depth(&mut self, node: usize) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.prune_to_cube_depth(child);
            }
        }
        if self.nodes[node].level > self.depth {
            self.prune_child(node);
        }
    }

    /// `QuantizeErrorFlatten`: the errors in pre-order, at most one per
    /// live node.
    fn flatten_errors(&self, node: usize, offset: usize, errors: &mut [f64]) -> usize {
        if offset >= self.live_nodes {
            return 0;
        }
        errors[offset] = self.nodes[node].quantize_error;
        let mut count = 1;
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                count += self.flatten_errors(child, offset + count, errors);
            }
        }
        count
    }

    /// `Reduce`: prunes the nodes at or below the threshold and finds the
    /// next one.
    fn reduce(&mut self, node: usize) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.reduce(child);
            }
        }
        if self.nodes[node].quantize_error <= self.pruning_threshold {
            self.prune_child(node);
        } else {
            if self.nodes[node].number_unique > 0 {
                self.colors += 1;
            }
            if self.nodes[node].quantize_error < self.next_threshold {
                self.next_threshold = self.nodes[node].quantize_error;
            }
        }
    }

    /// `ReduceImageColors`: a first threshold from the sorted errors that
    /// leaves about 10% more nodes than colours, then pruning rounds until
    /// the colours fit.
    fn reduce_colors(&mut self) {
        self.next_threshold = 0.0;
        if self.colors > self.maximum_colors {
            let mut errors = vec![0.0f64; self.live_nodes];
            self.flatten_errors(ROOT, 0, &mut errors);
            errors.sort_by(f64::total_cmp);
            let kept = 110 * (self.maximum_colors + 1) / 100;
            if self.live_nodes > kept {
                self.next_threshold = errors[self.live_nodes - kept];
            }
        }
        while self.colors > self.maximum_colors {
            self.pruning_threshold = self.next_threshold;
            self.next_threshold = self.nodes[ROOT].quantize_error - 1.0;
            self.colors = 0;
            self.reduce(ROOT);
        }
    }

    /// `DefineImageColormap`: each occupied cube's mean colour, in tree
    /// order. With alpha, a translucent mean is unpremultiplied.
    fn define_colormap(&mut self, node: usize) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.define_colormap(child);
            }
        }
        if self.nodes[node].number_unique == 0 {
            return;
        }
        let total = self.nodes[node].total_color;
        let reciprocal = safe_reciprocal(self.nodes[node].number_unique as f64);
        let mut color = Color {
            alpha: OPAQUE_ALPHA as f64,
            ..Color::default()
        };
        if self.associate_alpha {
            let opacity = reciprocal * QUANTUM_RANGE * total.alpha;
            color.alpha = clamp_to_quantum(opacity) as f64;
        }
        if !self.associate_alpha || color.alpha == QUANTUM_RANGE {
            color.red = clamp_to_quantum(reciprocal * QUANTUM_RANGE * total.red) as f64;
            color.green = clamp_to_quantum(reciprocal * QUANTUM_RANGE * total.green) as f64;
            color.blue = clamp_to_quantum(reciprocal * QUANTUM_RANGE * total.blue) as f64;
        } else {
            let gamma = safe_reciprocal(QUANTUM_SCALE * color.alpha);
            color.red = clamp_to_quantum(reciprocal * gamma * QUANTUM_RANGE * total.red) as f64;
            color.green = clamp_to_quantum(reciprocal * gamma * QUANTUM_RANGE * total.green) as f64;
            color.blue = clamp_to_quantum(reciprocal * gamma * QUANTUM_RANGE * total.blue) as f64;
        }
        self.nodes[node].color_number = self.colormap.len();
        self.colormap.push(color);
    }

    /// `ClosestColor` over a node and its descendants.
    fn closest_color(&self, node: usize, search: &mut Search) {
        for id in 0..self.child_count() {
            if let Some(child) = self.child(node, id) {
                self.closest_color(child, search);
            }
        }
        let info = &self.nodes[node];
        if info.number_unique == 0 {
            return;
        }
        let color = &self.colormap[info.color_number];
        let target = &search.target;
        let (alpha, beta) = if self.associate_alpha {
            (QUANTUM_SCALE * color.alpha, QUANTUM_SCALE * target.alpha)
        } else {
            (1.0, 1.0)
        };
        let mut difference = alpha * color.red - beta * target.red;
        let mut distance = difference * difference;
        if distance > search.distance {
            return;
        }
        difference = alpha * color.green - beta * target.green;
        distance += difference * difference;
        if distance > search.distance {
            return;
        }
        difference = alpha * color.blue - beta * target.blue;
        distance += difference * difference;
        if distance > search.distance {
            return;
        }
        if self.associate_alpha {
            difference = color.alpha - target.alpha;
            distance += difference * difference;
        }
        if distance <= search.distance {
            search.distance = distance;
            search.color_number = info.color_number;
        }
    }

    /// The colormap entry for a colour: the deepest node holding it, then
    /// the closest colour among its parent's descendants. `color_number`
    /// is the cube's last result, which a search that finds nothing
    /// (NaN) keeps.
    fn closest(&self, pixel: &Color, color_number: &mut usize) -> usize {
        let mut node = ROOT;
        for index in (1..MAX_TREE_DEPTH).rev() {
            let id = self.node_id(pixel, index);
            match self.child(node, id) {
                Some(child) => node = child,
                None => break,
            }
        }
        let mut search = Search {
            target: *pixel,
            distance: 4.0 * (QUANTUM_RANGE + 1.0) * (QUANTUM_RANGE + 1.0) + 1.0,
            color_number: *color_number,
        };
        self.closest_color(self.nodes[node].parent, &mut search);
        *color_number = search.color_number;
        search.color_number
    }

    /// The dither colour cache (`cube_info->cache`), one slot per
    /// `CacheShift`-coarse colour.
    fn new_cache(&self) -> Vec<i32> {
        let channels = if self.associate_alpha { 4 } else { 3 };
        let bits = channels * (8 - self.cache_shift as usize);
        vec![-1; 1 << bits]
    }

    /// `CacheOffset`.
    fn cache_offset(&self, pixel: &Color) -> usize {
        let shift = self.cache_shift;
        let width = 8 - shift;
        let red = quantum_to_char(clamp_pixel(pixel.red)) >> shift;
        let green = quantum_to_char(clamp_pixel(pixel.green)) >> shift;
        let blue = quantum_to_char(clamp_pixel(pixel.blue)) >> shift;
        let mut offset = red | (green << width) | (blue << (2 * width));
        if self.associate_alpha {
            let alpha = quantum_to_char(clamp_pixel(pixel.alpha)) >> shift;
            offset |= alpha << (3 * width);
        }
        offset
    }

    /// A dithered colour clamped back to quanta (alpha too when it is
    /// classified).
    fn clamp_dithered(&self, pixel: &mut Color) {
        pixel.red = clamp_pixel(pixel.red) as f64;
        pixel.green = clamp_pixel(pixel.green) as f64;
        pixel.blue = clamp_pixel(pixel.blue) as f64;
        if self.associate_alpha {
            pixel.alpha = clamp_pixel(pixel.alpha) as f64;
        }
    }

    /// The error a dithered colour leaves once its colormap colour stands
    /// in for it.
    fn dither_error(&self, pixel: &Color, color: &Color) -> Color {
        let associated = self.associate_alpha_color(color);
        let alpha = if self.associate_alpha { pixel.alpha - associated.alpha } else { 0.0 };
        Color {
            red: pixel.red - associated.red,
            green: pixel.green - associated.green,
            blue: pixel.blue - associated.blue,
            alpha,
        }
    }

    /// `FloydSteinbergDither` on a serpentine path.
    fn floyd_steinberg(&self, data: &mut [f32]) {
        let width = self.width;
        let mut cache = self.new_cache();
        let mut errors = vec![Color::default(); 2 * width];
        for y in 0..self.height {
            // Each row searches with a copy of the cube.
            let mut color_number = 0;
            let current_row = (y & 1) * width;
            let previous_row = ((y + 1) & 1) * width;
            let reverse = y & 1 != 0;
            for x in 0..width {
                let u = if reverse { width - 1 - x } else { x };
                // `u - v` and `u + v`, where `v` is the direction of travel.
                let behind = if reverse { u + 1 } else { u.wrapping_sub(1) };
                let ahead = if reverse { u.wrapping_sub(1) } else { u + 1 };
                let offset = y * width + u;
                let mut pixel = self.associate_alpha_pixel(self.layout.pixel(data, offset));
                if x > 0 {
                    let error = errors[current_row + behind];
                    pixel.red += 7.0 * DIFFUSION * error.red / 16.0;
                    pixel.green += 7.0 * DIFFUSION * error.green / 16.0;
                    pixel.blue += 7.0 * DIFFUSION * error.blue / 16.0;
                    if self.associate_alpha {
                        pixel.alpha += 7.0 * DIFFUSION * error.alpha / 16.0;
                    }
                }
                if y > 0 {
                    if x < width - 1 {
                        let error = errors[previous_row + ahead];
                        pixel.red += DIFFUSION * error.red / 16.0;
                        pixel.green += DIFFUSION * error.green / 16.0;
                        pixel.blue += DIFFUSION * error.blue / 16.0;
                        if self.associate_alpha {
                            pixel.alpha += DIFFUSION * error.alpha / 16.0;
                        }
                    }
                    let error = errors[previous_row + u];
                    pixel.red += 5.0 * DIFFUSION * error.red / 16.0;
                    pixel.green += 5.0 * DIFFUSION * error.green / 16.0;
                    pixel.blue += 5.0 * DIFFUSION * error.blue / 16.0;
                    if self.associate_alpha {
                        pixel.alpha += 5.0 * DIFFUSION * error.alpha / 16.0;
                    }
                    if x > 0 {
                        let error = errors[previous_row + behind];
                        pixel.red += 3.0 * DIFFUSION * error.red / 16.0;
                        pixel.green += 3.0 * DIFFUSION * error.green / 16.0;
                        pixel.blue += 3.0 * DIFFUSION * error.blue / 16.0;
                        if self.associate_alpha {
                            pixel.alpha += 3.0 * DIFFUSION * error.alpha / 16.0;
                        }
                    }
                }
                self.clamp_dithered(&mut pixel);
                let slot = self.cache_offset(&pixel);
                if cache[slot] < 0 {
                    cache[slot] = self.closest(&pixel, &mut color_number) as i32;
                }
                let color = self.colormap[cache[slot] as usize];
                self.layout.set_pixel(data, offset, &color, self.associate_alpha);
                errors[current_row + u] = self.dither_error(&pixel, &color);
            }
        }
    }

    /// `DitherImage` with `RiemersmaDitherMethod`: the error carried along
    /// a Hilbert curve that covers the image, from the top left.
    fn riemersma(&self, data: &mut [f32]) {
        let extent = self.width.max(self.height);
        let mut level = (usize::BITS - 1 - extent.leading_zeros()) as usize;
        if (1usize << level) < extent {
            level += 1;
        }
        let mut weights = [0.0f64; ERROR_QUEUE_LENGTH];
        let mut weight = 1.0f64;
        for entry in weights.iter_mut() {
            *entry = safe_reciprocal(weight);
            weight *= ERROR_WEIGHT_GROWTH;
        }
        let mut walker = Riemersma {
            quantizer: self,
            data,
            cache: self.new_cache(),
            weights,
            errors: [Color::default(); ERROR_QUEUE_LENGTH],
            x: 0,
            y: 0,
            color_number: 0,
        };
        if level > 0 {
            walker.curve(level, Direction::North);
        }
        walker.dither(Direction::Forget);
    }
}

/// `RiemersmaDither`'s walker: position, error queue and weights.
struct Riemersma<'a> {
    quantizer: &'a Quantizer,
    data: &'a mut [f32],
    cache: Vec<i32>,
    weights: [f64; ERROR_QUEUE_LENGTH],
    errors: [Color; ERROR_QUEUE_LENGTH],
    x: isize,
    y: isize,
    color_number: usize,
}

impl Riemersma<'_> {
    /// `RiemersmaDither`: dithers the current pixel, then moves.
    fn dither(&mut self, direction: Direction) {
        let quantizer = self.quantizer;
        let inside = self.x >= 0 && self.x < quantizer.width as isize && self.y >= 0 && self.y < quantizer.height as isize;
        if inside {
            let offset = self.y as usize * quantizer.width + self.x as usize;
            let mut pixel = quantizer.associate_alpha_pixel(quantizer.layout.pixel(self.data, offset));
            for (weight, error) in self.weights.iter().zip(self.errors.iter()) {
                pixel.red += ERROR_RELATIVE_WEIGHT * DIFFUSION * weight * error.red;
                pixel.green += ERROR_RELATIVE_WEIGHT * DIFFUSION * weight * error.green;
                pixel.blue += ERROR_RELATIVE_WEIGHT * DIFFUSION * weight * error.blue;
                if quantizer.associate_alpha {
                    pixel.alpha += ERROR_RELATIVE_WEIGHT * DIFFUSION * weight * error.alpha;
                }
            }
            quantizer.clamp_dithered(&mut pixel);
            let slot = quantizer.cache_offset(&pixel);
            if self.cache[slot] < 0 {
                self.cache[slot] = quantizer.closest(&pixel, &mut self.color_number) as i32;
            }
            let color = quantizer.colormap[self.cache[slot] as usize];
            quantizer.layout.set_pixel(self.data, offset, &color, quantizer.associate_alpha);
            // The queue moves up one; the new error goes last.
            let error = quantizer.dither_error(&pixel, &color);
            self.errors.copy_within(1.., 0);
            let last = &mut self.errors[ERROR_QUEUE_LENGTH - 1];
            last.red = error.red;
            last.green = error.green;
            last.blue = error.blue;
            if quantizer.associate_alpha {
                last.alpha = error.alpha;
            }
        }
        match direction {
            Direction::West => self.x -= 1,
            Direction::East => self.x += 1,
            Direction::North => self.y -= 1,
            Direction::South => self.y += 1,
            Direction::Forget => {}
        }
    }

    /// `Riemersma`: the Hilbert curve of `level` in `direction`.
    fn curve(&mut self, level: usize, direction: Direction) {
        use Direction::*;
        if level == 1 {
            let steps = match direction {
                West => [East, South, West],
                East => [West, North, East],
                North => [South, East, North],
                South => [North, West, South],
                Forget => return,
            };
            for step in steps {
                self.dither(step);
            }
            return;
        }
        let (curves, steps) = match direction {
            West => ([North, West, West, South], [East, South, West]),
            East => ([South, East, East, North], [West, North, East]),
            North => ([West, North, North, East], [South, East, North]),
            South => ([East, South, South, West], [North, West, South]),
            Forget => return,
        };
        self.curve(level - 1, curves[0]);
        self.dither(steps[0]);
        self.curve(level - 1, curves[1]);
        self.dither(steps[1]);
        self.curve(level - 1, curves[2]);
        self.dither(steps[2]);
        self.curve(level - 1, curves[3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_depths_follow_quantize_image() {
        assert_eq!(automatic_tree_depth(256, false, false), 5);
        assert_eq!(automatic_tree_depth(256, true, false), 5);
        assert_eq!(automatic_tree_depth(65536, true, false), 8);
        assert_eq!(automatic_tree_depth(2, false, false), 2);
        assert_eq!(automatic_tree_depth(256, true, true), MAX_TREE_DEPTH);
    }

    #[test]
    fn few_colours_survive_unchanged() {
        let colors = [[1000.0f32, 2000.0, 3000.0], [65535.0, 0.0, 0.0], [12.5, 40000.25, 65535.0]];
        let mut data = Vec::new();
        for index in 0..64 {
            data.extend_from_slice(&colors[(index * 7 + index / 8) % 3]);
        }
        let original = data.clone();
        for dither in [Dither::Riemersma, Dither::FloydSteinberg] {
            let options = QuantizeOptions {
                maximum_colors: 256,
                tree_depth: 5,
                dither,
                cache_shift: 2,
            };
            let mut quantized = original.clone();
            let colormap = quantize(&mut quantized, 8, 8, Layout::Rgb, &options);
            assert_eq!(colormap.len(), 3);
            assert_eq!(quantized, data);
        }
    }

    #[test]
    fn grey_images_quantize_like_their_rgb_copies() {
        let gray: Vec<f32> = (0..32 * 24).map(|index| ((index * 7919) % 65536) as f32 * 0.999).collect();
        let rgb: Vec<f32> = gray.iter().flat_map(|&value| [value, value, value]).collect();
        for dither in [Dither::Riemersma, Dither::FloydSteinberg] {
            let options = QuantizeOptions {
                maximum_colors: 16,
                tree_depth: MAX_TREE_DEPTH,
                dither,
                cache_shift: 2,
            };
            let mut gray_result = gray.clone();
            let mut rgb_result = rgb.clone();
            quantize(&mut gray_result, 32, 24, Layout::Gray, &options);
            quantize(&mut rgb_result, 32, 24, Layout::Rgb, &options);
            let from_rgb: Vec<f32> = rgb_result.chunks(3).map(|pixel| pixel[0]).collect();
            assert_eq!(gray_result, from_rgb);
        }
    }

    #[test]
    fn colours_reduce_to_the_maximum() {
        let data: Vec<f32> = (0..64 * 64 * 4).map(|index| ((index * 2654435761usize) % 65536) as f32).collect();
        for (layout, depth) in [(Layout::Rgb, 5), (Layout::Rgba, 5)] {
            let channels = layout.channels();
            let mut image: Vec<f32> = data[..64 * 64 * channels].to_vec();
            let options = QuantizeOptions {
                maximum_colors: 256,
                tree_depth: depth,
                dither: Dither::Riemersma,
                cache_shift: 2,
            };
            let colormap = quantize(&mut image, 64, 64, layout, &options);
            assert!(colormap.len() <= 256);
            for pixel in image.chunks(channels) {
                let found = colormap.iter().any(|color| {
                    let rgb = pixel[0] == color.red as f32 && pixel[1] == color.green as f32 && pixel[2] == color.blue as f32;
                    rgb && (channels == 3 || pixel[3] == color.alpha as f32)
                });
                assert!(found, "pixel {pixel:?} is not a colormap colour");
            }
        }
    }
}
