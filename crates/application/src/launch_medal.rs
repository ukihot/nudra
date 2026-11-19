//! メダル射出（要件16.2、基本設計 4.4）。

use std::time::Duration;

use domain::{
    event::DomainEvent,
    launch::{CatapultState, LaunchError, LaunchPlan},
    session::{GameSession, GameSessionError},
    spec::{MedalType, SpecCatalog},
    stage::Stage,
};

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchOutput {
    pub plan: LaunchPlan,
    pub event: DomainEvent,
}

/// いずれも「操作の拒否」。呼び出し側で `match` し、エラーハンドラへ流さない（基本設計 8.2）。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchMedalError {
    #[error(transparent)]
    Session(#[from] GameSessionError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error("no spec for {0:?} medal")]
    NoSpec(MedalType),
}

pub fn launch_medal(
    session: &GameSession,
    stage: &Stage,
    state: &mut CatapultState,
    specs: &SpecCatalog,
    now: Duration,
) -> Result<LaunchOutput, LaunchMedalError> {
    session.ensure_playing()?;
    let medal = state.selected();
    let spec = session
        .medal_spec(medal, specs)
        .ok_or(LaunchMedalError::NoSpec(medal))?;
    let plan = state.fire(stage.catapult(), &spec, now)?;
    Ok(LaunchOutput { plan, event: DomainEvent::MedalLaunched { medal } })
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

        let out =
            launch_medal(&session, &stage, &mut state, &specs, Duration::ZERO).expect("launched");

        assert_eq!(out.event, DomainEvent::MedalLaunched { medal: MedalType::Normal });
        assert_eq!(out.plan.spec.mass.get(), medal_spec().mass.get() * 3.0);
        assert_eq!(out.plan.origin, stage.catapult().position());
    }

    #[rstest]
    fn second_launch_within_cooldown_is_rejected(
        session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut state = CatapultState::default();
        launch_medal(&session, &stage, &mut state, &specs, Duration::ZERO).expect("first");
        assert!(matches!(
            launch_medal(&session, &stage, &mut state, &specs, Duration::from_millis(10)),
            Err(LaunchMedalError::Launch(LaunchError::CoolingDown { .. }))
        ));
    }

    #[rstest]
    fn launch_after_game_over_is_rejected_without_firing(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let enemy = session.tick(&stage, Duration::ZERO)[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        let mut state = CatapultState::default();
        let before = state.clone();

        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Duration::ZERO),
            Err(LaunchMedalError::Session(GameSessionError::NotPlaying(
                session.status()
            )))
        );
        assert_eq!(state, before, "cooldown must not start");
        assert_eq!(enemy, EnemyId(1));
    }

    #[rstest]
    fn medal_without_spec_is_rejected(session: GameSession, stage: Stage, specs: SpecCatalog) {
        let mut state = CatapultState::default();
        state.select(MedalType::Special(SpecialKind::Fire));
        let before = state.clone();
        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Duration::ZERO),
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

        let out =
            launch_medal(&session, &stage, &mut state, &specs, Duration::ZERO).expect("launched");

        assert_eq!(out.event, DomainEvent::MedalLaunched { medal: heavy });
        assert_eq!(out.plan.medal, heavy);
    }
}
