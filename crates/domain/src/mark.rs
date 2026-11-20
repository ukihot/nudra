//! 印（マーク）と、メダル・敵が今かかっている状態（基本設計 3.14）。
//!
//! - 印の種類と効き目は定義データ（濡れ・砂・毒…）。コードにあるのは効き目の語彙 [`Modifiers`] だけ
//! - 個体の状態 [`Conditions`] は値として ECS の Component が持つ（個体は ECS が正本）。
//!   状態の変化のルール（重ね掛け・期限・浄化・効き目の合成）はここに閉じる

use std::num::NonZeroU8;

use crate::{id::MarkId, positive::Positive, time::Tick};

/// 倍率の上限。重ね掛けしても、合成した倍率は `[1/MAX_FACTOR, MAX_FACTOR]`（速さ・価値は `[0,
/// MAX_FACTOR]`）に収める。
pub const MAX_FACTOR: f32 = 16.0;
/// 摩擦係数の上限。
pub const MAX_FRICTION: f32 = 2.0;

/// 印の効き目。重ね掛けは倍率が `stacks` 乗、進路のずれが `stacks` 倍で効く。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modifiers {
    /// 摩擦の上書き（濡れて滑る・砂で重い）。`None` は変えない。複数あれば後に付いた印が勝つ
    pub friction: Option<f32>,
    pub mass: Positive,
    pub size: Positive,
    /// 敵の最大速度の倍率（0 で足止め）
    pub speed: f32,
    /// 敵の進行方向のずれ（ラジアン）
    pub heading: f32,
    /// 落下したときの進化ゲージのポイントの倍率（0 で偽物）
    pub value: f32,
    /// 能力を封じる
    pub sealed: bool,
    /// メダルをすり抜ける
    pub ghost: bool,
}

impl Modifiers {
    /// 何も変えない効き目。
    pub const NONE: Self = Self {
        friction: None,
        mass: Positive::ONE,
        size: Positive::ONE,
        speed: 1.0,
        heading: 0.0,
        value: 1.0,
        sealed: false,
        ghost: false,
    };
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MarkError {
    #[error("friction of mark {0} must be within 0..={MAX_FRICTION}")]
    Friction(MarkId),
    #[error("factor of mark {0} must be within 0..={MAX_FACTOR}")]
    Factor(MarkId),
    #[error("heading of mark {0} must be within -PI..=PI")]
    Heading(MarkId),
}

/// 印の定義。生成時に効き目の値域を検証する。
#[derive(Debug, Clone, PartialEq)]
pub struct MarkDef {
    id: MarkId,
    harmful: bool,
    max_stacks: NonZeroU8,
    vanish: bool,
    modifiers: Modifiers,
}

impl MarkDef {
    /// - `harmful`: 浄化で消える悪い印
    /// - `vanish`: 期限が切れると持ち主が消える（焼き尽くす・毒）。消えたメダルは落下と同じに数える
    pub fn new(
        id: MarkId,
        harmful: bool,
        max_stacks: NonZeroU8,
        vanish: bool,
        modifiers: Modifiers,
    ) -> Result<Self, MarkError> {
        let m = modifiers;
        let factor = |v: f32| (0.0..=MAX_FACTOR).contains(&v);
        let scale = |v: Positive| (1.0 / MAX_FACTOR..=MAX_FACTOR).contains(&v.get());
        if m.friction
            .is_some_and(|f| !(0.0..=MAX_FRICTION).contains(&f))
        {
            return Err(MarkError::Friction(id));
        }
        if !(factor(m.speed) && factor(m.value) && scale(m.mass) && scale(m.size)) {
            return Err(MarkError::Factor(id));
        }
        if !(-std::f32::consts::PI..=std::f32::consts::PI).contains(&m.heading) {
            return Err(MarkError::Heading(id));
        }
        Ok(Self { id, harmful, max_stacks, vanish, modifiers })
    }

    pub fn id(&self) -> &MarkId {
        &self.id
    }
    pub fn harmful(&self) -> bool {
        self.harmful
    }
    pub fn max_stacks(&self) -> NonZeroU8 {
        self.max_stacks
    }
    pub fn vanish(&self) -> bool {
        self.vanish
    }
    pub fn modifiers(&self) -> &Modifiers {
        &self.modifiers
    }
}

/// かかっている印1つ。`until` が `None` なら永続（成長など）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMark {
    pub mark: MarkId,
    pub stacks: NonZeroU8,
    pub until: Option<Tick>,
}

/// 期限切れの結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Expiry {
    /// 印が1つ以上外れた（効き目を反映し直す）
    pub changed: bool,
    /// 外れた印に `vanish` があった（持ち主を消す）
    pub vanish: bool,
}

