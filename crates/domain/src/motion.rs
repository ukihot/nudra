//! メダルの挙動ステータス（基本設計 3.15）。
//!
//! 能力の発動（着地・静止・再始動）と、見た目（アニメ・アセットの切り替え）は同じこの状態で決まる。
//! 状態は描画ではなく物理（固定ステップ）から決める: ECS が毎 tick の速度と接地を測って
//! [`MedalMotion::observe`] に渡し、 結果を Component として持つ。Presentation はその Component
//! を読むだけにする。
//!
//! ```text
//! 射出 ──► 飛行中 ──着地──► 転がり中 ⇄ 静止
//!            ▲                 │
//!            └──── 浮かされた ──┘（上へ押されると飛行中に戻り、着地でまた発動する）
//! ```

use crate::{
    positive::Positive,
    time::{Tick, Ticks},
};

/// 挙動の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// 射出されてから着地するまで（浮かされた後も）
    Flying,
    Rolling,
    Resting,
}

/// 状態が変わった出来事。能力の発動条件になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionEvent {
    /// 飛行中 → 盤面
    Landed,
    /// 転がり中 → 静止
    Rested,
    /// 静止 → 転がり中（押された）
    Woke,
}

/// 1 tick の物理の観測（ECS が測る）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSample {
    /// 速さ（m/s）
    pub speed: f32,
    /// 盤面（床・他のメダル）に触れている
    pub grounded: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MotionRulesError {
    #[error("wake speed must be greater than rest speed (hysteresis)")]
    NoHysteresis,
    #[error("rest delay must be at least one tick")]
    ZeroRestDelay,
}

/// 状態遷移のしきい値。静止は「`rest_speed` 未満が `rest_delay` 続いた」、再始動は「`wake_speed`
/// を超えた」。 `rest_speed < wake_speed`
/// のヒステリシスで、止まりかけのメダルが静止と転がりを行き来しないようにする。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionRules {
    rest_speed: Positive,
    wake_speed: Positive,
    rest_delay: Ticks,
}

impl MotionRules {
    pub fn new(
        rest_speed: Positive,
        wake_speed: Positive,
        rest_delay: Ticks,
    ) -> Result<Self, MotionRulesError> {
        if wake_speed <= rest_speed {
            return Err(MotionRulesError::NoHysteresis);
        }
        if rest_delay == Ticks(0) {
            return Err(MotionRulesError::ZeroRestDelay);
        }
        Ok(Self { rest_speed, wake_speed, rest_delay })
    }
}

impl Default for MotionRules {
    /// 秒速5cm未満が0.2秒（60Hzで12tick）続いたら静止、秒速15cmを超えたら再始動。
    fn default() -> Self {
        Self {
            rest_speed: Positive::new(0.05).expect("0.05 is positive"),
            wake_speed: Positive::new(0.15).expect("0.15 is positive"),
            rest_delay: Ticks(12),
        }
    }
}

/// 個体（メダル1枚）の挙動状態。ECS の Component が値として持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MedalMotion {
    state: Motion,
    /// 転がり中に遅くなり始めた Tick（静止の判定に使う）
    slow_since: Option<Tick>,
}

impl MedalMotion {
    /// 射出（スポーン）した直後は飛行中。
    pub fn launched() -> Self {
        Self { state: Motion::Flying, slow_since: None }
    }

    /// 盤面に置かれた状態から始める（初期配置のメダル）。
    pub fn placed() -> Self {
        Self { state: Motion::Resting, slow_since: None }
    }

    pub fn state(&self) -> Motion {
        self.state
    }

