//! 正の有限値。質量・半径・強化倍率など、0以下・NaN・無限大が物理的に無意味な値に使う。
//! 0が有効な値（速度・摩擦・効果範囲など）には使わない。

/// `is_finite() && value > 0.0` を満たす値のみ生成できる。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Positive(f32);

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{0} is not a finite positive number")]
pub struct NotPositive(pub f32);

impl Positive {
    pub fn new(value: f32) -> Result<Self, NotPositive> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(NotPositive(value))
        }
    }

    pub const fn get(self) -> f32 {
        self.0
    }

    /// 積が正の有限値にならない場合（オーバーフロー・アンダーフロー）は `None`。
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        Self::new(self.0 * rhs.0).ok()
    }
}

impl TryFrom<f32> for Positive {
    type Error = NotPositive;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Positive> for f32 {
    fn from(value: Positive) -> Self {
        value.get()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::one(1.0)]
    #[case::smallest_normal(f32::MIN_POSITIVE)]
    #[case::smallest_subnormal(f32::from_bits(1))]
    #[case::largest(f32::MAX)]
    fn finite_positive_values_are_accepted(#[case] value: f32) {
        assert_eq!(Positive::new(value).map(Positive::get), Ok(value));
        assert_eq!(f32::from(Positive::try_from(value).expect("positive")), value);
    }

    #[rstest]
    #[case::zero(0.0)]
    #[case::negative_zero(-0.0)]
    #[case::negative(-1.0)]
    #[case::positive_infinity(f32::INFINITY)]
    #[case::negative_infinity(f32::NEG_INFINITY)]
    fn non_positive_or_infinite_values_are_rejected(#[case] value: f32) {
        assert_eq!(Positive::new(value), Err(NotPositive(value)));
    }

    #[rstest]
    fn nan_is_rejected() {
        // NaN同士は等しくないため、エラー値はビット列で確認する
        let err = Positive::new(f32::NAN).expect_err("NaN must be rejected");
        assert!(err.0.is_nan());
    }

    fn p(value: f32) -> Positive {
        Positive::new(value).expect("positive")
    }

    #[rstest]
    #[case::normal(2.0, 3.0, Some(6.0))]
    #[case::shrink(4.0, 0.5, Some(2.0))]
    #[case::overflow(f32::MAX, 2.0, None)]
    #[case::underflow_to_zero(1e-30, 1e-30, None)]
    fn checked_mul_keeps_result_finite_and_positive(
        #[case] a: f32,
        #[case] b: f32,
        #[case] expected: Option<f32>,
    ) {
        assert_eq!(p(a).checked_mul(p(b)).map(Positive::get), expected);
    }

    proptest! {
        #[test]
        fn new_accepts_exactly_finite_positive(value in any::<f32>()) {
            prop_assert_eq!(Positive::new(value).is_ok(), value.is_finite() && value > 0.0);
        }

        #[test]
        fn checked_mul_never_yields_invalid_value(
            a in any::<f32>().prop_filter_map("positive", |v| Positive::new(v).ok()),
            b in any::<f32>().prop_filter_map("positive", |v| Positive::new(v).ok()),
        ) {
            match a.checked_mul(b) {
                Some(product) => prop_assert!(product.get().is_finite() && product.get() > 0.0),
                None => {
                    let raw = a.get() * b.get();
                    prop_assert!(!raw.is_finite() || raw <= 0.0);
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::Positive;

    /// テスト用の簡潔な生成。不正値ならパニックする。
    pub fn p(value: f32) -> Positive {
        Positive::new(value).expect("positive")
    }
}
