//! Wave進行と敵の出現（要件16.3の物理前段、基本設計 4.4）。

use std::time::Duration;

use domain::{
    event::DomainEvent,
    session::{GameSession, SpawnOrder},
    stage::Stage,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaveOutput {
    pub spawns: Vec<SpawnOrder>,
    pub events: Vec<DomainEvent>,
}

pub fn advance_waves(session: &mut GameSession, stage: &Stage, dt: Duration) -> WaveOutput {
    let spawns = session.tick(stage, dt);
    let events = spawns
        .iter()
        .map(|s| DomainEvent::EnemySpawned { enemy: s.enemy, kind: s.kind.clone(), goal: s.goal })
        .collect();
    WaveOutput { spawns, events }
}

#[cfg(test)]
mod tests {
    use domain::id::{EnemyId, EnemyTypeId, GoalId};
    use rstest::rstest;

    use super::*;
    use crate::test_support::{session, stage};

    #[rstest]
    fn spawn_orders_come_with_spawned_events(mut session: GameSession, stage: Stage) {
        let out = advance_waves(&mut session, &stage, Duration::from_secs(1));
        assert_eq!(
            out.spawns.iter().map(|s| s.enemy).collect::<Vec<_>>(),
            [EnemyId(1), EnemyId(2)]
        );
        assert_eq!(
            out.events,
            [EnemyId(1), EnemyId(2)].map(|enemy| DomainEvent::EnemySpawned {
                enemy,
                kind: EnemyTypeId::new("grunt"),
                goal: GoalId(1),
            })
        );
    }

    #[rstest]
    fn nothing_happens_before_schedule_or_after_game_over(mut session: GameSession, stage: Stage) {
        let first = advance_waves(&mut session, &stage, Duration::ZERO);
        assert_eq!(first.spawns.len(), 1);
        assert_eq!(
            advance_waves(&mut session, &stage, Duration::from_millis(500)),
            WaveOutput::default()
        );

        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        assert_eq!(
            advance_waves(&mut session, &stage, Duration::from_secs(10)),
            WaveOutput::default()
        );
    }
}
