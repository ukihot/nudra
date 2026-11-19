//! メダル射出（要件16.2、基本設計 4.4, 6.4）。
//!
//! 発射ボタンは継続的な意図であり、要求ではない。意図がある間は毎tickこれを呼び、
//! 発射条件（クールダウン）が成立したときだけ発射する。成立していなければ `Ok(None)`。
//! クールダウン中に発射しないことは却下ではない。発射した事実だけを `MedalLaunched` で通知する。

use domain::{
    event::DomainEvent,
    launch::{CatapultState, LaunchPlan},
    session::{GameSession, GameSessionError},
    spec::{MedalType, SpecCatalog},
    stage::Stage,
    time::Tick,
};

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchOutput {
    pub plan: LaunchPlan,
    pub event: DomainEvent,
}

/// 発射の前提が崩れている場合（終了後のセッション、定義データの欠落）。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchMedalError {
    #[error(transparent)]
    Session(#[from] GameSessionError),
    #[error("no spec for {0:?} medal")]
    NoSpec(MedalType),
}

pub fn launch_medal(
    session: &GameSession,
    stage: &Stage,
    state: &mut CatapultState,
    specs: &SpecCatalog,
    now: Tick,
) -> Result<Option<LaunchOutput>, LaunchMedalError> {
    session.ensure_playing()?;
    let medal = state.selected();
    let spec = session
        .medal_spec(medal, specs)
        .ok_or(LaunchMedalError::NoSpec(medal))?;
    Ok(state
        .fire(stage.catapult(), &spec, now)
        .map(|plan| LaunchOutput { plan, event: DomainEvent::MedalLaunched { medal } }))
}

#[cfg(test)]
mod tests {
    use domain::{
        id::{EnemyId, GoalId},
        session::Upgrade,
        spec::SpecialKind,
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{character, medal_spec, p, session, specs, stage};

    #[rstest]
    fn launches_level_adjusted_medal(mut session: GameSession, stage: Stage, specs: SpecCatalog) {
        session
            .apply_level_up(
                Upgrade::MedalMass { medal: MedalType::Normal, factor: p(3.0) },
                &specs,
                &character(),
            )
            .expect("playing");
        let mut state = CatapultState::default();

        let out = launch_medal(&session, &stage, &mut state, &specs, Tick(0))
            .expect("playing")
            .expect("ready");

        assert_eq!(out.event, DomainEvent::MedalLaunched { medal: MedalType::Normal });
        assert_eq!(out.plan.spec.mass.get(), medal_spec().mass.get() * 3.0);
        assert_eq!(out.plan.origin, stage.catapult().position());
    }

    #[rstest]
    fn no_launch_while_cooling_down_is_not_an_error(
        session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut state = CatapultState::default();
        launch_medal(&session, &stage, &mut state, &specs, Tick(0))
            .expect("playing")
            .expect("first");
        assert_eq!(launch_medal(&session, &stage, &mut state, &specs, Tick(1)), Ok(None));
        let next = state.next_fire();
        assert!(
            launch_medal(&session, &stage, &mut state, &specs, next)
                .expect("playing")
                .is_some()
        );
    }

    #[rstest]
    fn launch_after_game_over_fails_without_firing(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        let mut state = CatapultState::default();
        let before = state.clone();

        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Tick(0)),
            Err(LaunchMedalError::Session(GameSessionError::NotPlaying(
                session.status()
            )))
        );
        assert_eq!(state, before, "cooldown must not start");
        assert_eq!(enemy, EnemyId(1));
    }

    #[rstest]
    fn medal_without_spec_fails(session: GameSession, stage: Stage, specs: SpecCatalog) {
        let mut state = CatapultState::default();
        state.select(MedalType::Special(SpecialKind::Fire));
        let before = state.clone();
        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Tick(0)),
            Err(LaunchMedalError::NoSpec(MedalType::Special(SpecialKind::Fire)))
        );
        assert_eq!(state, before);
    }

    #[rstest]
    fn selected_special_medal_is_launched(session: GameSession, stage: Stage) {
        let heavy = MedalType::Special(SpecialKind::Heavy);
        let specs = SpecCatalog::new(
            std::collections::HashMap::from([
                (MedalType::Normal, medal_spec()),
                (heavy, medal_spec()),
            ]),
            std::collections::HashMap::new(),
        )
        .expect("valid specs");
        let mut state = CatapultState::default();
        state.select(heavy);

        let out = launch_medal(&session, &stage, &mut state, &specs, Tick(0))
            .expect("playing")
            .expect("ready");

        assert_eq!(out.event, DomainEvent::MedalLaunched { medal: heavy });
        assert_eq!(out.plan.medal, heavy);
    }
}
