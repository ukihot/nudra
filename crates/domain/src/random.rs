//! 決定的な乱数（基本設計 3.13）。New! の候補選びに使う。
//!
//! セッション開始時のシードから生成し、同じシードなら同じ列になる（テストで再現でき、
//! 外部crateも不要）。 アルゴリズムは SplitMix64（シード0でも偏らない）。暗号用途には使わない。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRng {
    state: u64,
}

impl SessionRng {
    pub fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..len` から1つ選ぶ。`len` が0なら `None`。
    pub fn pick(&mut self, len: usize) -> Option<usize> {
        // ponytail: 剰余による偏りは len / 2^64 程度で、候補数十件のゲーム用途では無視できる
        let len64 = u64::try_from(len).ok().filter(|&n| n > 0)?;
        usize::try_from(self.next_u64() % len64).ok()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn same_seed_gives_the_same_sequence() {
        let (mut a, mut b) = (SessionRng::seeded(42), SessionRng::seeded(42));
        let a: Vec<_> = (0..5).map(|_| a.next_u64()).collect();
        let b: Vec<_> = (0..5).map(|_| b.next_u64()).collect();
        assert_eq!(a, b);
    }

    #[rstest]
    fn different_seeds_give_different_sequences() {
        assert_ne!(SessionRng::seeded(1).next_u64(), SessionRng::seeded(2).next_u64());
    }

    #[rstest]
    fn seed_zero_still_varies() {
        let mut rng = SessionRng::seeded(0);
        let (a, b) = (rng.next_u64(), rng.next_u64());
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }

    #[rstest]
    fn picking_from_nothing_is_none() {
        assert_eq!(SessionRng::seeded(7).pick(0), None);
    }

    #[rstest]
    fn picks_spread_over_all_candidates() {
        let mut rng = SessionRng::seeded(3);
        let mut seen = [false; 16];
        for _ in 0..500 {
            seen[rng.pick(16).expect("non-empty")] = true;
        }
        assert!(seen.iter().all(|&s| s), "every index is picked at least once");
    }

    proptest! {
        #[test]
        fn pick_is_always_in_range(seed in any::<u64>(), len in 1usize..100) {
            let mut rng = SessionRng::seeded(seed);
            prop_assert!(rng.pick(len).expect("non-empty") < len);
        }
    }
}
