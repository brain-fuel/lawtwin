//! SplitMix64, the one generator every core and peer draws from, so the
//! same seed gives the same choices in every language
//! (docs/reference/kafka-core.md#determinism).

/// A seeded SplitMix64 generator.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// The generator for a seed.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next draw.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// A draw below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    /// A draw in `lo..=hi`.
    pub fn between(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi as i128 - lo as i128 + 1) as u128;
        (lo as i128 + (self.next_u64() as u128 % span) as i128) as i64
    }

    /// True with this chance, in percent.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// One of the items.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { items.get(self.below(items.len() as u64) as usize) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first draws for seed 0 are SplitMix64's published ones.
    #[test]
    fn matches_splitmix64() {
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xe220a8397b1dcdaf);
        assert_eq!(r.next_u64(), 0x6e789e6aa1b965f4);
    }
}
