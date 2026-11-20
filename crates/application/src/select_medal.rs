//! 射出するメダル種の選択（基本設計 3.12）。

use domain::{
    id::MedalId,
    launch::CatapultState,
    session::{GameSession, GameSessionError},
};

/// 選べない理由。進行できない間（終了後・進化の選択中）と、所持していないメダル種。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SelectMedalError {
    #[error(transparent)]
    Session(#[from] GameSessionError),
    #[error("{0} medal is not owned")]
    NotOwned(MedalId),
}

/// 失敗時は選択を変えない。
pub fn select_medal(
    session: &GameSession,
    catapult: &mut CatapultState,
    medal: MedalId,
) -> Result<(), SelectMedalError> {
    session.ensure_running()?;
    if session.medals().level(&medal).is_none() {
        return Err(SelectMedalError::NotOwned(medal));
    }
    catapult.select(medal);
    Ok(())
}

#[cfg(test)]
mod tests {
    use domain::{
        evolution::Evolution,
        id::{EnemyId, GoalId},
        spec::SpecCatalog,
        stage::Stage,
        time::Tick,
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{fire, session, specs, stage};

    /// New! で提示された素材を取得する（test_support: Easy は1枚で満タン）。
    fn acquire_offer(session: &mut GameSession, specs: &SpecCatalog) -> MedalId {
        session.medals_fell(1, specs);
        let offer = session.evolution_offer().cloned().expect("offered");
        session
            .evolve(Evolution::Acquire(offer.clone()), specs)
            .expect("acquired");
        offer
    }

    #[rstest]
    fn owned_medal_is_selected(mut session: GameSession, stage: Stage, specs: SpecCatalog) {
        let medal = acquire_offer(&mut session, &specs);
        let mut catapult = CatapultState::new(stage.catapult());
        assert_eq!(select_medal(&session, &mut catapult, medal.clone()), Ok(()));
        assert_eq!(catapult.selected(), &medal);
    }

    #[rstest]
    fn unowned_medal_is_rejected_without_change(session: GameSession, stage: Stage) {
        let mut catapult = CatapultState::new(stage.catapult());
        assert_eq!(
            select_medal(&session, &mut catapult, fire()),
            Err(SelectMedalError::NotOwned(fire()))
        );
        assert_eq!(catapult.selected(), &MedalId::normal());
    }

    #[rstest]
    fn selection_waits_while_an_evolution_is_pending(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        session.medals_fell(1, &specs);
        let mut catapult = CatapultState::new(stage.catapult());
        assert_eq!(
            select_medal(&session, &mut catapult, MedalId::normal()),
            Err(SelectMedalError::Session(GameSessionError::EvolutionPending))
        );
    }

    #[rstest]
    fn selection_after_game_over_is_rejected(mut session: GameSession, stage: Stage) {
        session.advance(&stage, Tick(0));
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        let mut catapult = CatapultState::new(stage.catapult());
        assert_eq!(
            select_medal(&session, &mut catapult, MedalId::normal()),
            Err(SelectMedalError::Session(GameSessionError::NotPlaying(
                session.status()
            )))
        );
    }
}
