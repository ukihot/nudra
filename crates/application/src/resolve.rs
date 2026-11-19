//! 物理結果からゲーム上の意味への変換（要件16.3の物理後段、基本設計 4.3）。

use domain::{
    event::DomainEvent,
    id::{EnemyId, GoalId},
    session::GameSession,
    spec::SpecCatalog,
};

/// Infrastructureがセンサー接触から作る、ゲーム判定に必要な物理上の事実。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsFact {
    EnemyEnteredAbyss(EnemyId),
    EnemyEnteredGoal(EnemyId, GoalId),
    /// このtickに奈落へ落ちたメダルの枚数（大量の接触を1件に集計して渡す、基本設計 3.12）
    MedalsFell(u32),
}

/// `facts` を消費して判定する。同じtickでは落下（撃破）を到達より先に処理する（基本設計 3.4）。
pub fn resolve_physics(
    session: &mut GameSession,
    facts: &mut Vec<PhysicsFact>,
    specs: &SpecCatalog,
) -> Vec<DomainEvent> {
    // 安定ソート: 落下(0) → 到達(1)。同種内の到着順は保つ。
    facts.sort_by_key(|f| matches!(f, PhysicsFact::EnemyEnteredGoal(..)));
    let mut events = Vec::new();
    for fact in facts.drain(..) {
        match fact {
            PhysicsFact::EnemyEnteredAbyss(enemy) => events.extend(session.enemy_fell(enemy)),
            PhysicsFact::EnemyEnteredGoal(enemy, goal) => {
                events.extend(session.enemy_reached_goal(enemy, goal))
            },
            PhysicsFact::MedalsFell(count) => events.extend(session.medals_fell(count, specs)),
        }
    }
    events
}

#[cfg(test)]
mod tests {

    use PhysicsFact::{EnemyEnteredAbyss as Fell, EnemyEnteredGoal as Goal};
    use domain::{session::GameStatus, stage::Stage};
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;
    use crate::test_support::{character, session, specs, stage};

    fn with_two_enemies(mut session: GameSession, stage: &Stage) -> GameSession {
        session.advance(stage, domain::time::Tick(60));
        session
    }

    #[rstest]
    fn fall_defeats_enemy_and_consumes_facts(session: GameSession, stage: Stage) {
        let mut session = with_two_enemies(session, &stage);
        let mut facts = vec![Fell(EnemyId(1))];
        assert_eq!(
            resolve_physics(&mut session, &mut facts, &specs()),
            [DomainEvent::EnemyDefeated { enemy: EnemyId(1) }]
        );
        assert!(facts.is_empty());
    }

    #[rstest]
    fn goal_loses_the_game(session: GameSession, stage: Stage) {
        let mut session = with_two_enemies(session, &stage);
        let events =
            resolve_physics(&mut session, &mut vec![Goal(EnemyId(2), GoalId(1))], &specs());
        assert_eq!(
            events.last(),
            Some(&DomainEvent::GameLost { enemy: EnemyId(2), goal: GoalId(1) })
        );
        assert_eq!(
            session.status(),
            GameStatus::Lost { enemy: EnemyId(2), goal: GoalId(1) }
        );
    }

    #[rstest]
    fn fall_wins_over_goal_in_the_same_tick(session: GameSession, stage: Stage) {
        let mut session = with_two_enemies(session, &stage);
        let events = resolve_physics(
            &mut session,
            &mut vec![Goal(EnemyId(1), GoalId(1)), Fell(EnemyId(1))],
            &specs(),
        );
        assert_eq!(events, [DomainEvent::EnemyDefeated { enemy: EnemyId(1) }]);
        assert_eq!(session.status(), GameStatus::Playing);
    }

    #[rstest]
    fn no_facts_no_events(session: GameSession) {
        let mut session = session;
        assert!(resolve_physics(&mut session, &mut Vec::new(), &specs()).is_empty());
    }

    #[rstest]
    fn falling_medals_fill_the_evolution_gauge(session: GameSession) {
        let mut session = session;
        // Easy は1枚10ポイント、10で満タン（test_support）
        let events = resolve_physics(&mut session, &mut vec![PhysicsFact::MedalsFell(1)], &specs());
        assert_eq!(events, [DomainEvent::EvolutionReady]);
        assert!(session.gauge().is_full());
    }

    fn fact() -> impl Strategy<Value = PhysicsFact> {
        prop_oneof![
            (1u64..3).prop_map(|id| Fell(EnemyId(id))),
            (1u64..3).prop_map(|id| Goal(EnemyId(id), GoalId(1)))
        ]
    }

    proptest! {
        /// 同じtickで落下した敵は、到達がどの順で届いても敗北原因にならない。
        #[test]
        fn fallen_enemy_never_causes_loss(mut facts in prop::collection::vec(fact(), 0..10)) {
            let stage = stage();
            let mut session = with_two_enemies(session(stage.clone(), character(), specs()), &stage);
            let fallen: Vec<_> = facts.iter().filter_map(|f| match f { Fell(id) => Some(*id), _ => None }).collect();
            resolve_physics(&mut session, &mut facts, &specs());
            if let GameStatus::Lost { enemy, .. } = session.status() {
                prop_assert!(!fallen.contains(&enemy));
            }
        }
    }
}
