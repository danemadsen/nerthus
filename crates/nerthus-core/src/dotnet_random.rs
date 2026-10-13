//! Bit-exact port of the .NET Framework `System.Random` (Knuth subtractive
//! generator). Many random layouts are seeded with it, so matching it
//! reproduces the same terrain for the same seed.

const MBIG: i32 = i32::MAX;
const MSEED: i32 = 161_803_398;

#[derive(Clone, Debug)]
pub struct DotNetRandom {
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

impl DotNetRandom {
    pub fn new(seed: i32) -> Self {
        let mut seed_array = [0i32; 56];
        let subtraction = if seed == i32::MIN { i32::MAX } else { seed.abs() };
        let mut mj = MSEED - subtraction;
        seed_array[55] = mj;
        let mut mk = 1i32;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            seed_array[ii] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = seed_array[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                let other = seed_array[1 + (i + 30) % 55];
                seed_array[i] = seed_array[i].wrapping_sub(other);
                if seed_array[i] < 0 {
                    seed_array[i] = seed_array[i].wrapping_add(MBIG);
                }
            }
        }
        Self {
            seed_array,
            inext: 0,
            inextp: 21,
        }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut next = self.inext + 1;
        if next >= 56 {
            next = 1;
        }
        let mut nextp = self.inextp + 1;
        if nextp >= 56 {
            nextp = 1;
        }
        let mut value = self.seed_array[next].wrapping_sub(self.seed_array[nextp]);
        if value == MBIG {
            value -= 1;
        }
        if value < 0 {
            value = value.wrapping_add(MBIG);
        }
        self.seed_array[next] = value;
        self.inext = next;
        self.inextp = nextp;
        value
    }

    fn sample(&mut self) -> f64 {
        self.internal_sample() as f64 * (1.0 / MBIG as f64)
    }

    fn sample_for_large_range(&mut self) -> f64 {
        let mut result = self.internal_sample();
        let negative = self.internal_sample() % 2 == 0;
        if negative {
            result = -result;
        }
        let mut value = result as f64;
        value += (i32::MAX - 1) as f64;
        value /= 2.0 * i32::MAX as u32 as f64 - 1.0;
        value
    }

    /// `Random.Next()` (named after .NET's, not `Iterator::next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        self.internal_sample()
    }

    /// `Random.Next(maxValue)`
    pub fn next_max(&mut self, max_value: i32) -> i32 {
        (self.sample() * max_value as f64) as i32
    }

    /// `Random.Next(minValue, maxValue)`
    pub fn next_range(&mut self, min_value: i32, max_value: i32) -> i32 {
        let range = max_value as i64 - min_value as i64;
        if range <= i32::MAX as i64 {
            return (self.sample() * range as f64) as i32 + min_value;
        }
        let scaled = (self.sample_for_large_range() * range as f64) as i64;
        (scaled + min_value as i64) as i32
    }

    /// `Random.NextDouble()`
    pub fn next_double(&mut self) -> f64 {
        self.sample()
    }

    /// `Random.NextBytes(buffer)`
    pub fn next_bytes(&mut self, buffer: &mut [u8]) {
        for byte in buffer.iter_mut() {
            *byte = (self.internal_sample() % 256) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_dotnet_reference_sequence() {
        // new Random(0).Next() sequence from .NET Framework.
        let mut random = DotNetRandom::new(0);
        assert_eq!(random.next(), 1_559_595_546);
        assert_eq!(random.next(), 1_755_192_844);
        assert_eq!(random.next(), 1_649_316_166);
    }

    #[test]
    fn matches_dotnet_reference_doubles() {
        // new Random(42).NextDouble() from .NET Framework.
        let mut random = DotNetRandom::new(42);
        let first = random.next_double();
        assert!((first - 0.668_106_465_911_542_3).abs() < 1e-15);
    }

    #[test]
    fn matches_dotnet_reference_ranges() {
        // new Random(12345): Next(0, 100), Next(10), Next(-50, 50) from .NET.
        let mut random = DotNetRandom::new(12345);
        assert_eq!(random.next_range(0, 100), 6);
        assert_eq!(random.next_max(10), 0);
        assert_eq!(random.next_range(-50, 50), 27);
    }
}
