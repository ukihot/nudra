//! メダル個体（盤面の1枚）の状態（基本設計 3.16）。
//!
//! 個体は ECS が正本（位置・速度と同じ）。この値は ECS の Component が持ち、
//! 個体についてのルール（封印・能力の回数・価値・変化）だけをここに閉じる。個体の ID は持たない。

use std::num::NonZeroU8;

use crate::{
    evolution::MedalLevel,
    id::MedalCode,
    mark::{Conditions, Modifiers},
    spec::abilities::AbilityBook,
};

/// 個体が通算で復活できる回数【仮決定】（試作では1回。能力の `uses`
/// とは別に、再生・変化でも共通の上限）。
pub const MAX_REVIVES: u8 = 1;

/// 生成できる最大の世代【仮決定】（試作では1。世代1の個体は生成・分割できない。基本設計 3.18）。
pub const MAX_GENERATION: u8 = 1;

/// メダルが盤面から退場した原因（基本設計 3.18）。通常の進化報酬は奈落への落下だけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MedalExitReason {
    FellIntoAbyss,
    /// 能力で消された（`Remove`・期限切れで消える印）
    DestroyedByAbility,
    /// 能力のコストとして消費された
    Consumed,
    /// 別の種に置き換えられた
    Transformed,
}

/// 個体の来歴。印の価値倍率とは別に持つ（価値を上げる印だけでは報酬資格を得られない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lineage {
    /// 奈落へ落ちたとき通常の進化報酬を得られる（初期配置・射出はあり、能力生成・
    /// 分割は既定でなし）
    pub rewarded: bool,
    /// 生成世代（初期配置・射出は0、能力生成・分割は親+1）。変化・復活・再生で戻らない
    pub generation: u8,
    /// 通算の復活回数（戻さない）
    pub revives: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedalUnit {
    medal: MedalCode,
    level: MedalLevel,
    conditions: Conditions,
    /// 回数制限のある能力の使用回数（能力の番号 → 回数）
    used: Vec<(usize, u8)>,
    lineage: Lineage,
}

impl MedalUnit {
    /// 初期配置・プレイヤーの射出（報酬資格あり、世代0）。
    pub fn new(medal: MedalCode, level: MedalLevel) -> Self {
        Self {
            medal,
            level,
            conditions: Conditions::default(),
            used: Vec::new(),
            lineage: Lineage { rewarded: true, generation: 0, revives: 0 },
        }
    }

    /// 能力で生成した子（世代は親+1、使用回数0、印は無し）。報酬資格は生成定義で明示したときだけ。
    pub fn offspring(&self, medal: MedalCode, level: MedalLevel, rewarded: bool) -> Self {
        Self {
            lineage: Lineage {
                rewarded,
                generation: self.lineage.generation.saturating_add(1),
                revives: 0,
            },
            ..Self::new(medal, level)
        }
    }

    /// 分割した片割れ（親の使用回数・印・復活回数を引き継ぎ、世代は親+1、報酬資格なし）。
    pub fn split_off(&self) -> Self {
        let mut half = self.clone();
        half.lineage.rewarded = false;
        half.lineage.generation = self.lineage.generation.saturating_add(1);
        half
    }

    pub fn lineage(&self) -> Lineage {
        self.lineage
    }

    /// この個体が生成・分割できるか（世代の上限 [`MAX_GENERATION`]）。
    pub fn can_spawn(&self) -> bool {
        self.lineage.generation < MAX_GENERATION
    }

    /// まだ復活できるか（個体通算の上限 [`MAX_REVIVES`]）。
    pub fn can_revive(&self) -> bool {
        self.lineage.revives < MAX_REVIVES
    }

    /// 復活を1回使う。上限に達していれば `false` で、何も変えない。
    pub fn revive(&mut self) -> bool {
        let ok = self.can_revive();
        if ok {
            self.lineage.revives += 1;
        }
        ok
    }

    /// 退場で得る通常の進化報酬（落下時の価値）。報酬資格のある個体が奈落へ落ちたときだけ。
    pub fn exit_reward(&self, reason: MedalExitReason, book: &AbilityBook) -> f32 {
        match reason {
            MedalExitReason::FellIntoAbyss if self.lineage.rewarded => self.modifiers(book).value,
            _ => 0.0,
        }
    }

