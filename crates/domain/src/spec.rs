//! 物理特性・進化ルールのゲーム上の定義（基本設計 3.6, 3.12, 3.13）。

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    num::NonZeroU32,
};

pub mod abilities;

use self::abilities::{AbilityBook, AbilityBookError, AbilityParts};
use crate::{
    difficulty::Difficulty,
    evolution::MedalLevel,
    id::{EnemyTypeId, MedalCode},
    positive::Positive,
    stage::{Stage, Wave},
};

/// メダルの階層（基本設計 3.13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MedalTier {
    /// 開始時に所持しているノーマルメダル
    Normal,
    /// 素材16種。進化画面の New! で手に入り、錬成の素材になる
    Element,
    /// 錬成72種。素材2種の錬成でだけ手に入る
    Fusion,
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

/// メダル種のレベル1〜4それぞれの物理特性（配列で4段階すべてが揃うことを型で保証する）。
pub type MedalLevels = [MedalSpec; MedalLevel::MAX.get() as usize];

/// メダル種の定義。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MedalDef {
    pub tier: MedalTier,
    pub levels: MedalLevels,
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

/// 進化ゲージのルール。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvolutionRules {
    /// 満タンになるポイント
    pub threshold: NonZeroU32,
    /// 難易度ごと（Easy → 最難）の、メダル1枚の落下あたりのポイント
    pub points_per_medal: [u32; Difficulty::LEVELS as usize],
}

impl EvolutionRules {
    pub fn points_per_medal(&self, difficulty: Difficulty) -> u32 {
        self.points_per_medal[usize::from(difficulty.level() - 1)]
    }
}

