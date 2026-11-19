//! 難易度（基本設計 3.12）。全4段階で、進化ゲージの溜まりやすさと有効なGoalの数に影響する。
//! 【未決】Easy 以外の名称。名前が決まるまでは段階の番号で表す。

/// 難易度の段階。`EASY` が最も易しく、数字が大きいほど難しい。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Difficulty(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("difficulty must be 1..={max} (got {0})", max = Difficulty::LEVELS)]
pub struct UnknownDifficulty(pub u8);

impl Difficulty {
    /// 段階の数（確定）
    pub const LEVELS: u8 = 4;
    pub const EASY: Self = Self(1);
    pub const HARDEST: Self = Self(Self::LEVELS);

    /// 1（Easy）〜4。範囲外は拒否する。
    pub fn new(level: u8) -> Result<Self, UnknownDifficulty> {
        if (1..=Self::LEVELS).contains(&level) {
            Ok(Self(level))
        } else {
            Err(UnknownDifficulty(level))
        }
    }

    pub fn level(self) -> u8 {
        self.0
    }

    /// 易しい順のすべての段階。
    pub fn all() -> impl Iterator<Item = Self> {
        (1..=Self::LEVELS).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::easy(1)]
    #[case::hardest(4)]
    fn levels_one_to_four_are_valid(#[case] level: u8) {
        assert_eq!(Difficulty::new(level).map(Difficulty::level), Ok(level));
    }

    #[rstest]
    #[case::zero(0)]
    #[case::five(5)]
    #[case::max(u8::MAX)]
    fn out_of_range_is_rejected(#[case] level: u8) {
        assert_eq!(Difficulty::new(level), Err(UnknownDifficulty(level)));
    }

    #[rstest]
    fn difficulties_are_ordered_from_easy() {
        let all: Vec<_> = Difficulty::all().collect();
        assert_eq!(all.len(), 4);
        assert_eq!(all.first(), Some(&Difficulty::EASY));
        assert_eq!(all.last(), Some(&Difficulty::HARDEST));
        assert!(all.windows(2).all(|w| w[0] < w[1]));
    }
}
