//! Deterministic 64-bit PRNG for the experiment harness.
//!
//! The gauntlet needs reproducible "randomness" (seeded shuffles, fixed
//! proposal streams) with no new dependencies. XorShift64* is enough:
//! this is experiment scheduling, not cryptography. Every stream is
//! seeded explicitly per (task, arm, seed) so arms share the identical
//! seed stream by construction.

/// XorShift64* generator. Not for cryptographic use.
#[derive(Debug, Clone)]
pub struct XorShift {
    state: u64,
}

impl XorShift {
    /// Build from a seed. A zero seed is replaced with a fixed non-zero
    /// constant so the generator never locks at zero.
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        XorShift { state }
    }

    /// Next `u64`. XorShift64* (Marsaglia; `* 0x2545F4914F6CDD1D`).
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform `f64` in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        const DIVISOR: f64 = u64::MAX as f64 + 1.0;
        (self.next_u64() as f64) / DIVISOR
    }

    /// Uniform index into `[0, bound)`. Panics on `bound == 0` — the
    /// caller must guarantee a non-empty range (internal invariant).
    pub fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "XorShift::below with empty range");
        (self.next_f64() * bound as f64) as usize
    }

    /// Fisher-Yates shuffle, in place.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }

    /// Sample `k` distinct indices from `[0, n)` without replacement.
    /// Returns fewer than `k` when `k > n` (never panics).
    pub fn sample_without_replacement(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..n).collect();
        self.shuffle(&mut indices);
        indices.truncate(k.min(n));
        indices
    }
}

#[cfg(test)]
mod tests {
    use super::XorShift;

    /// Validation: identical seeds produce identical streams.
    #[test]
    fn same_seed_same_stream() {
        let (mut a, mut b) = (XorShift::new(42), XorShift::new(42));
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    /// Validation: different seeds diverge quickly.
    #[test]
    fn different_seeds_diverge() {
        let (mut a, mut b) = (XorShift::new(1), XorShift::new(2));
        let (x, y): (Vec<_>, Vec<_>) = (
            (0..16).map(|_| a.next_u64()).collect(),
            (0..16).map(|_| b.next_u64()).collect(),
        );
        assert_ne!(x, y);
    }

    /// Validation: `below` stays in range and `shuffle` is a permutation.
    #[test]
    fn below_bounded_and_shuffle_permutes() {
        let mut rng = XorShift::new(7);
        for _ in 0..200 {
            assert!(rng.below(10) < 10);
        }
        let mut v: Vec<u32> = (0..20).collect();
        let before = v.clone();
        rng.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, before, "shuffle must be a permutation");
    }

    /// Adversarial: a zero seed must not produce a stuck (all-zero) stream.
    #[test]
    fn zero_seed_not_stuck() {
        let mut rng = XorShift::new(0);
        let vals: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
        assert!(
            vals.iter().any(|&v| v != 0),
            "zero seed stuck the generator"
        );
    }
}