/// `SpecCatalog::new` に渡す組み立て用の値。
#[derive(Debug, Clone)]
pub struct SpecParts {
    pub medals: BTreeMap<MedalCode, MedalDef>,
    pub enemies: HashMap<EnemyTypeId, EnemySpec>,
    /// 錬成レシピ `(A, B, 結果)`。AとBの順序は問わない
    pub fusions: Vec<(MedalCode, MedalCode, MedalCode)>,
    pub evolution: EvolutionRules,
    /// 能力・印・反応（基本設計 3.14）
    pub abilities: AbilityParts,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SpecError {
    #[error(transparent)]
    Ability(#[from] AbilityBookError),
    #[error("spec for {0} medal is missing")]
    MissingMedal(MedalCode),
    #[error("a medal cannot be fused with itself ({0})")]
    SelfFusion(MedalCode),
    #[error("fusion recipe for {0} + {1} is defined twice")]
    DuplicateRecipe(MedalCode, MedalCode),
    #[error("fusion result {0} is used by more than one recipe")]
    DuplicateResult(MedalCode),
    #[error("{0} is not an element medal and cannot be a fusion material")]
    NotAnElement(MedalCode),
    #[error("{0} is not a fusion medal and cannot be a fusion result")]
    NotAFusion(MedalCode),
    #[error("the normal medal must be in the normal tier")]
    NormalTier,
    #[error("stage refers to unknown enemy type {0}")]
    UnknownEnemyType(EnemyTypeId),
}

#[derive(Debug, Clone)]
pub struct SpecCatalog {
    /// ID順（New! の候補を決定的に選ぶため、並びを一定にする）
    medals: BTreeMap<MedalCode, MedalDef>,
    enemies: HashMap<EnemyTypeId, EnemySpec>,
    /// 正規化した `(小, 大)` → 結果
    fusions: BTreeMap<(MedalCode, MedalCode), MedalCode>,
    evolution: EvolutionRules,
    abilities: AbilityBook,
}

impl SpecCatalog {
    pub fn new(parts: SpecParts) -> Result<Self, SpecError> {
        let SpecParts { medals, enemies, fusions, evolution, abilities } = parts;
        match medals.get(&MedalCode::normal()).map(|def| def.tier) {
            None => return Err(SpecError::MissingMedal(MedalCode::normal())),
            Some(MedalTier::Normal) => {},
            Some(_) => return Err(SpecError::NormalTier),
        }
        let tier = |m: &MedalCode| medals.get(m).map(|def| def.tier);
        let mut recipes = BTreeMap::new();
        let mut results = HashSet::new();
        for (a, b, result) in fusions {
            if a == b {
                return Err(SpecError::SelfFusion(a));
            }
            if let Some(missing) = [&a, &b, &result]
                .into_iter()
                .find(|m| !medals.contains_key(*m))
            {
                return Err(SpecError::MissingMedal(missing.clone()));
            }
            if let Some(material) = [&a, &b]
                .into_iter()
                .find(|m| tier(m) != Some(MedalTier::Element))
            {
                return Err(SpecError::NotAnElement(material.clone()));
            }
            if tier(&result) != Some(MedalTier::Fusion) {
                return Err(SpecError::NotAFusion(result));
            }
            if recipes.contains_key(&pair(&a, &b)) {
                return Err(SpecError::DuplicateRecipe(a, b));
            }
            if !results.insert(result.clone()) {
                return Err(SpecError::DuplicateResult(result));
            }
            recipes.insert(pair(&a, &b), result);
        }
        let abilities = AbilityBook::new(abilities, |m| medals.contains_key(m))?;
        Ok(Self {
            medals,
            enemies,
            fusions: recipes,
            evolution,
            abilities,
        })
    }

    /// そのレベルでのメダルの物理特性。
    pub fn medal(&self, medal: &MedalCode, level: MedalLevel) -> Option<&MedalSpec> {
        self.medals
            .get(medal)
            .map(|def| &def.levels[usize::from(level.get() - 1)])
    }

    pub fn tier(&self, medal: &MedalCode) -> Option<MedalTier> {
        self.medals.get(medal).map(|def| def.tier)
    }

    /// 定義済みメダルのID（図鑑・翻訳カタログの整合確認用）。
    pub fn medals(&self) -> impl Iterator<Item = &MedalCode> {
        self.medals.keys()
    }

    /// 素材の階層のメダル（ID順）。New! の候補になる。
    pub fn elements(&self) -> impl Iterator<Item = &MedalCode> {
        self.medals
            .iter()
            .filter(|(_, def)| def.tier == MedalTier::Element)
            .map(|(id, _)| id)
    }

    pub fn enemy(&self, enemy: &EnemyTypeId) -> Option<&EnemySpec> {
        self.enemies.get(enemy)
    }

    /// A+B の錬成結果（順序は問わない）。
    pub fn fusion(&self, a: &MedalCode, b: &MedalCode) -> Option<MedalCode> {
        self.fusions.get(&pair(a, b)).cloned()
    }

    pub fn evolution(&self) -> &EvolutionRules {
        &self.evolution
    }

    /// メダルの能力・印・反応。
    pub fn abilities(&self) -> &AbilityBook {
        &self.abilities
    }

    /// 定義済みの錬成レシピ（素材ID順）。進化画面の選択肢の生成に使う。
    pub fn fusions(&self) -> impl Iterator<Item = (&MedalCode, &MedalCode, &MedalCode)> {
        self.fusions
            .iter()
            .map(|((a, b), result)| (a, b, result))
    }

    /// ステージのWavePlanが参照する敵種がすべて定義されているか。
    pub fn check_stage(&self, stage: &Stage) -> Result<(), SpecError> {
        match stage
            .wave_plan()
            .waves()
            .iter()
            .flat_map(Wave::entries)
            .find(|s| !self.enemies.contains_key(&s.enemy))
        {
            Some(s) => Err(SpecError::UnknownEnemyType(s.enemy.clone())),
            None => Ok(()),
        }
    }
}

/// 錬成レシピのキー（順序を問わないよう小さい方を先にする）。
fn pair(a: &MedalCode, b: &MedalCode) -> (MedalCode, MedalCode) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use rstest::fixture;

    use super::*;
    use crate::positive::fixture::p;

    /// Lv.1 のメダル（レベルが上がるごとに質量 +1）
    pub fn medal() -> MedalSpec {
        MedalSpec {
            mass: p(1.0),
            radius: p(0.3),
            thickness: p(0.08),
            friction: 0.5,
            restitution: 0.1,
        }
    }

    pub fn levels() -> MedalLevels {
        [1.0, 2.0, 3.0, 4.0].map(|mass| MedalSpec { mass: p(mass), ..medal() })
    }

    pub fn def(tier: MedalTier) -> MedalDef {
        MedalDef { tier, levels: levels() }
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

    pub fn fire() -> MedalCode {
        MedalCode::new("fire")
    }
    pub fn water() -> MedalCode {
        MedalCode::new("water")
    }
    pub fn sauna() -> MedalCode {
        MedalCode::new("sauna")
    }

    /// ノーマル、素材 fire・water、錬成 sauna（fire + water）。ゲージは10で満タン、難易度ごとに
    /// 4,3,2,1 ポイント。
    pub fn parts() -> SpecParts {
        SpecParts {
            medals: BTreeMap::from([
                (MedalCode::normal(), def(MedalTier::Normal)),
                (fire(), def(MedalTier::Element)),
                (water(), def(MedalTier::Element)),
                (sauna(), def(MedalTier::Fusion)),
            ]),
            enemies: HashMap::from([(EnemyTypeId::new("grunt"), enemy())]),
            fusions: vec![(fire(), water(), sauna())],
            evolution: EvolutionRules {
                threshold: NonZeroU32::new(10).expect("non-zero"),
                points_per_medal: [4, 3, 2, 1],
            },
            abilities: AbilityParts::default(),
        }
    }

    #[fixture]
    pub fn catalog() -> SpecCatalog {
        SpecCatalog::new(parts()).expect("valid catalog")
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{
        fixture::{catalog, enemy, fire, levels, parts, sauna, water},
        *,
    };
    use crate::stage::fixture::stage;

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    #[rstest]
    fn medal_spec_depends_on_level(catalog: SpecCatalog) {
        assert_eq!(
            catalog
                .medal(&MedalCode::normal(), lv(1))
                .map(|s| s.mass.get()),
            Some(1.0)
        );
        assert_eq!(
            catalog
                .medal(&MedalCode::normal(), lv(4))
                .map(|s| s.mass.get()),
            Some(4.0)
        );
        assert_eq!(catalog.medal(&MedalCode::new("ice"), lv(1)), None);
        assert_eq!(catalog.tier(&fire()), Some(MedalTier::Element));
        assert_eq!(catalog.enemy(&EnemyTypeId::new("grunt")), Some(&enemy()));
    }

    #[rstest]
    fn invalid_abilities_reject_the_catalog() {
        let mut parts = parts();
        parts
            .abilities
            .medals
            .insert(MedalCode::new("ghost"), abilities::MedalAbilities::default());
        assert_eq!(
            SpecCatalog::new(parts).err(),
            Some(SpecError::Ability(AbilityBookError::UnknownMedal(MedalCode::new(
                "ghost"
            ))))
        );
    }

    #[rstest]
    fn elements_are_listed_in_id_order(catalog: SpecCatalog) {
        assert_eq!(catalog.elements().cloned().collect::<Vec<_>>(), [fire(), water()]);
    }

    #[rstest]
    fn fusion_ignores_order(catalog: SpecCatalog) {
        assert_eq!(catalog.fusion(&fire(), &water()), Some(sauna()));
        assert_eq!(catalog.fusion(&water(), &fire()), Some(sauna()));
        assert_eq!(catalog.fusion(&MedalCode::normal(), &fire()), None);
    }

    #[rstest]
    fn different_recipes_cannot_share_a_result() {
        let mut parts = parts();
        let sand = MedalCode::new("sand");
        parts
            .medals
            .insert(sand.clone(), fixture::def(MedalTier::Element));
        parts.fusions.push((fire(), sand, sauna()));
        assert!(SpecCatalog::new(parts).is_err(), "fusion results must be unique");
    }

    #[rstest]
    #[case::easy(Difficulty::EASY, 4)]
    #[case::hardest(Difficulty::HARDEST, 1)]
    fn gauge_points_depend_on_difficulty(
        catalog: SpecCatalog,
        #[case] d: Difficulty,
        #[case] points: u32,
    ) {
        assert_eq!(catalog.evolution().points_per_medal(d), points);
    }

    #[rstest]
    #[case::no_normal(
        |p: &mut SpecParts| { p.medals.remove(&MedalCode::normal()); },
        SpecError::MissingMedal(MedalCode::normal())
    )]
    #[case::fusion_result_undefined(
        |p: &mut SpecParts| { p.medals.remove(&sauna()); },
        SpecError::MissingMedal(sauna())
    )]
    #[case::fusion_material_undefined(
        |p: &mut SpecParts| { p.medals.remove(&water()); },
        SpecError::MissingMedal(water())
    )]
    #[case::self_fusion(
        |p: &mut SpecParts| p.fusions.push((fire(), fire(), sauna())),
        SpecError::SelfFusion(fire())
    )]
    #[case::material_not_element(
        |p: &mut SpecParts| p.fusions.push((MedalCode::normal(), fire(), sauna())),
        SpecError::NotAnElement(MedalCode::normal())
    )]
    #[case::result_not_fusion(
        |p: &mut SpecParts| { p.fusions[0].2 = MedalCode::normal(); },
        SpecError::NotAFusion(MedalCode::normal())
    )]
    #[case::normal_in_wrong_tier(
        |p: &mut SpecParts| { p.medals.insert(MedalCode::normal(), super::fixture::def(MedalTier::Element)); },
        SpecError::NormalTier
    )]
    #[case::duplicate_recipe(
        |p: &mut SpecParts| p.fusions.push((water(), fire(), sauna())),
        SpecError::DuplicateRecipe(water(), fire())
    )]
    fn invalid_specs_are_rejected(
        #[case] break_it: fn(&mut SpecParts),
        #[case] expected: SpecError,
    ) {
        let mut parts = parts();
        break_it(&mut parts);
        assert_eq!(SpecCatalog::new(parts).err(), Some(expected));
    }

    #[rstest]
    fn stage_with_known_enemies_passes(catalog: SpecCatalog, stage: Stage) {
        assert_eq!(catalog.check_stage(&stage), Ok(()));
    }

    #[rstest]
    fn stage_with_unknown_enemy_is_rejected(stage: Stage) {
        let catalog =
            SpecCatalog::new(SpecParts { enemies: HashMap::new(), ..parts() }).expect("valid");
        assert_eq!(
            catalog.check_stage(&stage),
            Err(SpecError::UnknownEnemyType(EnemyTypeId::new("grunt")))
        );
    }

    #[rstest]
    fn all_four_levels_are_kept() {
        assert_eq!(levels().len(), 4);
    }
}
