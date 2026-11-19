use super::StageError;
use crate::{id::RailId, math::Vec3};

/// Pusher専用の軌道。曲線・円弧は点列で近似した折れ線として持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct Rail {
    id: RailId,
    points: Vec<Vec3>,
    /// 各点までの累積弧長。`cumulative[0] == 0`。
    cumulative: Vec<f32>,
}

impl Rail {
    pub fn new(id: RailId, points: Vec<Vec3>) -> Result<Self, StageError> {
        let cumulative: Vec<f32> = std::iter::once(0.0)
            .chain(points.windows(2).scan(0.0, |acc, w| {
                *acc += w[0].distance(w[1]);
                Some(*acc)
            }))
            .collect();
        match cumulative.last() {
            Some(&len) if points.len() >= 2 && len > 0.0 => Ok(Self { id, points, cumulative }),
            _ => Err(StageError::InvalidRail(id)),
        }
    }

    pub fn id(&self) -> RailId {
        self.id
    }

    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    pub fn length(&self) -> f32 {
        *self
            .cumulative
            .last()
            .expect("Rail::new guarantees at least 2 points")
    }

    /// 弧長 `s` の位置。範囲外は端に丸める。
    pub fn point_at(&self, s: f32) -> Vec3 {
        let s = s.clamp(0.0, self.length());
        let i = self
            .cumulative
            .partition_point(|&c| c <= s)
            .clamp(1, self.points.len() - 1);
        let (s0, s1) = (self.cumulative[i - 1], self.cumulative[i]);
        let t = if s1 > s0 {
            (s - s0) / (s1 - s0)
        } else {
            0.0
        };
        self.points[i - 1].lerp(self.points[i], t)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    fn l_shape() -> Rail {
        Rail::new(RailId(1), vec![Vec3::ZERO, Vec3::X * 2.0, Vec3::new(2.0, 0.0, 2.0)])
            .expect("valid")
    }

    #[rstest]
    #[case::one_point(vec![Vec3::ZERO])]
    #[case::no_points(vec![])]
    #[case::zero_length(vec![Vec3::ONE, Vec3::ONE])]
    fn invalid_rail_is_rejected(#[case] points: Vec<Vec3>) {
        assert_eq!(
            Rail::new(RailId(3), points).err(),
            Some(StageError::InvalidRail(RailId(3)))
        );
    }

    #[rstest]
    #[case::start(0.0, Vec3::ZERO)]
    #[case::first_segment(1.0, Vec3::X)]
    #[case::corner(2.0, Vec3::X * 2.0)]
    #[case::second_segment(3.0, Vec3::new(2.0, 0.0, 1.0))]
    #[case::end(4.0, Vec3::new(2.0, 0.0, 2.0))]
    #[case::clamped_high(99.0, Vec3::new(2.0, 0.0, 2.0))]
    #[case::clamped_low(-1.0, Vec3::ZERO)]
    fn point_at_walks_polyline_by_arc_length(#[case] s: f32, #[case] expected: Vec3) {
        let rail = l_shape();
        assert_eq!(rail.length(), 4.0);
        assert_eq!(rail.point_at(s), expected);
    }

    proptest! {
        /// 弧長で d 進んだ点は、直線距離で d より離れない（1-Lipschitz）。
        #[test]
        fn point_at_is_continuous(s in -1.0f32..5.0, d in 0.0f32..1.0) {
            let rail = l_shape();
            prop_assert!(rail.point_at(s).distance(rail.point_at(s + d)) <= d + 1e-4);
        }
    }
}
