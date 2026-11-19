//! レベルアップ（要件16.4、基本設計 4.4）。【仮】成長条件・選択方式は未確定。

use domain::{
    event::DomainEvent,
    pusher::{PusherCharacter, PusherStats},
    session::{GameSession, GameSessionError, Upgrade},
    spec::{MedalSpec, MedalType, SpecCatalog},
};

/// ECSへ1回だけ反映すべき変更後の値。Domainで検証済みのため、そのまま物理エンジンへ渡せる。
#[derive(Debug, Clone, PartialEq)]
pub struct LevelUpOutput {
    pub event: DomainEvent,
    pub medal: Option<(MedalType, MedalSpec)>,
    pub pusher: Option<PusherStats>,
}

pub fn level_up(
    session: &mut GameSession,
    specs: &SpecCatalog,
    character: &PusherCharacter,
    upgrade: Upgrade,
) -> Result<LevelUpOutput, GameSessionError> {
    let event = session.apply_level_up(upgrade, specs, character)?;
    let (medal, pusher) = match upgrade {
        Upgrade::MedalMass { medal, .. } => (
            session
                .medal_spec(medal, specs)
                .map(|spec| (medal, spec)),
            None,
        ),
        Upgrade::PusherSpeed { .. } | Upgrade::PusherForce { .. } => {
            (None, Some(session.pusher_stats(character)))
        },
    };
    Ok(LevelUpOutput { event, medal, pusher })
}

#[cfg(test)]
mod tests {
    use domain::{
        id::{EnemyId, GoalId},
        session::UpgradeError,
        stage::Stage,
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{character, medal_spec, p, session, specs, stage};

    #[rstest]
    fn medal_upgrade_returns_new_medal_spec(
        mut session: GameSession,
        specs: SpecCatalog,
        character: PusherCharacter,
    ) {
        let upgrade = Upgrade::MedalMass { medal: MedalType::Normal, factor: p(2.0) };
        let out = level_up(&mut session, &specs, &character, upgrade).expect("playing");
        assert_eq!(out.event, DomainEvent::LevelUpCompleted { level: 2, upgrade });
        assert_eq!(
            out.medal,
            Some((MedalType::Normal, MedalSpec { mass: p(2.0), ..medal_spec() }))
        );
        assert_eq!(out.pusher, None);
    }

    #[rstest]
    #[case::speed(
        Upgrade::PusherSpeed { factor: p(2.0) },
        PusherStats { speed: p(4.0), ..crate::test_support::character().base_stats }
    )]
    #[case::force(
        Upgrade::PusherForce { factor: p(2.0) },
        PusherStats { push_force: p(200.0), ..crate::test_support::character().base_stats }
    )]
    fn pusher_upgrade_returns_new_stats(
        mut session: GameSession,
        specs: SpecCatalog,
        character: PusherCharacter,
        #[case] upgrade: Upgrade,
        #[case] expected: PusherStats,
    ) {
        let out = level_up(&mut session, &specs, &character, upgrade).expect("playing");
        assert_eq!((out.medal, out.pusher), (None, Some(expected)));
    }

    #[rstest]
    fn overflowing_upgrade_is_rejected(
        mut session: GameSession,
        specs: SpecCatalog,
        character: PusherCharacter,
    ) {
        let upgrade = Upgrade::PusherForce { factor: p(f32::MAX) };
        assert_eq!(
            level_up(&mut session, &specs, &character, upgrade),
            Err(GameSessionError::Upgrade(UpgradeError::Overflow(upgrade)))
        );
        assert_eq!(session.pusher_stats(&character), character.base_stats);
    }

    #[rstest]
    fn level_up_after_game_over_is_rejected(
        mut session: GameSession,
        stage: Stage,
        specs: SpecCatalog,
        character: PusherCharacter,
    ) {
        session.tick(&stage, std::time::Duration::ZERO);
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        assert_eq!(
            level_up(
                &mut session,
                &specs,
                &character,
                Upgrade::PusherSpeed { factor: p(2.0) }
            ),
            Err(GameSessionError::NotPlaying(session.status()))
        );
        assert_eq!(session.level().level(), 1);
    }
}
