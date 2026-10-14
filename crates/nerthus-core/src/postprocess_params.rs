//! The generic "Post Process" parameters every maskable node carries.

use crate::param::{Param, ParamList};
use crate::slider;

/// The standard seed parameter: random default in [0, 128000].
pub fn seed_param() -> Param {
    seed_param_named("Seed")
}

pub fn seed_param_named(name: &str) -> Param {
    let default = random_seed();
    Param::int(name, default, 0, 128_000).with_brush(slider::SEED)
}

/// A process-unique pseudo random seed (as `new Random(Environment.TickCount)` gives).
pub fn random_seed() -> i32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use web_time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos() as u64).unwrap_or(0);
    let mut state = nanos ^ COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    state ^= state >> 33;
    state = state.wrapping_mul(0xff51_afd7_ed55_8ccd);
    state ^= state >> 33;
    (state % 128_000) as i32
}

pub fn post_process_params() -> ParamList {
    let params = vec![
        Param::title("Post Process"),
        Param::boolean("Autolevel", false),
        Param::boolean("Equalize", false),
        Param::boolean("Log", false),
        Param::boolean("Invert", false),
        Param::boolean("Influence", true),
        Param::double("Influence", 1.0),
        Param::boolean("Blur", false),
        Param::double_range("Power", 0.0, 0.05, 20.0).with_brush(slider::PINK1),
        Param::boolean("Clamp", false),
        Param::double("Min", 0.0).with_brush(slider::GREEN1),
        Param::double("Max", 1.0).with_brush(slider::GREEN2),
        Param::boolean("Clip", false),
        Param::double("ClipMin", 0.0).with_brush(slider::BLUE2),
        Param::double("ClipMax", 1.0).with_brush(slider::BLUE3),
        Param::boolean("Displace", false),
        Param::double_range("Scale", 0.6, 0.0001, 1.0).with_brush(slider::YELLOW_DARK1),
        Param::double_range("Displace", 0.1, 0.01, 1.0).with_brush(slider::YELLOW2),
        seed_param(),
        Param::boolean("Raise", false),
        Param::double_range("Multiplier", 1.0, 0.0, 10.0).with_brush(slider::TEAL1),
        Param::boolean("Shaper", false),
        Param::double_range("Shaper", 0.0, -1.0, 1.0).with_brush(slider::RED1),
        Param::boolean("MX", false),
        Param::boolean("MN", false),
        Param::boolean("Diff", false),
        Param::boolean("Drop", false),
        Param::boolean("Autolevel Mask", false),
    ];
    ParamList::from_vec(params)
}
