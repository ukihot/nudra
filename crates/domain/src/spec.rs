//! 物理特性のゲーム上の定義（基本設計 3.6）。

use std::collections::HashMap;

use crate::{
    id::EnemyTypeId,
    positive::Positive,
    stage::{Stage, Wave},
};

/// 特殊メダルの属性。効果の実装を伴うためenum。【未確定】初期実装はNormalのみ使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpecialKind {
    Fire,
    Ice,
    Explosion,
    Heavy,
    Magnet,
    Bounce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MedalType {
    Normal,
    Special(SpecialKind),
}

/// 寸法・質量は `Positive` で、不正な物理パラメータを生成できないようにする。
/// 摩擦・反発係数は0が有効値のため `f32`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MedalSpec {
    pub mass: Positive,
    pub radius: Positive,
    pub thickness: Positive,
    pub friction: f32,
    pub restitution: f32,
}

/// 敵の物理特性。【未確定】アビリティは未実装。
/// 速度・歩行力は0（動かない敵）が有効値のため `f32`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemySpec {
    pub mass: Positive,
    pub radius: Positive,
    pub height: Positive,
    pub max_speed: f32,
    pub walk_force: f32,
    pub friction: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SpecError {
    #[error("spec for {0:?} medal is missing")]
    MissingMedal(MedalType),
    #[error("stage refers to unknown enemy type {0}")]
    UnknownEnemyType(EnemyTypeId),
}

#[derive(Debug, Clone)]
pub struct SpecCatalog {
    medals: HashMap<MedalType, MedalSpec>,
    enemies: HashMap<EnemyTypeId, EnemySpec>,
}

impl SpecCatalog {
    pub fn new(
        medals: HashMap<MedalType, MedalSpec>,
        enemies: HashMap<EnemyTypeId, EnemySpec>,
    ) -> Result<Self, SpecError> {
        if !medals.contains_key(&MedalType::Normal) {
            return Err(SpecError::MissingMedal(MedalType::Normal));
        }
        Ok(Self { medals, enemies })
    }

    pub fn medal(&self, medal: MedalType) -> Option<&MedalSpec> {
        self.medals.get(&medal)
    }

    pub fn enemy(&self, enemy: &EnemyTypeId) -> Option<&EnemySpec> {
        self.enemies.get(enemy)
    }

    /// ステージのWavePlanが参照する敵種がすべて定義されているか。
    pub fn check_stage(&self, stage: &Stage) -> Result<(), SpecError> {
        match stage
            .wave_plan()
            .waves()
            .iter()
            .flat_map(Wave::spawns)
            .find(|s| !self.enemies.contains_key(&s.enemy))
        {
            Some(s) => Err(SpecError::UnknownEnemyType(s.enemy.clone())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use rstest::fixture;

    use super::*;
    use crate::positive::fixture::p;

    pub fn medal() -> MedalSpec {
        MedalSpec {
            mass: p(1.0),
            radius: p(0.3),
            thickness: p(0.08),
            friction: 0.5,
            restitution: 0.1,
        }
    }

    pub fn enemy() -> EnemySpec {
        EnemySpec {
            mass: p(2.0),
            radius: p(0.3),
            height: p(0.8),
            max_speed: 1.5,
            walk_force: 10.0,
            friction: 0.5,
        }
    }

    #[fixture]
    pub fn catalog() -> SpecCatalog {
        SpecCatalog::new(
            HashMap::from([(MedalType::Normal, medal())]),
            HashMap::from([(EnemyTypeId::new("grunt"), enemy())]),
        )
        .expect("valid catalog")
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{
        fixture::{catalog, enemy, medal},
        *,
    };
    use crate::stage::fixture::stage;

    #[rstest]
    fn valid_catalog_returns_specs(catalog: SpecCatalog) {
        assert_eq!(catalog.medal(MedalType::Normal), Some(&medal()));
        assert_eq!(catalog.medal(MedalType::Special(SpecialKind::Fire)), None);
        assert_eq!(catalog.enemy(&EnemyTypeId::new("grunt")), Some(&enemy()));
    }

    #[rstest]
    fn normal_medal_is_required() {
        assert_eq!(
            SpecCatalog::new(HashMap::new(), HashMap::new()).err(),
            Some(SpecError::MissingMedal(MedalType::Normal))
        );
    }

    #[rstest]
    fn stage_with_known_enemies_passes(catalog: SpecCatalog, stage: Stage) {
        assert_eq!(catalog.check_stage(&stage), Ok(()));
    }

    #[rstest]
    fn stage_with_unknown_enemy_is_rejected(stage: Stage) {
        let catalog =
            SpecCatalog::new(HashMap::from([(MedalType::Normal, medal())]), HashMap::new())
                .expect("valid");
        assert_eq!(
            catalog.check_stage(&stage),
            Err(SpecError::UnknownEnemyType(EnemyTypeId::new("grunt")))
        );
    }
}
