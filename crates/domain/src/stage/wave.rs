use crate::{
    id::{BridgeId, EnemyTypeId, GoalId},
    time::Ticks,
};

/// 敵の出現エントリ。`at` はWave開始からの経過Tick。
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnEntry {
    pub at: Ticks,
    pub enemy: EnemyTypeId,
    pub bridge: BridgeId,
    pub goal: GoalId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wave(Vec<SpawnEntry>);

impl Wave {
    /// 出現時刻順に並べ替えて保持する。
    pub fn new(mut spawns: Vec<SpawnEntry>) -> Self {
        spawns.sort_by_key(|s| s.at);
        Self(spawns)
    }

    pub fn spawns(&self) -> &[SpawnEntry] {
        &self.0
    }
}

/// 敵の出現スケジュール。【仮決定】Stageに含める（基本設計 3.3）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WavePlan(Vec<Wave>);

impl WavePlan {
    pub fn new(waves: Vec<Wave>) -> Self {
        Self(waves)
    }

    pub fn waves(&self) -> &[Wave] {
        &self.0
    }
}
