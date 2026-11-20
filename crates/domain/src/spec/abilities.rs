//! メダルの能力・印・反応の定義（基本設計 3.14）。
//!
//! 読込時にレベル倍率と覚醒を適用した Lv.1〜4 の能力を作り、検証も済ませる。
//! 実行中に能力の解決が失敗することは無い。

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ability::{AbilityDef, Effect, Happening, ScaleError, SpawnKind, Trigger},
    evolution::MedalLevel,
    id::{MarkId, MedalId},
    mark::{Conditions, MarkDef},
    positive::Positive,
    reaction::{Hit, ReactionDef, hit},
    time::{Tick, Ticks},
};

const LEVELS: usize = MedalLevel::MAX.get() as usize;

/// メダル1種の能力。`awaken` は Lv.4 で加わる能力（覚醒）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MedalAbilities {
    pub base: Vec<AbilityDef>,
    pub awaken: Vec<AbilityDef>,
}

/// `AbilityBook::new` に渡す組み立て用の値。
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityParts {
    pub marks: Vec<MarkDef>,
    pub reactions: Vec<ReactionDef>,
    pub medals: BTreeMap<MedalId, MedalAbilities>,
    /// Lv.1〜4 の倍率（強さ・範囲・時間・ポイント）
    pub scale: [Positive; LEVELS],
}

