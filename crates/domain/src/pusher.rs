//! PusherCharacter集約とインゲームの移動状態（基本設計 3.5）。

use crate::{id::PusherCharacterId, math::Vec3, positive::Positive, stage::Rail};

/// Pusherの能力値。基礎値（BaseStats）と成長補正後の値（CurrentStats）を同じ型で表す。
/// 要件6.2「自律的に往復運動する」ため、速度を含め全項目が正の有限値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PusherStats {
    /// Rail上の移動速度（m/s）
    pub speed: Positive,
    pub mass: Positive,
    /// 最大推力（N）。これを超える抵抗では押し切れない。
    pub push_force: Positive,
    /// 押し出し面の幅（m）
    pub push_width: Positive,
}

/// 【仮決定】端で停止して折り返す往復のみ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MovementPattern {
    PingPong { pause_at_ends: f32 },
}

/// 【未確定】Abilitiesは仕様確定まで持たない。
#[derive(Debug, Clone, PartialEq)]
pub struct PusherCharacter {
    pub id: PusherCharacterId,
    pub name: String,
    pub base_stats: PusherStats,
    pub movement: MovementPattern,
}

/// Rail上の移動状態。ECS Componentの中身として保持し、Domainに別のコピーは持たない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementState {
    s: f32,
    forward: bool,
    pause_left: f32,
}

impl MovementState {
    pub fn start(_rail: &Rail) -> Self {
        Self { s: 0.0, forward: true, pause_left: 0.0 }
    }

    /// 純関数。`dt` 秒進めた状態を返す（端で停止したあと反転）。
    #[must_use]
    pub fn advance(self, rail: &Rail, pattern: &MovementPattern, speed: f32, dt: f32) -> Self {
        let MovementPattern::PingPong { pause_at_ends } = *pattern;
        if self.pause_left > 0.0 {
            return Self {
                pause_left: (self.pause_left - dt).max(0.0),
                ..self
            };
        }
        let len = rail.length();
        let s = self.s
            + if self.forward {
                speed * dt
            } else {
                -speed * dt
            };
        if s >= len || s <= 0.0 {
            Self {
                s: s.clamp(0.0, len),
                forward: !self.forward,
                pause_left: pause_at_ends,
            }
        } else {
            Self { s, ..self }
        }
    }

    pub fn target(&self, rail: &Rail) -> Vec3 {
        rail.point_at(self.s)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::id::RailId;

    const PATTERN: MovementPattern = MovementPattern::PingPong { pause_at_ends: 0.5 };

    #[fixture]
    fn rail() -> Rail {
        Rail::new(RailId(1), vec![Vec3::ZERO, Vec3::X * 2.0]).expect("valid")
    }

    #[rstest]
    #[case::moves_forward(&[1.5], 1.5)]
    #[case::stops_at_end(&[1.5, 1.0], 2.0)]
    #[case::pauses_at_end(&[1.5, 1.0, 0.25], 2.0)]
    #[case::reverses_after_pause(&[1.5, 1.0, 0.5, 0.5], 1.5)]
    #[case::returns_to_start(&[2.0, 0.5, 3.0], 0.0)]
    fn ping_pong_along_rail(rail: Rail, #[case] steps: &[f32], #[case] expected_x: f32) {
        let state = steps
            .iter()
            .fold(MovementState::start(&rail), |s, &dt| {
                s.advance(&rail, &PATTERN, 1.0, dt)
            });
        assert_eq!(state.target(&rail), Vec3::X * expected_x);
    }

    proptest! {
        #[test]
        fn target_never_leaves_rail(steps in prop::collection::vec(0.0f32..2.0, 0..100), speed in 0.0f32..10.0) {
            let rail = rail();
            let mut state = MovementState::start(&rail);
            for dt in steps {
                state = state.advance(&rail, &PATTERN, speed, dt);
                let x = state.target(&rail).x;
                prop_assert!((0.0..=2.0).contains(&x), "x = {x}");
            }
        }
    }
}
