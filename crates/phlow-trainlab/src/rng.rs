//! A tiny deterministic PRNG (SplitMix64).
//!
//! Split generation must be reproducible from a seed alone, on any
//! machine, with no external crates. SplitMix64 is adequate for
//! shuffling and name generation; it is **not** a cryptographic RNG
//! and is never used for secrets.

/// SplitMix64 generator. State is one `u64`; every draw advances it.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Create a generator from `seed`. Any seed is valid.
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    /// Next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..bound`. `bound` must be positive; a zero
    /// bound is a caller bug and asserts.
    pub fn next_below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "next_below requires a positive bound");
        self.next_u64() % bound
    }

    /// In-place Fisher–Yates shuffle driven by this generator.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        if items.len() < 2 {
            return;
        }
        for i in (1..items.len()).rev() {
            let j =
                usize::try_from(self.next_below((i + 1) as u64)).expect("shuffle index fits usize");
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = SplitMix64::new(7);
        let mut items: Vec<u32> = (0..32).collect();
        rng.shuffle(&mut items);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..32).collect::<Vec<u32>>());
    }
}
