//! インゲーム成長（基本設計 3.10）。【骨格のみ】経験値・レベルアップ条件は未確定。
//! 強化は種別単位で適用する（メダル個体の状態をDomainに持たないため）。
//! 強化後の値はレベルアップ時にchecked演算で確定し、不正な物理パラメータを生成しない。

use std::collections::HashMap;

use crate::{
    positive::Positive,
    pusher::PusherStats,
    spec::{MedalSpec, MedalType, SpecCatalog},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Upgrade {
    MedalMass { medal: MedalType, factor: Positive },
    PusherSpeed { factor: Positive },
    PusherForce { factor: Positive },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UpgradeError {
    #[error("no spec for {0:?} medal")]
    MissingMedalSpec(MedalType),
    #[error("{0:?} would make a value non-finite")]
    Overflow(Upgrade),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelProgress {
    level: u32,
    /// 強化済みのメダル定義（強化されていない種別は基礎値を使う）
    medals: HashMap<MedalType, MedalSpec>,
    /// 強化済みのPusher能力値（未強化なら基礎値を使う）
    pusher: Option<PusherStats>,
}

impl LevelProgress {
    pub fn initial() -> Self {
        Self { level: 1, medals: HashMap::new(), pusher: None }
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    /// 強化を適用し、適用後のレベルを返す。失敗時は状態を変更しない。
    pub(super) fn apply(
        &mut self,
        upgrade: Upgrade,
        specs: &SpecCatalog,
        base_pusher: PusherStats,
    ) -> Result<u32, UpgradeError> {
        let scale = |value: Positive, factor: Positive| {
            value
                .checked_mul(factor)
                .ok_or(UpgradeError::Overflow(upgrade))
        };
        // 新しい値をコピー上で計算し、成功した場合だけ書き戻す（全か無か）。
        match upgrade {
            Upgrade::MedalMass { medal, factor } => {
                let mut spec = self
                    .medals
                    .get(&medal)
                    .or_else(|| specs.medal(medal))
                    .copied()
                    .ok_or(UpgradeError::MissingMedalSpec(medal))?;
                spec.mass = scale(spec.mass, factor)?;
                self.medals.insert(medal, spec);
            },
            Upgrade::PusherSpeed { factor } | Upgrade::PusherForce { factor } => {
                let mut stats = self.pusher.unwrap_or(base_pusher);
                let field = match upgrade {
                    Upgrade::PusherSpeed { .. } => &mut stats.speed,
                    _ => &mut stats.push_force,
                };
                *field = scale(*field, factor)?;
                self.pusher = Some(stats);
            },
        }
        self.level += 1;
        Ok(self.level)
    }

    pub fn medal_spec(&self, medal: MedalType, base: MedalSpec) -> MedalSpec {
        self.medals.get(&medal).copied().unwrap_or(base)
    }

    pub fn pusher_stats(&self, base: PusherStats) -> PusherStats {
        self.pusher.unwrap_or(base)
    }
}
