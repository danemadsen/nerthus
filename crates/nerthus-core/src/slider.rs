//! Slider accent colours used by the parameter editor (`SliderColors`).

pub const YELLOW_DARK1: &str = "#dbba6a";
pub const YELLOW_DARK2: &str = "#ad9559";
pub const YELLOW_DARK3: &str = "#977723";
pub const YELLOW1: &str = "#f6d973";
pub const YELLOW2: &str = "#cdc172";
pub const YELLOW3: &str = "#958e60";
pub const GREEN1: &str = "#86b065";
pub const GREEN2: &str = "#6e8a59";
pub const GREEN3: &str = "#52a779";
pub const GREEN4: &str = "#4d8f6c";
pub const GREEN5: &str = "#4d7e63";
pub const TEAL1: &str = "#00bda4";
pub const TEAL2: &str = "#20ac9a";
pub const TEAL3: &str = "#218a7c";
pub const TEAL4: &str = "#2a7172";
pub const BLUE1: &str = "#00a9fa";
pub const BLUE2: &str = "#1683b1";
pub const BLUE3: &str = "#376c99";
pub const PURPLE1: &str = "#635a96";
pub const PURPLE2: &str = "#575276";
pub const LAVENDER1: &str = "#cdaef1";
pub const LAVENDER2: &str = "#9482a9";
pub const PINK1: &str = "#c96395";
pub const PINK2: &str = "#bc5e8d";
pub const PINK3: &str = "#9b576d";
pub const PINK4: &str = "#df6475";
pub const RED1: &str = "#E24141";
pub const RED2: &str = "#C43434";
pub const ORANGE1: &str = "#ECAA4B";
pub const ORANGE2: &str = "#E8A333";

/// Seed sliders use this colour.
pub const SEED: &str = "#9482a9";

/// Parses `#rrggbb` into components.
pub fn parse_hex(color: &str) -> [u8; 3] {
    let digits = color.trim_start_matches('#');
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(digits.get(range).unwrap_or("80"), 16).unwrap_or(128);
    [channel(0..2), channel(2..4), channel(4..6)]
}