    pub fn medal(&self) -> &MedalCode {
        &self.medal
    }
    pub fn level(&self) -> MedalLevel {
        self.level
    }
    pub fn conditions(&self) -> &Conditions {
        &self.conditions
    }
    pub fn conditions_mut(&mut self) -> &mut Conditions {
        &mut self.conditions
    }

    /// 進化で同じ種のメダルが成長した。覚醒の能力は末尾に加わるため、使用回数はそのまま引き継ぐ。
    pub fn set_level(&mut self, level: MedalLevel) {
        self.level = level;
    }

    /// 別の種に変わった（変化の効果）。印は残し、能力の使用回数は新しい種のものとして数え直す。
    pub fn transform(&mut self, medal: MedalCode, level: MedalLevel) {
        self.medal = medal;
        self.level = level;
        self.used.clear();
    }

    /// 印の効き目（定義に無い印は無視）。
    pub fn modifiers(&self, book: &AbilityBook) -> Modifiers {
        self.conditions.modifiers(|m| book.mark(m))
    }

    /// 能力を使えるか。`uses` が `None` なら無制限。
    pub fn can_use(&self, index: usize, uses: Option<NonZeroU8>) -> bool {
        uses.is_none_or(|limit| self.count(index) < limit.get())
    }

    /// 能力を1回使ったことを記録する（無制限の能力は記録しない）。
    pub fn use_once(&mut self, index: usize, uses: Option<NonZeroU8>) {
        let Some(limit) = uses else {
            return;
        };
        match self.used.iter_mut().find(|(i, _)| *i == index) {
            Some((_, n)) => *n = n.saturating_add(1).min(limit.get()),
            None => self.used.push((index, 1)),
        }
    }

