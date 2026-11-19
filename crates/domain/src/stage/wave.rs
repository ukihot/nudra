//! 敵の出現スケジュール（基本設計 3.12）。
//!
//! Wave は全4つ（確定）。第1〜3Wave は通常の出現と、終盤に出る中ボス1体を持つ。
//! 第4Wave は最終ボスだけを持つ。ボスはWaveの構造で指定する（どの敵種かは定義データ次第）。

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
pub struct Wave {
    spawns: Vec<SpawnEntry>,
    boss: SpawnEntry,
}

impl Wave {
    /// 通常の出現は時刻順に並べ替えて保持する。
    pub fn new(mut spawns: Vec<SpawnEntry>, boss: SpawnEntry) -> Self {
        spawns.sort_by_key(|s| s.at);
        Self { spawns, boss }
    }

    /// ボスだけのWave（第4Wave）。
    pub fn boss_only(boss: SpawnEntry) -> Self {
        Self::new(Vec::new(), boss)
    }

    /// 通常の出現（時刻順）。
    pub fn spawns(&self) -> &[SpawnEntry] {
        &self.spawns
    }

    /// このWaveのボス（第1〜3Waveは中ボス、第4Waveは最終ボス）。撃破で次のWaveへ進む。
    pub fn boss(&self) -> &SpawnEntry {
        &self.boss
    }

    /// ボスを含むすべての出現エントリ。
    pub fn entries(&self) -> impl Iterator<Item = &SpawnEntry> {
        self.spawns.iter().chain(std::iter::once(&self.boss))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WavePlan(Vec<Wave>);

impl WavePlan {
    /// Waveの数（確定）
    pub const WAVES: usize = 4;

    pub fn new(waves: Vec<Wave>) -> Self {
        Self(waves)
    }

    pub fn waves(&self) -> &[Wave] {
        &self.0
    }
}
