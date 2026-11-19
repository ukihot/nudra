//! ゲーム内の論理時刻（基本設計 6.5）。
//!
//! ゲーム世界に時計は1つだけ。固定更新1回を1Tickとし、セッション開始を `Tick(0)` とする。
//! 一時停止中は進まず、端末の時計やフレームレートに依存しない。
//! Entityは「残り時間」を毎tick減算せず、「期限のTick」を持って現在のTickと比較する。

use std::ops::{Add, Sub};

/// ある時点のTick。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

/// Tickの間隔（長さ）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticks(pub u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    /// 次のTick。上限で止まる（u64を使い切ることは現実には無い）。
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl Ticks {
    /// ミリ秒をTickへ変換する。端数は切り上げ（クールダウンが短くならないようにする）。
    pub fn from_millis_ceil(millis: u64, ticks_per_second: u32) -> Self {
        let scaled = u128::from(millis) * u128::from(ticks_per_second);
        Self(u64::try_from(scaled.div_ceil(1000)).unwrap_or(u64::MAX))
    }
}

/// 期限の計算。上限で止まる。
impl Add<Ticks> for Tick {
    type Output = Tick;

    fn add(self, span: Ticks) -> Tick {
        Tick(self.0.saturating_add(span.0))
    }
}

/// 2つの時点の間隔。過去を引いた場合は0。
impl Sub for Tick {
    type Output = Ticks;

    fn sub(self, earlier: Tick) -> Ticks {
        Ticks(self.0.saturating_sub(earlier.0))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::exact(500, 60, 30)]
    #[case::rounds_up(10, 60, 1)]
    #[case::zero(0, 60, 0)]
    #[case::one_second(1000, 60, 60)]
    #[case::just_over(1001, 60, 61)]
    fn millis_convert_to_ticks_rounding_up(#[case] ms: u64, #[case] hz: u32, #[case] ticks: u64) {
        assert_eq!(Ticks::from_millis_ceil(ms, hz), Ticks(ticks));
    }

    #[rstest]
    fn deadline_and_interval() {
        assert_eq!(Tick(100) + Ticks(30), Tick(130));
        assert_eq!(Tick(130) - Tick(100), Ticks(30));
        assert_eq!(Tick(100) - Tick(130), Ticks(0), "earlier minus later is zero");
        assert_eq!(Tick(7).next(), Tick(8));
    }

    #[rstest]
    fn arithmetic_saturates_at_the_end_of_time() {
        assert_eq!(Tick(u64::MAX).next(), Tick(u64::MAX));
        assert_eq!(Tick(u64::MAX - 1) + Ticks(5), Tick(u64::MAX));
        assert_eq!(Ticks::from_millis_ceil(u64::MAX, 1000), Ticks(u64::MAX));
    }

    proptest! {
        /// 変換したTick数は、元のミリ秒を下回らない（クールダウンが短くならない）。
        #[test]
        fn converted_ticks_never_shorter(ms in 0u64..10_000_000, hz in 1u32..1000) {
            let ticks = Ticks::from_millis_ceil(ms, hz).0;
            prop_assert!(u128::from(ticks) * 1000 >= u128::from(ms) * u128::from(hz));
            prop_assert!(ticks == 0 || u128::from(ticks - 1) * 1000 < u128::from(ms) * u128::from(hz));
        }
    }
}