impl Default for AbilityParts {
    fn default() -> Self {
        Self {
            marks: Vec::new(),
            reactions: Vec::new(),
            medals: BTreeMap::new(),
            scale: [Positive::ONE; LEVELS],
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AbilityBookError {
    #[error("mark {0} is defined twice")]
    DuplicateMark(MarkId),
    #[error("unknown mark {0}")]
    UnknownMark(MarkId),
    #[error("reaction of {0} by {1} is defined twice")]
    DuplicateReaction(MarkId, MarkId),
    #[error("abilities are defined for unknown medal {0}")]
    UnknownMedal(MedalId),
    #[error("an ability of {0} has a zero duration or interval")]
    ZeroDuration(MedalId),
    #[error("reaction of {0} by {1} has a zero duration")]
    ReactionZeroDuration(MarkId, MarkId),
    #[error("reaction of {0} by {1} cannot replay an ability")]
    ReplayInReaction(MarkId, MarkId),
    #[error("revival of {0} must be triggered by its own fall and limited in uses")]
    InvalidRevive(MedalId),
    #[error("{0} spawns medals with abilities without a limit (set uses or a sealing mark)")]
    UnboundedSpawn(MedalId),
    #[error("abilities of {medal} do not stay within range at some level")]
    Scale {
        medal: MedalId,
        #[source]
        source: ScaleError,
    },
}

/// 検証済みの能力の定義。
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityBook {
    marks: BTreeMap<MarkId, MarkDef>,
    reactions: Vec<ReactionDef>,
    /// メダル → Lv.1〜4 の能力（倍率と覚醒を適用済み）
    medals: BTreeMap<MedalId, [Vec<AbilityDef>; LEVELS]>,
}

impl AbilityBook {
    /// `known` は定義済みのメダルか（メダルの物理特性の定義と突き合わせる）。
    pub fn new(
        parts: AbilityParts,
        known: impl Fn(&MedalId) -> bool,
    ) -> Result<Self, AbilityBookError> {
        let AbilityParts { marks: defs, reactions, medals: abilities, scale } = parts;
        let mut marks = BTreeMap::new();
        for def in defs {
            if marks.contains_key(def.id()) {
                return Err(AbilityBookError::DuplicateMark(def.id().clone()));
            }
            marks.insert(def.id().clone(), def);
        }
        let known_mark = |id: &MarkId| {
            if marks.contains_key(id) {
                Ok(())
            } else {
                Err(AbilityBookError::UnknownMark(id.clone()))
            }
        };
        let check_marks = |effects: &[Effect]| {
            effects
                .iter()
                .filter_map(mark_of)
                .try_for_each(known_mark)
        };

        let mut pairs = BTreeSet::new();
        for r in &reactions {
            known_mark(&r.on)?;
            known_mark(&r.by)?;
            check_marks(&r.effects)?;
            if r.effects.contains(&Effect::Replay) {
                return Err(AbilityBookError::ReplayInReaction(r.on.clone(), r.by.clone()));
            }
            if r.effects.iter().any(zero_duration) {
                return Err(AbilityBookError::ReactionZeroDuration(r.on.clone(), r.by.clone()));
            }
            if !pairs.insert((&r.on, &r.by)) {
                return Err(AbilityBookError::DuplicateReaction(r.on.clone(), r.by.clone()));
            }
        }

        let sealing = |mark: &Option<MarkId>| {
            mark.as_ref()
                .and_then(|m| marks.get(m))
                .is_some_and(|def| def.modifiers().sealed)
        };
        let mut medals = BTreeMap::new();
        for (medal, MedalAbilities { base, awaken }) in abilities {
            if !known(&medal) {
                return Err(AbilityBookError::UnknownMedal(medal));
            }
            for ability in base.iter().chain(&awaken) {
                check_marks(&ability.effects)?;
                for effect in &ability.effects {
                    match effect {
                        Effect::Revive { .. }
                            if ability.on != Trigger::SelfFell || ability.uses.is_none() =>
                        {
                            return Err(AbilityBookError::InvalidRevive(medal));
                        },
                        // 能力を持つメダルの生成は、回数か封印で上限を付ける（無限増殖を防ぐ）
                        Effect::Spawn { kind, mark, .. }
                            if *kind != SpawnKind::Normal
                                && ability.uses.is_none()
                                && !sealing(mark) =>
                        {
                            return Err(AbilityBookError::UnboundedSpawn(medal));
                        },
                        _ => {},
                    }
                }
            }
            let mut levels: [Vec<AbilityDef>; LEVELS] = Default::default();
            for (i, (abilities, factor)) in levels.iter_mut().zip(scale).enumerate() {
                let awakened = if i + 1 == LEVELS {
                    awaken.as_slice()
                } else {
                    &[]
                };
                *abilities = base
                    .iter()
                    .chain(awakened)
                    .map(|a| a.scaled(factor))
                    .collect::<Result<_, _>>()
                    .map_err(|source| AbilityBookError::Scale { medal: medal.clone(), source })?;
                // 倍率を掛けた後で調べる（四捨五入で0になることがある）
                if abilities.iter().any(|a| {
                    matches!(a.on, Trigger::Fuse(Ticks(0)) | Trigger::Periodic(Ticks(0)))
                        || a.effects.iter().any(zero_duration)
                }) {
                    return Err(AbilityBookError::ZeroDuration(medal));
                }
            }
            medals.insert(medal, levels);
        }
        Ok(Self { marks, reactions, medals })
    }

    /// そのレベルで持つ能力。能力の無いメダルは空。
    pub fn abilities(&self, medal: &MedalId, level: MedalLevel) -> &[AbilityDef] {
        self.medals
            .get(medal)
            .map_or(&[], |levels| &levels[usize::from(level.get() - 1)])
    }

    /// その出来事で発動する能力（番号つき。番号は [`crate::ability::AbilityRef`] に使う）。
    pub fn triggered(
        &self,
        medal: &MedalId,
        level: MedalLevel,
        happening: Happening,
    ) -> impl Iterator<Item = (usize, &AbilityDef)> {
        self.abilities(medal, level)
            .iter()
            .enumerate()
            .filter(move |(_, a)| a.on.fires_on(happening))
    }

    pub fn mark(&self, mark: &MarkId) -> Option<&MarkDef> {
        self.marks.get(mark)
    }

    /// 効果が対象に当たったときの印と反応（[`crate::reaction::hit`]）。
    pub fn hit(&self, target: &mut Conditions, incoming: &[Effect], now: Tick) -> Hit {
        hit(target, incoming, now, |m| self.marks.get(m), &self.reactions)
    }
}

/// 効果が参照する印。
fn mark_of(effect: &Effect) -> Option<&MarkId> {
    match effect {
        Effect::Mark { mark, .. } | Effect::Zone { mark, .. } => Some(mark),
        Effect::Spawn { mark, .. } => mark.as_ref(),
        _ => None,
    }
}

/// 長さ0の時間を持つ効果か（付けた瞬間に切れる印・場）。
fn zero_duration(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::Mark { duration: Some(Ticks(0)), .. }
            | Effect::Zone { duration: Ticks(0), .. }
            | Effect::Zone { linger: Ticks(0), .. }
    )
}
#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use rstest::rstest;

    use super::*;
    use crate::{
        ability::{
            Area, CollideWith, Direction, Happening, Partner, SpawnAt, Target, TransformInto,
        },
        mark::{
            Modifiers,
            fixture::{poison, sand, wet},
        },
        positive::fixture::p,
        spec::fixture::{fire, water},
    };

    fn known(m: &MedalId) -> bool {
        [MedalId::normal(), fire(), water()].contains(m)
    }

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    fn ability(on: Trigger, effects: Vec<Effect>) -> AbilityDef {
        AbilityDef {
            on,
            target: Target::Both,
            area: Area::Radius(p(1.0)),
            effects,
            uses: None,
        }
    }

    fn blast(strength: f32) -> Effect {
        Effect::Push { dir: Direction::Outward, strength: p(strength) }
    }

    fn mark(id: &str, duration: Option<u64>) -> Effect {
        Effect::Mark {
            mark: MarkId::new(id),
            duration: duration.map(Ticks),
        }
    }

    fn spawn(kind: SpawnKind, mark: Option<&str>) -> Effect {
        Effect::Spawn {
            kind,
            count: NonZeroU8::MIN,
            at: SpawnAt::Here,
            mark: mark.map(MarkId::new),
        }
    }

    /// 封印の印（生成したメダルの能力を止める）
    fn sealed() -> MarkDef {
        MarkDef::new(
            MarkId::new("sealed"),
            false,
            NonZeroU8::MIN,
            false,
            Modifiers { sealed: true, ..Modifiers::NONE },
        )
        .expect("valid")
    }

    /// fire: 接触で爆風（覚醒で濡らす）。倍率は Lv.1〜4 で 1, 1.5, 2, 3。濡れ + 毒 → 砂。
    fn parts() -> AbilityParts {
        AbilityParts {
            marks: vec![wet(), poison(), sand(), sealed()],
            reactions: vec![ReactionDef {
                on: MarkId::new("wet"),
                by: MarkId::new("poison"),
                effects: vec![mark("sand", Some(10))],
                consume: true,
            }],
            medals: BTreeMap::from([(
                fire(),
                MedalAbilities {
                    base: vec![ability(Trigger::Collide(CollideWith::Any), vec![blast(2.0)])],
                    awaken: vec![ability(
                        Trigger::Collide(CollideWith::Any),
                        vec![mark("wet", Some(10))],
                    )],
                },
            )]),
            scale: [p(1.0), p(1.5), p(2.0), p(3.0)],
        }
    }

    fn book() -> AbilityBook {
        AbilityBook::new(parts(), known).expect("valid")
    }

    #[rstest]
    fn nothing_defined_is_valid_and_gives_no_abilities() {
        let book = AbilityBook::new(AbilityParts::default(), known).expect("valid");
        assert!(book.abilities(&fire(), lv(1)).is_empty());
    }

    #[rstest]
    #[case::level_1(1, 1.0)]
    #[case::level_2(2, 1.5)]
    #[case::level_3(3, 2.0)]
    fn abilities_are_scaled_by_level(#[case] level: u8, #[case] factor: f32) {
        assert_eq!(
            book().abilities(&fire(), lv(level)),
            [AbilityDef {
                area: Area::Radius(p(factor)),
                ..ability(Trigger::Collide(CollideWith::Any), vec![blast(2.0 * factor)])
            }]
        );
    }

    #[rstest]
    fn level_four_adds_the_awakened_ability_also_scaled() {
        let book = book();
        assert_eq!(
            book.abilities(&fire(), lv(4)),
            [
                AbilityDef {
                    area: Area::Radius(p(3.0)),
                    ..ability(Trigger::Collide(CollideWith::Any), vec![blast(6.0)])
                },
                AbilityDef {
                    area: Area::Radius(p(3.0)),
                    ..ability(Trigger::Collide(CollideWith::Any), vec![mark("wet", Some(30))])
                },
            ]
        );
    }

    #[rstest]
    fn only_abilities_for_the_happening_are_triggered() {
        let collide = |with| ability(Trigger::Collide(with), vec![blast(1.0)]);
        let book = with(|x| {
            *base(x) = vec![
                collide(CollideWith::Any),
                ability(Trigger::Rest, vec![blast(2.0)]),
                collide(CollideWith::Enemies),
            ]
        })
        .expect("valid");
        let indices = |happening| {
            book.triggered(&fire(), lv(1), happening)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        assert_eq!(indices(Happening::Collided(Partner::Enemy)), [0, 2]);
        assert_eq!(indices(Happening::Collided(Partner::Medal)), [0]);
        assert_eq!(indices(Happening::Motion(crate::motion::MotionEvent::Rested)), [1]);
        assert_eq!(indices(Happening::Launched), Vec::<usize>::new());
        assert_eq!(
            book.triggered(&water(), lv(1), Happening::Launched)
                .count(),
            0
        );
    }
    #[rstest]
    fn medals_without_abilities_have_none() {
        assert!(book().abilities(&water(), lv(4)).is_empty());
    }

    #[rstest]
    fn hit_uses_the_defined_marks_and_reactions() {
        let book = book();
        let mut target = Conditions::default();
        book.hit(&mut target, &[mark("wet", None)], Tick(0));
        let out = book.hit(&mut target, &[mark("poison", None)], Tick(5));
        assert!(out.changed);
        assert!(target.has(&MarkId::new("sand")));
        assert!(!target.has(&MarkId::new("wet")), "consumed by the reaction");
        assert_eq!(book.mark(&MarkId::new("wet")), Some(&wet()));
    }

    fn with(edit: impl FnOnce(&mut AbilityParts)) -> Result<AbilityBook, AbilityBookError> {
        let mut parts = parts();
        edit(&mut parts);
        AbilityBook::new(parts, known)
    }

    fn base(parts: &mut AbilityParts) -> &mut Vec<AbilityDef> {
        &mut parts.medals.get_mut(&fire()).expect("fire").base
    }

    #[rstest]
    #[case::duplicate_mark(|x: &mut AbilityParts| x.marks.push(wet()), AbilityBookError::DuplicateMark(MarkId::new("wet")))]
    #[case::unknown_mark_in_ability(|x: &mut AbilityParts| base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![mark("ice", None)])), AbilityBookError::UnknownMark(MarkId::new("ice")))]
    #[case::unknown_mark_in_zone(|x: &mut AbilityParts| base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![Effect::Zone { mark: MarkId::new("ice"), radius: p(1.0), duration: Ticks(5), linger: Ticks(5) }])), AbilityBookError::UnknownMark(MarkId::new("ice")))]
    #[case::unknown_mark_on_spawn(|x: &mut AbilityParts| base(x).push(AbilityDef { uses: NonZeroU8::new(1), ..ability(Trigger::Launch, vec![spawn(SpawnKind::Same, Some("ice"))]) }), AbilityBookError::UnknownMark(MarkId::new("ice")))]
    #[case::unknown_mark_in_reaction(|x: &mut AbilityParts| x.reactions[0].by = MarkId::new("ice"), AbilityBookError::UnknownMark(MarkId::new("ice")))]
    #[case::unknown_mark_in_reaction_effect(|x: &mut AbilityParts| x.reactions[0].effects.push(mark("ice", None)), AbilityBookError::UnknownMark(MarkId::new("ice")))]
    #[case::duplicate_reaction(|x: &mut AbilityParts| x.reactions.push(x.reactions[0].clone()), AbilityBookError::DuplicateReaction(MarkId::new("wet"), MarkId::new("poison")))]
    #[case::unknown_medal(|x: &mut AbilityParts| { x.medals.insert(MedalId::new("ghost"), MedalAbilities::default()); }, AbilityBookError::UnknownMedal(MedalId::new("ghost")))]
    #[case::zero_fuse(|x: &mut AbilityParts| base(x).push(ability(Trigger::Fuse(Ticks(0)), vec![])), AbilityBookError::ZeroDuration(fire()))]
    #[case::zero_interval(|x: &mut AbilityParts| base(x).push(ability(Trigger::Periodic(Ticks(0)), vec![])), AbilityBookError::ZeroDuration(fire()))]
    #[case::zero_mark(|x: &mut AbilityParts| base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![mark("wet", Some(0))])), AbilityBookError::ZeroDuration(fire()))]
    #[case::zero_after_scaling(|x: &mut AbilityParts| { x.scale[0] = p(0.4); base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![mark("wet", Some(1))])); }, AbilityBookError::ZeroDuration(fire()))]
    #[case::zero_in_reaction(|x: &mut AbilityParts| x.reactions[0].effects = vec![mark("sand", Some(0))], AbilityBookError::ReactionZeroDuration(MarkId::new("wet"), MarkId::new("poison")))]
    #[case::zero_linger(|x: &mut AbilityParts| base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![Effect::Zone { mark: MarkId::new("wet"), radius: p(1.0), duration: Ticks(5), linger: Ticks(0) }])), AbilityBookError::ZeroDuration(fire()))]
    #[case::replay_in_reaction(|x: &mut AbilityParts| x.reactions[0].effects.push(Effect::Replay), AbilityBookError::ReplayInReaction(MarkId::new("wet"), MarkId::new("poison")))]
    #[case::revive_on_contact(|x: &mut AbilityParts| base(x).push(AbilityDef { uses: NonZeroU8::new(1), ..ability(Trigger::Collide(CollideWith::Any), vec![Effect::Revive { at: SpawnAt::Catapult }]) }), AbilityBookError::InvalidRevive(fire()))]
    #[case::endless_revive(|x: &mut AbilityParts| base(x).push(ability(Trigger::SelfFell, vec![Effect::Revive { at: SpawnAt::Catapult }])), AbilityBookError::InvalidRevive(fire()))]
    #[case::endless_clones(|x: &mut AbilityParts| base(x).push(ability(Trigger::Periodic(Ticks(60)), vec![spawn(SpawnKind::Same, None)])), AbilityBookError::UnboundedSpawn(fire()))]
    #[case::endless_elements(|x: &mut AbilityParts| base(x).push(ability(Trigger::Collide(CollideWith::Any), vec![spawn(SpawnKind::RandomElement, Some("wet"))])), AbilityBookError::UnboundedSpawn(fire()))]
    #[case::scale_overflow(|x: &mut AbilityParts| x.scale[3] = p(f32::MAX), AbilityBookError::Scale { medal: fire(), source: ScaleError(f32::MAX) })]
    fn invalid_definitions_are_rejected(
        #[case] edit: fn(&mut AbilityParts),
        #[case] expected: AbilityBookError,
    ) {
        assert_eq!(with(edit), Err(expected));
    }

    #[rstest]
    #[case::limited_clones(AbilityDef { uses: NonZeroU8::new(3), ..ability(Trigger::Periodic(Ticks(60)), vec![spawn(SpawnKind::Same, None)]) })]
    #[case::sealed_clones(ability(Trigger::Periodic(Ticks(60)), vec![spawn(SpawnKind::Same, Some("sealed"))]))]
    #[case::normal_medals(ability(Trigger::Periodic(Ticks(60)), vec![spawn(SpawnKind::Normal, None)]))]
    #[case::limited_revive(AbilityDef { uses: NonZeroU8::new(2), ..ability(Trigger::SelfFell, vec![Effect::Revive { at: SpawnAt::Sky }]) })]
    #[case::transform(ability(Trigger::Collide(CollideWith::Any), vec![Effect::Transform(TransformInto::RandomElement)]))]
    fn bounded_abilities_are_accepted(#[case] extra: AbilityDef) {
        assert!(with(|p| base(p).push(extra)).is_ok());
    }
}
