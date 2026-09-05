//! A small deterministic generator: splitmix64. No dependency, std only,
//! and the same sequence for the same seed on every platform, so a printed
//! seed replays a case exactly.

/// splitmix64 (Steele, Lea, Flood 2014).
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator seeded with `seed`.
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// A number in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            lo
        } else {
            lo + self.below(hi - lo + 1)
        }
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// A number in `[0, 1)`.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// One of the items, uniformly.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    /// A random permutation of `0..n`.
    pub fn permutation(&mut self, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = self.below(i as u64 + 1) as usize;
            v.swap(i, j);
        }
        v
    }

    /// A random subset of `0..n` with at most `max` members, in order.
    pub fn subset(&mut self, n: usize, max: usize) -> Vec<usize> {
        if n == 0 || max == 0 {
            return Vec::new();
        }
        let count = self.below(max.min(n) as u64 + 1) as usize;
        let mut perm = self.permutation(n);
        perm.truncate(count);
        perm.sort();
        perm
    }
}

/// The per-case seed: the suite seed mixed with the case index, so one
/// printed number pins one case.
pub fn case_seed(suite_seed: u64, index: u64) -> u64 {
    let mut g = SplitMix64::new(suite_seed ^ index.wrapping_mul(0xA24B_AED4_963E_E407));
    g.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<u64> = (0..8).map(|_| SplitMix64::new(7).next_u64()).collect();
        let mut g = SplitMix64::new(7);
        let mut h = SplitMix64::new(7);
        for _ in 0..8 {
            assert_eq!(g.next_u64(), h.next_u64());
        }
        assert!(a.iter().all(|&x| x == a[0]), "fresh generators agree");
        assert_eq!(SplitMix64::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);
    }

    #[test]
    fn helpers_stay_in_range() {
        let mut g = SplitMix64::new(42);
        for _ in 0..1000 {
            assert!(g.below(5) < 5);
            let r = g.range(3, 6);
            assert!((3..=6).contains(&r));
            let f = g.f64();
            assert!((0.0..1.0).contains(&f));
            let s = g.subset(6, 3);
            assert!(s.len() <= 3 && s.windows(2).all(|w| w[0] < w[1]));
        }
        assert_eq!(g.permutation(0), Vec::<usize>::new());
    }
}
