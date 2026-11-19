use crate::{
    id::{EnemyId, EnemyTypeId, GameSessionId, GoalId, PusherCharacterId, StageId},
    session::Upgrade,
    spec::MedalType,
};

/// ゲーム上の意味を持つ出来事。集約・ドメイン関数の戻り値として返す（基本設計 3.9）。
#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    GameStarted {
        session: GameSessionId,
        stage: StageId,
        character: PusherCharacterId,
    },
    MedalLaunched {
        medal: MedalType,
    },
    EnemySpawned {
        enemy: EnemyId,
        kind: EnemyTypeId,
        goal: GoalId,
    },
    EnemyDefeated {
        enemy: EnemyId,
    },
    EnemyReachedGoal {
        enemy: EnemyId,
        goal: GoalId,
    },
    GameLost {
        enemy: EnemyId,
        goal: GoalId,
    },
    LevelUpCompleted {
        level: u32,
        upgrade: Upgrade,
    },
}
