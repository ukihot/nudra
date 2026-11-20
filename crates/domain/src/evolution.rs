//! メダル進化（基本設計 3.12）。
//!
//! メダル種ごとにレベル（Lv.1〜4）を持つ。進化は2種類:
//! - レベルアップ: 所持しているメダル種を Lv+1（Lv.4 が上限）
//! - 錬成: どちらも Lv.4 のメダルA・Bを**消費**し、レシピの結果のメダルCを Lv.1 で得る
//!
//! UIのグレーアウトとは別に、ここで必ず条件を検証する。失敗したときは状態を変えない（全か無か）。

use std::{collections::BTreeMap, num::NonZeroU32};

use crate::id::MedalId;

/// メダル種のレベル（1〜4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MedalLevel(u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("medal level must be 1..=4 (got {0})")]
pub struct InvalidMedalLevel(pub u8);

impl MedalLevel {
    pub const MIN: Self = Self(1);
    /// 上限（確定）
    pub const MAX: Self = Self(4);

    pub fn new(level: u8) -> Result<Self, InvalidMedalLevel> {
        if (Self::MIN.0..=Self::MAX.0).contains(&level) {
            Ok(Self(level))
        } else {
            Err(InvalidMedalLevel(level))
        }
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    /// 次のレベル。上限なら `None`。
    pub fn next(self) -> Option<Self> {
        (self < Self::MAX).then_some(Self(self.0 + 1))
    }
}

/// 進化ゲージ。落下したメダルの枚数 × 難易度の倍率で溜まり、満タンで進化できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvolutionGauge {
    points: u32,
    threshold: NonZeroU32,
}

impl EvolutionGauge {
    pub fn new(threshold: NonZeroU32) -> Self {
        Self { points: 0, threshold }
    }

    /// 加算する。満タンを超えた分は捨てる。この加算で満タンになったときだけ `true`。
    pub fn fill(&mut self, points: u32) -> bool {
        let was_full = self.is_full();
        self.points = self
            .points
            .saturating_add(points)
            .min(self.threshold.get());
        !was_full && self.is_full()
    }

    pub fn is_full(&self) -> bool {
        self.points >= self.threshold.get()
    }

    /// 溜まり具合（0.0〜1.0、HUD用）。
    pub fn ratio(&self) -> f32 {
        self.points as f32 / self.threshold.get() as f32
    }

    /// 進化したら空に戻す。
    pub fn reset(&mut self) {
        self.points = 0;
    }
}

/// 進化の選択肢。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evolution {
    LevelUp(MedalId),
    Fuse(MedalId, MedalId),
    /// 進化画面の New!: 提示された未所持のメダルを Lv.1 で得る
    Acquire(MedalId),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvolutionError {
    #[error("evolution gauge is not full yet")]
    NotReady,
    #[error("{0} medal is not owned")]
    NotOwned(MedalId),
    #[error("{0} medal is already at the max level")]
    AtMaxLevel(MedalId),
    #[error("{0} medal must be at the max level to fuse")]
    NotMaxLevel(MedalId),
    #[error("a medal cannot be fused with itself")]
    SameMedal,
    #[error("no fusion recipe for {0} + {1}")]
    NoRecipe(MedalId, MedalId),
    #[error("fusion result {0} is already owned")]
    ResultAlreadyOwned(MedalId),
    #[error("{0} is not the offered new medal")]
    NotOffered(MedalId),
    #[error("{0} medal is already owned")]
    AlreadyOwned(MedalId),
}

/// 進化画面の選択肢。`available` が偽なら表示だけする（錬成素材のレベル不足・結果を所持済み）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvolutionChoice {
    pub evolution: Evolution,
    /// 選んだ結果のメダル種（レベルアップは同じ種、錬成は結果の種、New! は獲得する種）
    pub result: MedalId,
    pub available: bool,
}

/// 進化の結果: どのメダル種が何レベルになったか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evolved {
    pub medal: MedalId,
    pub level: MedalLevel,
}

/// セッション中に所持しているメダル種とレベル。開始時はノーマル Lv.1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedalCollection(BTreeMap<MedalId, MedalLevel>);

impl Default for MedalCollection {
    fn default() -> Self {
        Self(BTreeMap::from([(MedalId::normal(), MedalLevel::MIN)]))
    }
}

impl MedalCollection {
    pub fn level(&self, medal: &MedalId) -> Option<MedalLevel> {
        self.0.get(medal).copied()
    }

    /// 所持しているメダル種（ID順）。
    pub fn owned(&self) -> impl Iterator<Item = (&MedalId, MedalLevel)> {
        self.0.iter().map(|(m, l)| (m, *l))
    }

