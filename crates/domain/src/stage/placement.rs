use crate::math::Vec3;

/// 開始時のノーマルメダル配置。【仮決定】格子配置のみ。
#[derive(Debug, Clone, PartialEq)]
pub struct InitialMedalPlacement {
    origin: Vec3,
    columns: u32,
    rows: u32,
    layers: u32,
    spacing: Vec3,
}

impl InitialMedalPlacement {
    /// `origin` は格子の中心（最下段）。列はX、行はZ、段はY方向。
    pub fn grid(origin: Vec3, columns: u32, rows: u32, layers: u32, spacing: Vec3) -> Self {
        Self { origin, columns, rows, layers, spacing }
    }

    pub fn count(&self) -> usize {
        (self.columns * self.rows * self.layers) as usize
    }

    pub fn positions(&self) -> impl Iterator<Item = Vec3> + '_ {
        let half = Vec3::new(
            (self.columns.saturating_sub(1)) as f32 * self.spacing.x / 2.0,
            0.0,
            (self.rows.saturating_sub(1)) as f32 * self.spacing.z / 2.0,
        );
        (0..self.layers).flat_map(move |y| {
            (0..self.rows).flat_map(move |z| {
                (0..self.columns).map(move |x| {
                    self.origin - half + Vec3::new(x as f32, y as f32, z as f32) * self.spacing
                })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn grid_is_centered_on_origin() {
        let all: Vec<_> = InitialMedalPlacement::grid(Vec3::Y, 3, 2, 2, Vec3::ONE)
            .positions()
            .collect();
        assert_eq!(all.first(), Some(&Vec3::new(-1.0, 1.0, -0.5)));
        assert_eq!(all.last(), Some(&Vec3::new(1.0, 2.0, 0.5)));
    }

    #[rstest]
    #[case::no_columns(0, 3, 1)]
    #[case::no_layers(3, 3, 0)]
    fn empty_grid_has_no_positions(#[case] c: u32, #[case] r: u32, #[case] l: u32) {
        let p = InitialMedalPlacement::grid(Vec3::ZERO, c, r, l, Vec3::ONE);
        assert_eq!(p.count(), 0);
        assert_eq!(p.positions().count(), 0);
    }

    proptest! {
        #[test]
        fn count_matches_positions(c in 0u32..8, r in 0u32..8, l in 0u32..4) {
            let p = InitialMedalPlacement::grid(Vec3::ZERO, c, r, l, Vec3::ONE);
            prop_assert_eq!(p.positions().count(), p.count());
            prop_assert_eq!(p.count(), (c * r * l) as usize);
        }
    }
}