    /// 観測を取り込む。状態が変わったときだけ出来事を返す（1回の観測で高々1つ）。
    pub fn observe(
        &mut self,
        sample: MotionSample,
        now: Tick,
        rules: &MotionRules,
    ) -> Option<MotionEvent> {
        let slow = sample.speed < rules.rest_speed.get();
        match self.state {
            Motion::Flying if sample.grounded => {
                self.state = Motion::Rolling;
                self.slow_since = slow.then_some(now);
                Some(MotionEvent::Landed)
            },
            // 跳ねて浮いても飛行中には戻さない（浮かせるのは `lift` だけ）
            Motion::Flying => None,
            Motion::Rolling if slow => {
                let since = *self.slow_since.get_or_insert(now);
                (now - since >= rules.rest_delay).then(|| {
                    self.state = Motion::Resting;
                    self.slow_since = None;
                    MotionEvent::Rested
                })
            },
            Motion::Rolling => {
                self.slow_since = None;
                None
            },
            Motion::Resting => (sample.speed > rules.wake_speed.get()).then(|| {
                self.state = Motion::Rolling;
                MotionEvent::Woke
            }),
        }
    }

    /// 上へ押された（浮かされた）。飛行中に戻り、次の着地でまた `Landed` になる。
    pub fn lift(&mut self) {
        *self = Self::launched();
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::positive::fixture::p;

    /// 0.1 未満が 3 tick 続いたら静止、0.5 を超えたら再始動。
    #[fixture]
    fn rules() -> MotionRules {
        MotionRules::new(p(0.1), p(0.5), Ticks(3)).expect("valid")
    }

    fn air(speed: f32) -> MotionSample {
        MotionSample { speed, grounded: false }
    }

    fn ground(speed: f32) -> MotionSample {
        MotionSample { speed, grounded: true }
    }

    /// 観測を Tick 0 から順に与え、出来事を集める。
    fn run(
        motion: &mut MedalMotion,
        samples: &[MotionSample],
        rules: &MotionRules,
    ) -> Vec<(u64, MotionEvent)> {
        samples
            .iter()
            .zip(0u64..)
            .filter_map(|(s, t)| motion.observe(*s, Tick(t), rules).map(|e| (t, e)))
            .collect()
    }

    #[rstest]
    #[case::hysteresis(0.1, 0.5, 3, None)]
    #[case::same_speeds(0.5, 0.5, 3, Some(MotionRulesError::NoHysteresis))]
    #[case::inverted(0.5, 0.1, 3, Some(MotionRulesError::NoHysteresis))]
    #[case::zero_delay(0.1, 0.5, 0, Some(MotionRulesError::ZeroRestDelay))]
    fn rules_are_validated(
        #[case] rest: f32,
        #[case] wake: f32,
        #[case] delay: u64,
        #[case] error: Option<MotionRulesError>,
    ) {
        assert_eq!(MotionRules::new(p(rest), p(wake), Ticks(delay)).err(), error);
    }

    #[rstest]
    fn default_rules_are_valid() {
        let d = MotionRules::default();
        assert_eq!(MotionRules::new(d.rest_speed, d.wake_speed, d.rest_delay), Ok(d));
    }

    #[rstest]
    fn launched_medals_fly_until_they_touch_the_board(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        assert_eq!(m.state(), Motion::Flying);
        let events = run(&mut m, &[air(9.0), air(8.0), ground(7.0)], &rules);
        assert_eq!(events, [(2, MotionEvent::Landed)]);
        assert_eq!(m.state(), Motion::Rolling);
    }

    #[rstest]
    fn a_rolling_medal_rests_after_staying_slow_long_enough(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        let events = run(
            &mut m,
            &[ground(1.0), ground(0.05), ground(0.05), ground(0.05), ground(0.05)],
            &rules,
        );
        // Tick 1 から遅い。3 tick 経った Tick 4 で静止
        assert_eq!(events, [(0, MotionEvent::Landed), (4, MotionEvent::Rested)]);
        assert_eq!(m.state(), Motion::Resting);
    }

    #[rstest]
    fn exactly_the_rest_speed_is_not_slow(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        let events = run(&mut m, &[ground(0.1); 6], &rules);
        assert_eq!(events, [(0, MotionEvent::Landed)]);
        assert_eq!(m.state(), Motion::Rolling);
    }

    #[rstest]
    fn speeding_up_restarts_the_rest_timer(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        let events = run(
            &mut m,
            &[
                ground(1.0),
                ground(0.05),
                ground(0.05),
                ground(0.2),
                ground(0.05),
                ground(0.05),
                ground(0.05),
                ground(0.05),
            ],
            &rules,
        );
        assert_eq!(events, [(0, MotionEvent::Landed), (7, MotionEvent::Rested)]);
    }

    #[rstest]
    fn a_resting_medal_wakes_only_above_the_wake_speed(rules: MotionRules) {
        let mut m = MedalMotion::placed();
        let events = run(&mut m, &[ground(0.3), ground(0.5), ground(0.6)], &rules);
        assert_eq!(
            events,
            [(2, MotionEvent::Woke)],
            "0.3 and 0.5 are within the hysteresis"
        );
        assert_eq!(m.state(), Motion::Rolling);
    }

    #[rstest]
    fn bouncing_does_not_make_a_medal_fly_again(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        let events = run(&mut m, &[ground(2.0), air(2.0), ground(2.0), air(2.0)], &rules);
        assert_eq!(events, [(0, MotionEvent::Landed)]);
        assert_eq!(m.state(), Motion::Rolling);
    }

    #[rstest]
    #[case::rolling(MedalMotion::launched(), &[ground(2.0)])]
    #[case::resting(MedalMotion::placed(), &[])]
    fn a_lifted_medal_flies_and_lands_again(
        #[case] mut m: MedalMotion,
        #[case] before: &[MotionSample],
        rules: MotionRules,
    ) {
        run(&mut m, before, &rules);
        m.lift();
        assert_eq!(m.state(), Motion::Flying);
        assert_eq!(m.observe(ground(1.0), Tick(50), &rules), Some(MotionEvent::Landed));
    }

    #[rstest]
    fn a_lifted_medal_starts_its_rest_timer_again(rules: MotionRules) {
        let mut m = MedalMotion::launched();
        run(&mut m, &[ground(0.05), ground(0.05)], &rules);
        m.lift();
        let events = run(
            &mut m,
            &[ground(0.05), ground(0.05), ground(0.05), ground(0.05)],
            &rules,
        );
        assert_eq!(events, [(0, MotionEvent::Landed), (3, MotionEvent::Rested)]);
    }

    fn sample() -> impl Strategy<Value = MotionSample> {
        (0.0f32..1.0, any::<bool>()).prop_map(|(speed, grounded)| MotionSample { speed, grounded })
    }

    proptest! {
        /// 出来事は状態が変わったときだけで、その状態と矛盾しない。静止は遅い観測が続いた後だけ。
        #[test]
        fn events_match_state_changes(samples in prop::collection::vec(sample(), 0..60), lifts in prop::collection::vec(any::<bool>(), 60)) {
            let rules = rules();
            let mut m = MedalMotion::launched();
            let mut slow_run = 0u64;
            for (t, (s, lift)) in samples.iter().zip(lifts).enumerate() {
                if lift {
                    m.lift();
                    slow_run = 0;
                }
                let before = m.state();
                let event = m.observe(*s, Tick(t as u64), &rules);
                slow_run = if s.speed < 0.1 && m.state() != Motion::Flying { slow_run + 1 } else { 0 };
                match event {
                    None => prop_assert_eq!(m.state(), before),
                    Some(MotionEvent::Landed) => prop_assert_eq!((before, m.state()), (Motion::Flying, Motion::Rolling)),
                    Some(MotionEvent::Rested) => {
                        prop_assert_eq!((before, m.state()), (Motion::Rolling, Motion::Resting));
                        prop_assert!(slow_run > 3, "rested after {} slow samples", slow_run);
                    },
                    Some(MotionEvent::Woke) => prop_assert_eq!((before, m.state()), (Motion::Resting, Motion::Rolling)),
                }
            }
        }
    }
}
