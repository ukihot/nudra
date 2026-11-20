use crate::{
    evolution::MedalLevel,
    id::{EnemyId, EnemyTypeId, GameSessionId, GoalId, MedalCode, PusherCharacterId, StageId},
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
        medal: MedalCode,
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
    /// Waveのボスを倒した（1始まりのWave番号）
    WaveCleared {
        wave: usize,
    },
    /// 最終Waveのボスを倒した
    GameWon,
    /// 進化ゲージが満タンになった（進化画面を出す）
    EvolutionReady,
    /// 進化した: そのメダル種がこのレベルになった（錬成なら新しいメダルの Lv.1）
    Evolved {
        medal: MedalCode,
        level: MedalLevel,
    },
}
