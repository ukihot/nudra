//! カタパルトの操作状態と射出計画（MedalLaunchService相当、基本設計 3.8）。

use std::time::Duration;

use crate::{
    math::Vec3,
    spec::{MedalSpec, MedalType},
    stage::Catapult,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchPlan {
    pub medal: MedalType,
    pub spec: MedalSpec,
    pub origin: Vec3,
    pub velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchError {
    #[error("catapult is cooling down ({remaining:?} left)")]
    CoolingDown { remaining: Duration },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatapultState {
    yaw: f32,
    selected: MedalType,
    last_fired: Option<Duration>,
}

impl Default for CatapultState {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            selected: MedalType::Normal,
            last_fired: None,
        }
    }
}

impl CatapultState {
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn selected(&self) -> MedalType {
        self.selected
    }

    /// 範囲外は丸める（エラーにしない）。
    pub fn aim(&mut self, delta: f32, catapult: &Catapult) {
        self.yaw = catapult.clamp_yaw(self.yaw + delta);
    }

    pub fn select(&mut self, medal: MedalType) {
        self.selected = medal;
    }

    /// 成功時のみ `last_fired` を更新する（全か無か）。
    pub fn fire(
        &mut self,
        catapult: &Catapult,
        spec: &MedalSpec,
        now: Duration,
    ) -> Result<LaunchPlan, LaunchError> {
        if let Some(ready_at) = self.last_fired.map(|t| t + catapult.cooldown())
            && now < ready_at
        {
            return Err(LaunchError::CoolingDown { remaining: ready_at - now });
        }
        self.last_fired = Some(now);
        Ok(LaunchPlan {
            medal: self.selected,
            spec: *spec,
            origin: catapult.position(),
            velocity: catapult.launch_velocity(self.yaw),
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;
    use crate::{
        spec::fixture::medal,
        stage::{
            Stage,
            fixture::{parts, stage},
        },
    };

    #[rstest]
    #[case::within(0.3, 0.3)]
    #[case::over_max(10.0, 0.5)]
    #[case::under_min(-10.0, -0.5)]
    fn aim_is_clamped_to_catapult_range(stage: Stage, #[case] delta: f32, #[case] expected: f32) {
        let mut state = CatapultState::default();
        state.aim(delta, stage.catapult());
        assert_eq!(state.yaw(), expected);
    }

    #[rstest]
    fn first_launch_fires_forward_from_catapult(stage: Stage) {
        let plan = CatapultState::default()
            .fire(stage.catapult(), &medal(), Duration::ZERO)
            .expect("ready");
        assert_eq!(plan.origin, stage.catapult().position());
        assert_eq!(plan.medal, MedalType::Normal);
        assert!(
            plan.velocity.z < 0.0 && plan.velocity.y > 0.0,
            "forward and upward: {:?}",
            plan.velocity
        );
        assert!((plan.velocity.length() - 10.0).abs() < 1e-4);
    }

    #[rstest]
    #[case::just_after(50, 150)]
    #[case::just_before_ready(199, 1)]
    fn launch_during_cooldown_is_rejected_without_state_change(
        stage: Stage,
        #[case] after_ms: u64,
        #[case] remaining_ms: u64,
    ) {
        let mut state = CatapultState::default();
        state
            .fire(stage.catapult(), &medal(), Duration::ZERO)
            .expect("first shot");
        let before = state.clone();
        assert_eq!(
            state.fire(stage.catapult(), &medal(), Duration::from_millis(after_ms)),
            Err(LaunchError::CoolingDown { remaining: Duration::from_millis(remaining_ms) })
        );
        assert_eq!(state, before);
    }

    #[rstest]
    fn launch_is_allowed_once_cooldown_elapsed(stage: Stage) {
        let mut state = CatapultState::default();
        state
            .fire(stage.catapult(), &medal(), Duration::ZERO)
            .expect("first shot");
        assert!(
            state
                .fire(stage.catapult(), &medal(), Duration::from_millis(200))
                .is_ok()
        );
    }

    proptest! {
        #[test]
        fn yaw_never_leaves_range(deltas in prop::collection::vec(-5.0f32..5.0, 0..50)) {
            let stage = stage(parts());
            let mut state = CatapultState::default();
            for d in deltas {
                state.aim(d, stage.catapult());
                prop_assert!((-0.5..=0.5).contains(&state.yaw()));
            }
        }
    }
}