    /// 進化を適用する。`recipe` は錬成レシピの検索（定義データ）、`offered` は New!
    /// で提示中のメダル。 失敗時は状態を変えない。
    pub fn evolve(
        &mut self,
        evolution: Evolution,
        recipe: impl Fn(&MedalId, &MedalId) -> Option<MedalId>,
        offered: Option<&MedalId>,
    ) -> Result<Evolved, EvolutionError> {
        // 検証がすべて済んでから変更する（全か無か）
        let evolved = self.check(&evolution, recipe, offered)?;
        if let Evolution::Fuse(a, b) = &evolution {
            self.0.remove(a);
            self.0.remove(b);
        }
        self.0.insert(evolved.medal.clone(), evolved.level);
        Ok(evolved)
    }

    /// 状態を変えずに、進化の可否と結果を調べる。`evolve` と進化画面の選択肢の両方がこれを使う。
    pub fn check(
        &self,
        evolution: &Evolution,
        recipe: impl Fn(&MedalId, &MedalId) -> Option<MedalId>,
        offered: Option<&MedalId>,
    ) -> Result<Evolved, EvolutionError> {
        match evolution {
            Evolution::Acquire(medal) => {
                if self.0.contains_key(medal) {
                    return Err(EvolutionError::AlreadyOwned(medal.clone()));
                }
                if offered != Some(medal) {
                    return Err(EvolutionError::NotOffered(medal.clone()));
                }
                Ok(Evolved { medal: medal.clone(), level: MedalLevel::MIN })
            },
            Evolution::LevelUp(medal) => {
                let level = self
                    .level(medal)
                    .ok_or_else(|| EvolutionError::NotOwned(medal.clone()))?;
                let next = level
                    .next()
                    .ok_or_else(|| EvolutionError::AtMaxLevel(medal.clone()))?;
                Ok(Evolved { medal: medal.clone(), level: next })
            },
            Evolution::Fuse(a, b) => {
                if a == b {
                    return Err(EvolutionError::SameMedal);
                }
                for medal in [a, b] {
                    match self.level(medal) {
                        None => return Err(EvolutionError::NotOwned(medal.clone())),
                        Some(level) if level != MedalLevel::MAX => {
                            return Err(EvolutionError::NotMaxLevel(medal.clone()));
                        },
                        Some(_) => {},
                    }
                }
                let result =
                    recipe(a, b).ok_or_else(|| EvolutionError::NoRecipe(a.clone(), b.clone()))?;
                if self.0.contains_key(&result) {
                    return Err(EvolutionError::ResultAlreadyOwned(result));
                }
                Ok(Evolved { medal: result, level: MedalLevel::MIN })
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;

    fn m(id: &str) -> MedalId {
        MedalId::new(id)
    }

    /// normal + fire → heavy（順序は問わない）
    fn recipes(a: &MedalId, b: &MedalId) -> Option<MedalId> {
        let mut pair = [a.as_str(), b.as_str()];
        pair.sort_unstable();
        (pair == ["fire", "normal"]).then(|| m("heavy"))
    }

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    fn collection(medals: &[(&str, u8)]) -> MedalCollection {
        MedalCollection(medals.iter().map(|&(id, l)| (m(id), lv(l))).collect())
    }

    #[fixture]
    fn both_max() -> MedalCollection {
        collection(&[("normal", 4), ("fire", 4)])
    }

    #[rstest]
    #[case::zero(0, false)]
    #[case::one(1, true)]
    #[case::four(4, true)]
    #[case::five(5, false)]
    fn levels_are_one_to_four(#[case] level: u8, #[case] valid: bool) {
        assert_eq!(MedalLevel::new(level).is_ok(), valid);
    }

    #[rstest]
    fn next_level_stops_at_max() {
        assert_eq!(lv(3).next(), Some(lv(4)));
        assert_eq!(MedalLevel::MAX.next(), None);
    }

    #[rstest]
    fn collection_starts_with_normal_level_one() {
        let start = MedalCollection::default();
        assert_eq!(
            start.owned().collect::<Vec<_>>(),
            [(&MedalId::normal(), MedalLevel::MIN)]
        );
    }

    #[rstest]
    fn level_up_raises_one_level() {
        let mut medals = MedalCollection::default();
        let evolved = medals.evolve(Evolution::LevelUp(MedalId::normal()), recipes, None);
        assert_eq!(evolved, Ok(Evolved { medal: MedalId::normal(), level: lv(2) }));
        assert_eq!(medals.level(&MedalId::normal()), Some(lv(2)));
    }

    #[rstest]
    #[case::at_max(&[("normal", 4)], "normal", EvolutionError::AtMaxLevel(m("normal")))]
    #[case::not_owned(&[("normal", 1)], "fire", EvolutionError::NotOwned(m("fire")))]
    fn invalid_level_up_is_rejected_without_change(
        #[case] owned: &[(&str, u8)],
        #[case] medal: &str,
        #[case] expected: EvolutionError,
    ) {
        let mut medals = collection(owned);
        let before = medals.clone();
        assert_eq!(
            medals.evolve(Evolution::LevelUp(m(medal)), recipes, None),
            Err(expected)
        );
        assert_eq!(medals, before);
    }

    #[rstest]
    fn fusing_two_max_medals_consumes_them_and_yields_level_one(mut both_max: MedalCollection) {
        let evolved = both_max.evolve(Evolution::Fuse(m("fire"), m("normal")), recipes, None);
        assert_eq!(evolved, Ok(Evolved { medal: m("heavy"), level: MedalLevel::MIN }));
        assert_eq!(both_max.owned().collect::<Vec<_>>(), [(&m("heavy"), MedalLevel::MIN)]);
    }

    #[rstest]
    #[case::not_max(&[("normal", 4), ("fire", 3)], ("normal", "fire"), EvolutionError::NotMaxLevel(m("fire")))]
    #[case::not_owned(&[("normal", 4)], ("normal", "fire"), EvolutionError::NotOwned(m("fire")))]
    #[case::same(&[("normal", 4)], ("normal", "normal"), EvolutionError::SameMedal)]
    #[case::no_recipe(&[("normal", 4), ("ice", 4)], ("normal", "ice"), EvolutionError::NoRecipe(m("normal"), m("ice")))]
    #[case::result_owned(&[("normal", 4), ("fire", 4), ("heavy", 2)], ("normal", "fire"), EvolutionError::ResultAlreadyOwned(m("heavy")))]
    fn invalid_fusion_is_rejected_without_change(
        #[case] owned: &[(&str, u8)],
        #[case] pair: (&str, &str),
        #[case] expected: EvolutionError,
    ) {
        let mut medals = collection(owned);
        let before = medals.clone();
        assert_eq!(
            medals.evolve(Evolution::Fuse(m(pair.0), m(pair.1)), recipes, None),
            Err(expected)
        );
        assert_eq!(medals, before);
    }

    #[rstest]
    fn acquiring_the_offered_medal_adds_it_at_level_one() {
        let mut medals = MedalCollection::default();
        let evolved = medals.evolve(Evolution::Acquire(m("fire")), recipes, Some(&m("fire")));
        assert_eq!(evolved, Ok(Evolved { medal: m("fire"), level: MedalLevel::MIN }));
        assert_eq!(medals.level(&m("fire")), Some(MedalLevel::MIN));
    }

    #[rstest]
    #[case::nothing_offered("fire", None, EvolutionError::NotOffered(m("fire")))]
    #[case::other_offered("fire", Some("water"), EvolutionError::NotOffered(m("fire")))]
    #[case::already_owned("normal", Some("normal"), EvolutionError::AlreadyOwned(m("normal")))]
    fn invalid_acquisition_is_rejected_without_change(
        #[case] medal: &str,
        #[case] offered: Option<&str>,
        #[case] expected: EvolutionError,
    ) {
        let mut medals = MedalCollection::default();
        let offered = offered.map(m);
        let result = medals.evolve(Evolution::Acquire(m(medal)), recipes, offered.as_ref());
        assert_eq!(result, Err(expected));
        assert_eq!(medals, MedalCollection::default());
    }

    fn gauge(threshold: u32) -> EvolutionGauge {
        EvolutionGauge::new(NonZeroU32::new(threshold).expect("non-zero"))
    }

    #[rstest]
    fn gauge_reports_full_only_on_the_fill_that_reaches_the_threshold() {
        let mut g = gauge(10);
        assert!(!g.fill(4));
        assert!(!g.is_full());
        assert!(g.fill(6), "reaches exactly 10");
        assert!(g.is_full());
        assert!(!g.fill(5), "already full: not reported again");
        assert_eq!(g.ratio(), 1.0, "excess is discarded");
    }

    #[rstest]
    #[case::empty(0, 0.0)]
    #[case::quarter(25, 0.25)]
    #[case::full(100, 1.0)]
    fn gauge_ratio(#[case] points: u32, #[case] ratio: f32) {
        let mut g = gauge(100);
        g.fill(points);
        assert_eq!(g.ratio(), ratio);
    }

    #[rstest]
    fn huge_fill_does_not_overflow_and_reset_empties() {
        let mut g = gauge(10);
        g.fill(7);
        assert!(g.fill(u32::MAX));
        g.reset();
        assert_eq!(g.ratio(), 0.0);
        assert!(!g.is_full());
    }
}
