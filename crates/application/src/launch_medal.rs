//! メダル射出（要件16.2、基本設計 4.4, 6.4）。
//!
//! 発射ボタンは継続的な意図であり、要求ではない。意図がある間は毎tickこれを呼び、
//! 発射条件（クールダウン）が成立したときだけ発射する。成立していなければ `Ok(None)`。
//! クールダウン中に発射しないことは却下ではない。発射した事実だけを `MedalLaunched` で通知する。

use domain::{
    event::DomainEvent,
    id::MedalCode,
    launch::{CatapultState, LaunchPlan},
    session::{GameSession, GameSessionError},
    spec::SpecCatalog,
    stage::Stage,
    time::Tick,
};

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchOutput {
    pub plan: LaunchPlan,
    pub event: DomainEvent,
}

/// 発射の前提が崩れている場合（終了後・進化の選択中のセッション、所持していないメダル種）。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LaunchMedalError {
    #[error(transparent)]
    Session(#[from] GameSessionError),
    #[error("{0:?} medal is not owned")]
    NotOwned(MedalCode),
}

pub fn launch_medal(
    session: &GameSession,
    stage: &Stage,
    state: &mut CatapultState,
    specs: &SpecCatalog,
    now: Tick,
) -> Result<Option<LaunchOutput>, LaunchMedalError> {
    session.ensure_running()?;
    let medal = state.selected().clone();
    let spec = session
        .medal_spec(&medal, specs)
        .ok_or_else(|| LaunchMedalError::NotOwned(medal.clone()))?;
    Ok(state
        .fire(stage.catapult(), &spec, now)
        .map(|plan| LaunchOutput { plan, event: DomainEvent::MedalLaunched { medal } }))
}

#[cfg(test)]
mod tests {
    use domain::{
        evolution::Evolution,
        id::{EnemyId, GoalId},
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{fire, levels, session, specs, stage};

    #[rstest]
    fn launches_the_medal_at_its_current_level(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        session.medals_fell(1.0, &specs);
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &specs)
            .expect("evolved");
        let mut state = CatapultState::new(stage.catapult());

        let out = launch_medal(&session, &stage, &mut state, &specs, Tick(0))
            .expect("playing")
            .expect("ready");

        assert_eq!(out.event, DomainEvent::MedalLaunched { medal: MedalCode::normal() });
        assert_eq!(out.plan.spec, levels()[1], "level 2 spec");
        assert_eq!(out.plan.origin, stage.catapult().position());
    }

    #[rstest]
    fn no_launch_between_fire_intervals_is_not_an_error(
        session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut state = CatapultState::new(stage.catapult());
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
        let mut state = CatapultState::new(stage.catapult());
        let before = state.clone();

        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Tick(0)),
            Err(LaunchMedalError::Session(GameSessionError::NotPlaying(
                session.status()
            )))
        );
        assert_eq!(state, before, "the fire interval must not start");
        assert_eq!(enemy, EnemyId(1));
    }

    #[rstest]
    fn no_launch_while_an_evolution_is_pending(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        session.medals_fell(1.0, &specs);
        let mut state = CatapultState::new(stage.catapult());
        let before = state.clone();
        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Tick(0)),
            Err(LaunchMedalError::Session(GameSessionError::EvolutionPending))
        );
        assert_eq!(state, before);
    }

    #[rstest]
    fn unowned_medal_cannot_be_launched(session: GameSession, stage: Stage, specs: SpecCatalog) {
        let mut state = CatapultState::new(stage.catapult());
        state.select(fire());
        let before = state.clone();
        assert_eq!(
            launch_medal(&session, &stage, &mut state, &specs, Tick(0)),
            Err(LaunchMedalError::NotOwned(fire()))
        );
        assert_eq!(state, before);
    }
}