/// 個体（メダル1枚・敵1体）にかかっている印。付いた順に並ぶ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conditions(Vec<ActiveMark>);

impl Conditions {
    pub fn marks(&self) -> &[ActiveMark] {
        &self.0
    }

    pub fn has(&self, mark: &MarkId) -> bool {
        self.0.iter().any(|m| &m.mark == mark)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 印を付ける。すでにあれば重ね掛け（上限まで）して、期限は遅い方（永続が最強）にする。
    pub fn apply(&mut self, def: &MarkDef, until: Option<Tick>) {
        let Some(active) = self.0.iter_mut().find(|m| &m.mark == def.id()) else {
            self.0.push(ActiveMark {
                mark: def.id().clone(),
                stacks: NonZeroU8::MIN,
                until,
            });
            return;
        };
        active.stacks = active.stacks.saturating_add(1).min(def.max_stacks());
        // 永続（None）が最強。期限付きどうしは遅い方
        active.until = active.until.zip(until).map(|(a, b)| a.max(b));
    }

    /// 期限（`until <= now`）の来た印を外す。
    pub fn expire<'a>(
        &mut self,
        now: Tick,
        marks: impl Fn(&MarkId) -> Option<&'a MarkDef>,
    ) -> Expiry {
        let mut expiry = Expiry::default();
        self.0.retain(|m| {
            let due = m.until.is_some_and(|t| t <= now);
            if due {
                expiry.changed = true;
                expiry.vanish |= marks(&m.mark).is_some_and(MarkDef::vanish);
            }
            !due
        });
        expiry
    }

