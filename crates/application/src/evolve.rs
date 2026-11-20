//! メダル進化（基本設計 3.12）。進化画面で選んだレベルアップ・錬成・New! を実行する。

use domain::{
    event::DomainEvent,
    evolution::Evolution,
    id::MedalCode,
    launch::CatapultState,
    session::{GameSession, GameSessionError},
    spec::{MedalSpec, SpecCatalog},
};

/// ECSへ1回だけ反映すべき結果: 進化したメダル種と、その新しい物理特性。
#[derive(Debug, Clone, PartialEq)]
pub struct EvolveOutput {
    pub event: DomainEvent,
    pub medal: Option<(MedalCode, MedalSpec)>,
}

/// 錬成で選択中のメダルを素材として消費したら、選択をノーマルへ戻す（所持しないメダルを撃たない）。
pub fn evolve(
    session: &mut GameSession,
    catapult: &mut CatapultState,
    specs: &SpecCatalog,
    evolution: Evolution,
) -> Result<EvolveOutput, GameSessionError> {
    let event = session.evolve(evolution, specs)?;
    if session.medals().level(catapult.selected()).is_none() {
        catapult.select(MedalCode::normal());
    }
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
    use crate::test_support::{fire, levels, sauna, session, specs, stage, water};

    /// ゲージを満たしてから進化する（test_support: Easy は1枚で満タン）。
    fn evolve_now(
        session: &mut GameSession,
        catapult: &mut CatapultState,
        specs: &SpecCatalog,
        evolution: Evolution,
    ) -> Result<EvolveOutput, GameSessionError> {
        session.medals_fell(1.0, specs);
        evolve(session, catapult, specs, evolution)
    }

    #[rstest]
    fn evolution_before_the_gauge_is_full_is_rejected(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        assert_eq!(
            evolve(
                &mut session,
                &mut catapult,
                &specs,
                Evolution::LevelUp(MedalCode::normal())
            ),
            Err(GameSessionError::Evolution(EvolutionError::NotReady))
        );
    }

    #[rstest]
    fn level_up_returns_the_new_spec_and_keeps_the_selection(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        let out = evolve_now(
            &mut session,
            &mut catapult,
            &specs,
            Evolution::LevelUp(MedalCode::normal()),
        )
        .expect("evolved");
        let level = MedalLevel::new(2).expect("valid");
        assert_eq!(out.event, DomainEvent::Evolved { medal: MedalCode::normal(), level });
        assert_eq!(out.medal, Some((MedalCode::normal(), levels()[1])));
        assert_eq!(catapult.selected(), &MedalCode::normal());
    }

    /// fire・water を取得して Lv.4 にし、fire を選んだ状態で錬成する。
    #[rstest]
    fn fusion_consuming_the_selected_medal_selects_normal(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        for _ in 0..2 {
            session.medals_fell(1.0, &specs);
            let offer = session.evolution_offer().cloned().expect("offered");
            evolve(&mut session, &mut catapult, &specs, Evolution::Acquire(offer))
                .expect("acquired");
        }
        for medal in [fire(), water()] {
            for _ in 0..3 {
                evolve_now(&mut session, &mut catapult, &specs, Evolution::LevelUp(medal.clone()))
                    .expect("level up");
            }
        }
        catapult.select(fire());

        let out = evolve_now(&mut session, &mut catapult, &specs, Evolution::Fuse(fire(), water()))
            .expect("fused");

        assert_eq!(out.medal.map(|(m, _)| m), Some(sauna()));
        assert_eq!(catapult.selected(), &MedalCode::normal());
    }

    #[rstest]
    fn evolution_that_keeps_the_selected_medal_keeps_the_selection(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        session.medals_fell(1.0, &specs);
        let offer = session.evolution_offer().cloned().expect("offered");
        evolve(&mut session, &mut catapult, &specs, Evolution::Acquire(offer.clone()))
            .expect("acquired");
        catapult.select(offer.clone());

        evolve_now(&mut session, &mut catapult, &specs, Evolution::LevelUp(offer.clone()))
            .expect("level up");

        assert_eq!(catapult.selected(), &offer);
    }

    #[rstest]
    fn rejected_evolution_keeps_the_selection(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        catapult.select(fire());
        assert!(
            evolve_now(&mut session, &mut catapult, &specs, Evolution::LevelUp(fire())).is_err()
        );
        assert_eq!(catapult.selected(), &fire());
    }

    #[rstest]
    fn evolution_after_game_over_is_rejected(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
    ) {
        let mut catapult = CatapultState::new(stage.catapult());
        session.advance(&stage, Tick(0));
        session.medals_fell(1.0, &specs);
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        assert_eq!(
            evolve(
                &mut session,
                &mut catapult,
                &specs,
                Evolution::LevelUp(MedalCode::normal())
            ),
            Err(GameSessionError::NotPlaying(session.status()))
        );
    }
}