    fn count(&self, index: usize) -> u8 {
        self.used
            .iter()
            .find_map(|&(i, n)| (i == index).then_some(n))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    fn unit() -> MedalUnit {
        MedalUnit::new(MedalCode::new("fire"), lv(1))
    }

    fn shine() -> crate::mark::MarkDef {
        crate::mark::MarkDef::new(
            crate::id::MarkId::new("shine"),
            false,
            NonZeroU8::MIN,
            false,
            Modifiers { value: 2.0, ..Modifiers::NONE },
        )
        .expect("valid")
    }

    fn book() -> AbilityBook {
        use crate::spec::abilities::AbilityParts;
        AbilityBook::new(
            AbilityParts { marks: vec![shine()], ..AbilityParts::default() },
            |_| true,
        )
        .expect("valid")
    }

    #[rstest]
    fn placed_and_launched_medals_are_rewarded_and_first_generation() {
        assert_eq!(
            unit().lineage(),
            Lineage { rewarded: true, generation: 0, revives: 0 }
        );
    }

    fn used_wet_revived() -> MedalUnit {
        let mut parent = unit();
        parent.use_once(0, NonZeroU8::new(1));
        parent
            .conditions_mut()
            .apply(&crate::mark::fixture::wet(), None);
        assert!(parent.revive());
        parent
    }

    #[rstest]
    #[case::plain(false)]
    #[case::rewarded(true)]
    fn offspring_are_a_generation_later_and_start_fresh(#[case] rewarded: bool) {
        let child = used_wet_revived().offspring(MedalCode::new("water"), lv(2), rewarded);
        assert_eq!(child.lineage(), Lineage { rewarded, generation: 1, revives: 0 });
        assert_eq!((child.medal().as_str(), child.level()), ("water", lv(2)));
        assert!(child.conditions().is_empty());
        assert!(child.can_use(0, NonZeroU8::new(1)));
    }

    #[rstest]
    fn split_halves_inherit_counts_marks_and_revives_but_not_the_reward() {
        let parent = used_wet_revived();
        let half = parent.split_off();
        assert_eq!(half.lineage(), Lineage { rewarded: false, generation: 1, revives: 1 });
        assert_eq!((half.medal(), half.level()), (parent.medal(), parent.level()));
        assert!(!half.can_use(0, NonZeroU8::new(1)));
        assert_eq!(half.conditions(), parent.conditions());
    }

    #[rstest]
    fn only_the_first_generation_can_spawn() {
        let parent = unit();
        assert!(parent.can_spawn());
        let child = parent.offspring(MedalCode::new("water"), lv(1), false);
        assert!(!child.can_spawn(), "generation {MAX_GENERATION} cannot spawn");
        assert!(!parent.split_off().can_spawn());
    }
    #[rstest]
    fn generations_saturate_instead_of_overflowing() {
        let mut u = unit();
        for _ in 0..300 {
            u = u.offspring(MedalCode::new("fire"), lv(1), false);
        }
        assert_eq!(u.lineage().generation, u8::MAX);
        assert_eq!(u.split_off().lineage().generation, u8::MAX);
    }

    #[rstest]
    fn revival_is_limited_per_unit_and_never_refunded() {
        let mut u = unit();
        assert!(u.can_revive());
        assert!(u.revive());
        assert_eq!(u.lineage().revives, MAX_REVIVES);
        assert!(!u.can_revive());
        let before = u.clone();
        assert!(!u.revive());
        assert_eq!(u, before, "a refused revival changes nothing");
    }

    #[rstest]
    fn transforming_keeps_the_lineage() {
        let mut u = used_wet_revived().offspring(MedalCode::new("water"), lv(1), false);
        let lineage = u.lineage();
        u.transform(MedalCode::new("sand"), lv(3));
        assert_eq!(u.lineage(), lineage);
    }

    #[rstest]
    #[case::fell(MedalExitReason::FellIntoAbyss, 2.0)]
    #[case::destroyed(MedalExitReason::DestroyedByAbility, 0.0)]
    #[case::consumed(MedalExitReason::Consumed, 0.0)]
    #[case::transformed(MedalExitReason::Transformed, 0.0)]
    fn only_falling_into_the_abyss_is_rewarded(
        #[case] reason: MedalExitReason,
        #[case] reward: f32,
    ) {
        let mut u = unit();
        u.conditions_mut().apply(&shine(), None);
        assert_eq!(u.exit_reward(reason, &book()), reward);
    }

    #[rstest]
    fn unrewarded_medals_earn_nothing_even_when_shiny() {
        let mut child = unit().offspring(MedalCode::normal(), lv(1), false);
        child.conditions_mut().apply(&shine(), None);
        assert_eq!(child.exit_reward(MedalExitReason::FellIntoAbyss, &book()), 0.0);
    }
    #[rstest]
    fn unlimited_abilities_are_always_usable() {
        let mut u = unit();
        for _ in 0..300 {
            assert!(u.can_use(0, None));
            u.use_once(0, None);
        }
        assert_eq!(u, unit(), "nothing is recorded");
    }

    #[rstest]
    fn limited_abilities_run_out_independently() {
        let mut u = unit();
        let twice = NonZeroU8::new(2);
        for _ in 0..2 {
            assert!(u.can_use(1, twice));
            u.use_once(1, twice);
        }
        assert!(!u.can_use(1, twice));
        assert!(u.can_use(2, twice), "another ability has its own count");
        u.use_once(1, twice);
        assert!(!u.can_use(1, twice), "using up stays used up");
    }

    #[rstest]
    fn growing_keeps_the_counts() {
        let mut u = unit();
        u.use_once(0, NonZeroU8::new(1));
        u.set_level(lv(4));
        assert_eq!(u.level(), lv(4));
        assert!(!u.can_use(0, NonZeroU8::new(1)));
    }

    #[rstest]
    fn transforming_changes_the_kind_and_resets_the_counts_but_keeps_marks() {
        let mut u = unit();
        u.use_once(0, NonZeroU8::new(1));
        u.conditions_mut()
            .apply(&crate::mark::fixture::wet(), None);
        u.transform(MedalCode::new("water"), lv(2));
        assert_eq!((u.medal().as_str(), u.level()), ("water", lv(2)));
        assert!(u.can_use(0, NonZeroU8::new(1)));
        assert!(!u.conditions().is_empty());
    }
}
