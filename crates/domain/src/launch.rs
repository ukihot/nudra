//! カタパルトの操作状態と射出（MedalLaunchService相当、基本設計 3.8）。
//!
//! 発射ボタンは「発射したい」という継続的な意図であり、要求ではない（基本設計 6.4）。
//! 意図がある間は毎tick `fire` を評価し、クールダウンが明けたときだけ発射する。
//! クールダウン中に発射されないのは却下（エラー）ではなく、まだ条件が成立していないだけ。

use crate::{
    math::Vec3,
    spec::{MedalSpec, MedalType},
    stage::Catapult,
    time::Tick,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchPlan {
    pub medal: MedalType,
    pub spec: MedalSpec,
    pub origin: Vec3,
    pub velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatapultState {
    yaw: f32,
    selected: MedalType,
    /// 次に発射できるTick（期限）。毎tick残り時間を減らさず、現在のTickと比較する
    next_fire: Tick,
}

impl Default for CatapultState {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            selected: MedalType::Normal,
            next_fire: Tick::ZERO,
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

    pub fn next_fire(&self) -> Tick {
        self.next_fire
    }

    /// 発射できるTickか。
    pub fn is_ready(&self, now: Tick) -> bool {
        now >= self.next_fire
    }

    /// 発射条件が成立していれば発射し、計画を返す。成立していなければ `None`（状態は変わらない）。
    pub fn fire(&mut self, catapult: &Catapult, spec: &MedalSpec, now: Tick) -> Option<LaunchPlan> {
        if !self.is_ready(now) {
            return None;
        }
        self.next_fire = now + catapult.cooldown();
        Some(LaunchPlan {
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
        time::Ticks,
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
    fn first_shot_fires_forward_from_catapult(stage: Stage) {
        let mut state = CatapultState::default();
        assert!(state.is_ready(Tick::ZERO));
        let plan = state
            .fire(stage.catapult(), &medal(), Tick::ZERO)
            .expect("ready");
        assert_eq!(plan.origin, stage.catapult().position());
        assert_eq!(plan.medal, MedalType::Normal);
        assert!(plan.velocity.z < 0.0 && plan.velocity.y > 0.0, "{:?}", plan.velocity);
        assert!((plan.velocity.length() - 10.0).abs() < 1e-4);
    }

    /// 例: クールダウン30Tick。Tick 105 で発射 → 次は Tick 135 から。
    /// Tick 120 では発射されず、Tick 140 で再び発射する。
    #[rstest]
    fn deadline_tick_decides_the_next_shot(stage: Stage) {
        let catapult = stage.catapult().clone();
        let catapult = Catapult::new(catapult.position(), 0.0, (0.0, 0.0), 0.2, 10.0, Ticks(30))
            .expect("valid");
        let mut state = CatapultState::default();
        assert!(state.fire(&catapult, &medal(), Tick(105)).is_some());
        assert_eq!(state.next_fire(), Tick(135));

        let before = state.clone();
        assert!(!state.is_ready(Tick(120)));
        assert_eq!(state.fire(&catapult, &medal(), Tick(120)), None);
        assert_eq!(state, before, "no shot leaves the state unchanged");

        assert!(state.is_ready(Tick(135)));
        assert!(state.fire(&catapult, &medal(), Tick(140)).is_some());
        assert_eq!(state.next_fire(), Tick(170));
    }

    /// 発射ボタンを押し続けたとき、毎tick評価しても発射はクールダウンごとにしか起きない。
    /// 例: クールダウン12Tick・180Tick（60Hzで3秒）押し続け → Tick 0, 12, …, 180 の16回。
    #[rstest]
    fn holding_fire_shoots_once_per_cooldown(stage: Stage) {
        let mut state = CatapultState::default();
        let shots = (0..=180)
            .filter_map(|t| state.fire(stage.catapult(), &medal(), Tick(t)))
            .count();
        assert_eq!(shots, 16);
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

        /// どんなTickの列で評価しても、2つの発射の間隔はクールダウン以上。
        #[test]
        fn shots_are_never_closer_than_cooldown(steps in prop::collection::vec(0u64..30, 1..200)) {
            let stage = stage(parts());
            let mut state = CatapultState::default();
            let mut now = Tick::ZERO;
            let mut last: Option<Tick> = None;
            for step in steps {
                now = now + Ticks(step);
                if state.fire(stage.catapult(), &medal(), now).is_some() {
                    if let Some(prev) = last {
                        prop_assert!(now - prev >= stage.catapult().cooldown());
                    }
                    last = Some(now);
                }
            }
        }
    }
}
