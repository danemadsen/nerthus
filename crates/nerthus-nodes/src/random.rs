//! The random number generators.

/// `XorShift64Star`: xorshift64* with its helpers for ints and floats.
#[derive(Clone, Copy, Debug)]
pub struct XorShift64Star {
    state: u64,
}

const MULTIPLIER: u64 = 2_685_821_657_736_338_717;

impl XorShift64Star {
    /// A generator seeded like `new XorShift64Star(seed)`; seed 0 is
    /// replaced by a fixed non-zero state.
    pub fn new(seed: i32) -> Self {
        let state = if seed == 0 { 11_688_269_583_697_384_694 } else { seed as u32 as u64 };
        Self { state }
    }

    fn advance(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 7;
        self.state = state;
        state.wrapping_mul(MULTIPLIER)
    }

    /// A 31-bit integer.
    pub fn next_int(&mut self) -> i32 {
        (self.advance() >> 33) as i32
    }

    /// An integer in `0..limit`.
    pub fn next_below(&mut self, limit: i32) -> i32 {
        let value = (self.advance().wrapping_sub(1) & 0x7FFF_FFFF) as i32;
        value % limit
    }

    /// An integer in `low..high`.
    pub fn next_range(&mut self, low: i32, high: i32) -> i32 {
        let value = (self.advance().wrapping_sub(1) & 0x7FFF_FFFF) as i32;
        low + value % (high - low)
    }

    /// A float in [0, 1).
    pub fn next_float(&mut self) -> f32 {
        let bits = self.advance() >> 24;
        bits as f32 * 9.094947e-13
    }

    /// `next_float() * scale`.
    pub fn next_scaled(&mut self, scale: f32) -> f32 {
        self.next_float() * scale
    }

    /// A float in [`low`, `high`).
    pub fn next_float_range(&mut self, low: f32, high: f32) -> f32 {
        low + self.next_float() * (high - low)
    }
}

/// `std::mt19937`.
pub struct Mt19937 {
    state: [u32; MT_STATE_SIZE],
    index: usize,
}

const MT_STATE_SIZE: usize = 624;
const MT_SHIFT_SIZE: usize = 397;

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; MT_STATE_SIZE];
        state[0] = seed;
        for index in 1..MT_STATE_SIZE {
            let previous = state[index - 1];
            let mixed = 1812433253u32.wrapping_mul(previous ^ (previous >> 30));
            state[index] = mixed.wrapping_add(index as u32);
        }
        Self { state, index: MT_STATE_SIZE }
    }

    fn twist(&mut self) {
        for index in 0..MT_STATE_SIZE {
            let next = self.state[(index + 1) % MT_STATE_SIZE];
            let combined = (self.state[index] & 0x8000_0000) | (next & 0x7FFF_FFFF);
            let mut value = self.state[(index + MT_SHIFT_SIZE) % MT_STATE_SIZE] ^ (combined >> 1);
            if combined & 1 != 0 {
                value ^= 0x9908_B0DF;
            }
            self.state[index] = value;
        }
        self.index = 0;
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= MT_STATE_SIZE {
            self.twist();
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9D2C_5680;
        value ^= (value << 15) & 0xEFC6_0000;
        value ^ (value >> 18)
    }

    /// MSVC's `_Rng_from_urng`: a uniform index below `bound` by rejection.
    fn below(&mut self, bound: u64) -> u64 {
        loop {
            let mut value = 0u64;
            let mut mask = 0u64;
            while mask < bound - 1 {
                value = (value << 32) | self.next_u32() as u64;
                mask = (mask << 32) | 0xFFFF_FFFF;
            }
            if value / bound < mask / bound || mask % bound == bound - 1 {
                return value % bound;
            }
        }
    }
}

/// `std::shuffle` as MSVC's standard library implements it.
pub fn msvc_shuffle<T>(items: &mut [T], generator: &mut Mt19937) {
    for target in 1..items.len() {
        let chosen = generator.below(target as u64 + 1) as usize;
        if chosen != target {
            items.swap(target, chosen);
        }
    }
}