    /// 浄化。`harmful_only` なら悪い印だけ、そうでなければすべて外す。外したら `true`。
    pub fn cleanse<'a>(
        &mut self,
        harmful_only: bool,
        marks: impl Fn(&MarkId) -> Option<&'a MarkDef>,
    ) -> bool {
        let before = self.0.len();
        self.0
            .retain(|m| harmful_only && !marks(&m.mark).is_some_and(MarkDef::harmful));
        self.0.len() != before
    }

    /// 印を外す。外したら `true`。
    pub fn remove(&mut self, mark: &MarkId) -> bool {
        let before = self.0.len();
        self.0.retain(|m| &m.mark != mark);
        self.0.len() != before
    }

    /// かかっている印の効き目を合成する。未定義の印は無視する。結果は値域に収める。
    pub fn modifiers<'a>(&self, marks: impl Fn(&MarkId) -> Option<&'a MarkDef>) -> Modifiers {
        let mut out = Modifiers::NONE;
        let (mut mass, mut size) = (1.0f32, 1.0f32);
        for active in &self.0 {
            let Some(def) = marks(&active.mark) else {
                continue;
            };
            let m = def.modifiers();
            let n = i32::from(active.stacks.get());
            out.friction = m.friction.or(out.friction);
            mass *= m.mass.get().powi(n);
            size *= m.size.get().powi(n);
            out.speed *= m.speed.powi(n);
            out.value *= m.value.powi(n);
            out.heading += m.heading * f32::from(active.stacks.get());
            out.sealed |= m.sealed;
            out.ghost |= m.ghost;
        }
        let scale = |v: f32| {
            Positive::new(v.clamp(1.0 / MAX_FACTOR, MAX_FACTOR))
                .expect("a value clamped to [1/MAX_FACTOR, MAX_FACTOR] is positive")
        };
        out.mass = scale(mass);
        out.size = scale(size);
        out.speed = out.speed.clamp(0.0, MAX_FACTOR);
        out.value = out.value.clamp(0.0, MAX_FACTOR);
        out
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::positive::fixture::p;

    pub fn stacks(n: u8) -> NonZeroU8 {
        NonZeroU8::new(n).expect("non-zero")
    }

    /// 濡れ: 滑る（摩擦 0.05）。悪い印ではない
    pub fn wet() -> MarkDef {
        MarkDef::new(
            MarkId::new("wet"),
            false,
            stacks(1),
            false,
            Modifiers { friction: Some(0.05), ..Modifiers::NONE },
        )
        .expect("valid")
    }

    /// 毒: 重ね掛け3まで。遅く・軽くなり、切れると消える
    pub fn poison() -> MarkDef {
        MarkDef::new(
            MarkId::new("poison"),
            true,
            stacks(3),
            true,
            Modifiers { mass: p(0.5), speed: 0.5, ..Modifiers::NONE },
        )
        .expect("valid")
    }

    /// 育つ: 永続で重ね掛け5まで。大きく・重くなる
    pub fn grow() -> MarkDef {
        MarkDef::new(
            MarkId::new("grow"),
            false,
            stacks(5),
            false,
            Modifiers { mass: p(2.0), size: p(1.5), ..Modifiers::NONE },
        )
        .expect("valid")
    }

    /// 砂: 摩擦を上げる悪い印
    pub fn sand() -> MarkDef {
        MarkDef::new(
            MarkId::new("sand"),
            true,
            stacks(1),
            false,
            Modifiers { friction: Some(1.2), speed: 0.6, ..Modifiers::NONE },
        )
        .expect("valid")
    }

    pub fn book(id: &MarkId) -> Option<&'static MarkDef> {
        use std::sync::LazyLock;
        static BOOK: LazyLock<[MarkDef; 4]> = LazyLock::new(|| [wet(), poison(), grow(), sand()]);
        BOOK.iter().find(|m| m.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::{fixture::*, *};
    use crate::positive::fixture::p;

    fn def(modifiers: Modifiers) -> Result<MarkDef, MarkError> {
        MarkDef::new(MarkId::new("m"), false, stacks(1), false, modifiers)
    }

    fn m() -> MarkId {
        MarkId::new("m")
    }

    #[rstest]
    #[case::none(Modifiers::NONE, None)]
    #[case::friction_zero(Modifiers { friction: Some(0.0), ..Modifiers::NONE }, None)]
    #[case::friction_max(Modifiers { friction: Some(MAX_FRICTION), ..Modifiers::NONE }, None)]
    #[case::friction_negative(Modifiers { friction: Some(-0.1), ..Modifiers::NONE }, Some(MarkError::Friction(m())))]
    #[case::friction_too_high(Modifiers { friction: Some(2.1), ..Modifiers::NONE }, Some(MarkError::Friction(m())))]
    #[case::friction_nan(Modifiers { friction: Some(f32::NAN), ..Modifiers::NONE }, Some(MarkError::Friction(m())))]
    #[case::speed_zero_is_a_stall(Modifiers { speed: 0.0, ..Modifiers::NONE }, None)]
    #[case::speed_negative(Modifiers { speed: -1.0, ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::speed_infinite(Modifiers { speed: f32::INFINITY, ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::value_zero_is_a_fake(Modifiers { value: 0.0, ..Modifiers::NONE }, None)]
    #[case::value_too_high(Modifiers { value: 17.0, ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::value_nan(Modifiers { value: f32::NAN, ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::mass_too_high(Modifiers { mass: p(17.0), ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::size_too_low(Modifiers { size: p(0.05), ..Modifiers::NONE }, Some(MarkError::Factor(m())))]
    #[case::heading_half_turn(Modifiers { heading: std::f32::consts::PI, ..Modifiers::NONE }, None)]
    #[case::heading_beyond(Modifiers { heading: 3.2, ..Modifiers::NONE }, Some(MarkError::Heading(m())))]
    #[case::heading_nan(Modifiers { heading: f32::NAN, ..Modifiers::NONE }, Some(MarkError::Heading(m())))]
    fn mark_effects_are_validated(#[case] modifiers: Modifiers, #[case] error: Option<MarkError>) {
        assert_eq!(def(modifiers).err(), error);
    }

    #[rstest]
    fn new_mark_starts_with_one_stack() {
        let mut c = Conditions::default();
        c.apply(&wet(), Some(Tick(10)));
        assert_eq!(
            c.marks(),
            [ActiveMark {
                mark: wet().id().clone(),
                stacks: stacks(1),
                until: Some(Tick(10))
            }]
        );
        assert!(c.has(wet().id()));
    }

    #[rstest]
    fn reapplying_stacks_up_to_the_limit_and_keeps_the_later_deadline() {
        let mut c = Conditions::default();
        for until in [10, 30, 20, 25] {
            c.apply(&poison(), Some(Tick(until)));
        }
        assert_eq!(c.marks()[0].stacks, stacks(3), "capped at max_stacks");
        assert_eq!(
            c.marks()[0].until,
            Some(Tick(30)),
            "a shorter one does not shorten it"
        );
    }

    #[rstest]
    #[case::permanent_wins_over_timed(Some(10), None, None)]
    #[case::timed_does_not_end_permanent(None, Some(10), None)]
    fn permanent_marks_stay_permanent(
        #[case] first: Option<u64>,
        #[case] second: Option<u64>,
        #[case] expected: Option<u64>,
    ) {
        let mut c = Conditions::default();
        c.apply(&grow(), first.map(Tick));
        c.apply(&grow(), second.map(Tick));
        assert_eq!(c.marks()[0].until, expected.map(Tick));
    }

    #[rstest]
    fn expired_marks_are_removed_and_report_vanishing() {
        let mut c = Conditions::default();
        c.apply(&wet(), Some(Tick(10)));
        c.apply(&poison(), Some(Tick(20)));
        c.apply(&grow(), None);

        assert_eq!(c.expire(Tick(9), book), Expiry::default(), "nothing is due yet");
        assert_eq!(c.expire(Tick(10), book), Expiry { changed: true, vanish: false });
        assert_eq!(c.expire(Tick(25), book), Expiry { changed: true, vanish: true });
        assert_eq!(c.marks().len(), 1, "the permanent mark stays");
        assert!(c.has(grow().id()));
    }

    #[rstest]
    #[case::harmful_only(true, &["wet", "grow"])]
    #[case::everything(false, &[])]
    fn cleanse_removes_marks(#[case] harmful_only: bool, #[case] left: &[&str]) {
        let mut c = Conditions::default();
        for mark in [wet(), poison(), grow(), sand()] {
            c.apply(&mark, None);
        }
        assert!(c.cleanse(harmful_only, book));
        let ids: Vec<_> = c.marks().iter().map(|m| m.mark.as_str()).collect();
        assert_eq!(ids, left);
    }

    #[rstest]
    fn cleansing_a_clean_target_changes_nothing() {
        let mut c = Conditions::default();
        c.apply(&wet(), None);
        assert!(!c.cleanse(true, book));
        assert!(!Conditions::default().cleanse(false, book));
    }

    #[rstest]
    fn remove_takes_off_one_mark() {
        let mut c = Conditions::default();
        c.apply(&wet(), None);
        c.apply(&sand(), None);
        assert!(c.remove(wet().id()));
        assert!(!c.remove(wet().id()));
        assert!(c.has(sand().id()));
    }

    #[rstest]
    fn no_marks_means_no_change() {
        assert_eq!(Conditions::default().modifiers(book), Modifiers::NONE);
    }

    #[rstest]
    fn stacked_factors_multiply_and_the_last_friction_wins() {
        let mut c = Conditions::default();
        c.apply(&wet(), None);
        c.apply(&poison(), None);
        c.apply(&poison(), None);
        c.apply(&sand(), None);
        let m = c.modifiers(book);
        assert_eq!(m.friction, Some(1.2), "sand was applied after wet");
        assert_eq!(m.mass, p(0.25), "poison x2: 0.5^2");
        assert_eq!(m.speed, 0.5 * 0.5 * 0.6);
        assert!(!m.sealed);
    }

    #[rstest]
    fn combined_factors_stay_within_the_limit() {
        let mut c = Conditions::default();
        for _ in 0..5 {
            c.apply(&grow(), None);
        }
        let m = c.modifiers(book);
        assert_eq!(m.mass, p(MAX_FACTOR), "2^5 = 32 is clamped");
        assert_eq!(m.size, p(1.5f32.powi(5)));
    }

    #[rstest]
    fn combined_speed_and_value_are_clamped() {
        let haste = MarkDef::new(
            MarkId::new("haste"),
            false,
            stacks(3),
            false,
            Modifiers { speed: 4.0, value: 4.0, ..Modifiers::NONE },
        )
        .expect("valid");
        let mut c = Conditions::default();
        for _ in 0..3 {
            c.apply(&haste, None);
        }
        let m = c.modifiers(|id| (id == haste.id()).then_some(&haste));
        assert_eq!((m.speed, m.value), (MAX_FACTOR, MAX_FACTOR), "4^3 = 64 is clamped");
    }

    #[rstest]
    fn unknown_marks_are_ignored() {
        let mut c = Conditions::default();
        c.apply(
            &def(Modifiers { value: 0.0, ..Modifiers::NONE }).expect("valid"),
            None,
        );
        assert_eq!(c.modifiers(book), Modifiers::NONE);
    }

    fn any_mark() -> impl Strategy<Value = MarkDef> {
        prop_oneof![Just(wet()), Just(poison()), Just(grow()), Just(sand())]
    }

    proptest! {
        /// どう付けても、重ね掛けは上限を超えず、期限切れの印は残らず、効き目は値域に収まる。
        #[test]
        fn conditions_stay_within_their_invariants(
            applied in prop::collection::vec((any_mark(), prop::option::of(0u64..100)), 0..40),
            now in 0u64..120,
        ) {
            let mut c = Conditions::default();
            for (mark, until) in &applied {
                c.apply(mark, until.map(Tick));
            }
            for active in c.marks() {
                let def = book(&active.mark).expect("known");
                prop_assert!(active.stacks <= def.max_stacks());
            }
            c.expire(Tick(now), book);
            prop_assert!(c.marks().iter().all(|m| m.until.is_none_or(|t| t > Tick(now))));
            let m = c.modifiers(book);
            prop_assert!((1.0 / MAX_FACTOR..=MAX_FACTOR).contains(&m.mass.get()));
            prop_assert!((0.0..=MAX_FACTOR).contains(&m.speed));
            prop_assert!(m.heading.is_finite());
        }
    }
}
