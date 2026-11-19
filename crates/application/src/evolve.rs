//! メダル進化（基本設計 3.12）。進化画面で選んだレベルアップ・錬成を実行する。

use domain::{
    event::DomainEvent,
    evolution::Evolution,
    id::MedalId,
    session::{GameSession, GameSessionError},
    spec::{MedalSpec, SpecCatalog},
};

/// ECSへ1回だけ反映すべき結果: 進化したメダル種と、その新しい物理特性。
#[derive(Debug, Clone, PartialEq)]
pub struct EvolveOutput {
    pub event: DomainEvent,
    pub medal: Option<(MedalId, MedalSpec)>,
}

pub fn evolve(
    session: &mut GameSession,
    specs: &SpecCatalog,
    evolution: Evolution,
) -> Result<EvolveOutput, GameSessionError> {
    let event = session.evolve(evolution, specs)?;
    let medal = match &event {
        DomainEvent::Evolved { medal, .. } => session
            .medal_spec(medal, specs)
            .map(|spec| (medal.clone(), spec)),
        _ => None,
    };
    Ok(EvolveOutput { event, medal })
}

#[cfg(test)]
mod tests {
    use domain::{
        evolution::{EvolutionError, MedalLevel},
        id::{EnemyId, GoalId},
        stage::Stage,
        time::Tick,
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{levels, session, specs, stage};

    #[rstest]
    fn evolution_before_the_gauge_is_full_is_rejected(
        mut session: GameSession,
        specs: SpecCatalog,
    ) {
        assert_eq!(
            evolve(&mut session, &specs, Evolution::LevelUp(MedalId::normal())),
            Err(GameSessionError::Evolution(EvolutionError::NotReady))
        );
    }

    #[rstest]
    fn level_up_returns_the_new_spec_to_apply(mut session: GameSession, specs: SpecCatalog) {
        session.medals_fell(1, &specs);
        let out =
            evolve(&mut session, &specs, Evolution::LevelUp(MedalId::normal())).expect("evolved");
        let level = MedalLevel::new(2).expect("valid");
        assert_eq!(out.event, DomainEvent::Evolved { medal: MedalId::normal(), level });
        assert_eq!(out.medal, Some((MedalId::normal(), levels()[1])));
    }

    #[rstest]
    fn evolution_after_game_over_is_rejected(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        session.medals_fell(1, &specs);
        session.advance(&stage, Tick(0));
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        assert_eq!(
            evolve(&mut session, &specs, Evolution::LevelUp(MedalId::normal())),
            Err(GameSessionError::NotPlaying(session.status()))
        );
    }
}
