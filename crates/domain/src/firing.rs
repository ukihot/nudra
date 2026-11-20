//! 能力の発動（ドメインサービス、基本設計 3.16）。
//!
//! セッション（進行中か・直前の能力・ゲージ・乱数）、能力の定義、メダル個体（封印・回数・復活・
//! 世代）にまたがる。 **計画と確定を分ける**: [`plan`]
//! は何も変更せずに能力1つの計画を作り、[`commit`] が回数・復活・乱数・履歴・ゲージを確定する。
//! その間に呼び出し側が対象の検索と予算の予約を行い、
//! 成り立たなければ計画を捨てるだけで済む（全か無か）。

use std::num::NonZeroU8;

use crate::{
    ability::{
        AbilityDef, AbilityRef, Area, Effect, Happening, SpawnAt, SpawnKind, Target, TransformInto,
    },
    event::DomainEvent,
    evolution::MedalLevel,
    id::{MarkId, MedalCode},
    random::SessionRng,
    session::{GameSession, pick_element},
    spec::SpecCatalog,
    unit::MedalUnit,
};

/// ECS が実行する効果。生成・変化のメダル種とレベルは解決済み。
#[derive(Debug, Clone, PartialEq)]
pub enum EffectOrder {
    /// 対象に当てる・その場に置く効果（押す・場・消去・分割）
    Apply(Effect),
    Spawn {
        medal: MedalCode,
        level: MedalLevel,
        count: NonZeroU8,
        at: SpawnAt,
        mark: Option<MarkId>,
        rewarded: bool,
    },
    Transform {
        medal: MedalCode,
        level: MedalLevel,
    },
}

/// 発動したメダルの位置で実行する能力1つ分の指示。
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityOrder {
    pub target: Target,
    pub area: Area,
    /// 範囲内の対象それぞれに当てる状態の効果（印・浄化）。
    /// 対象ごとに選り分けずに済むよう分けておく
    pub states: Vec<Effect>,
    /// それ以外（押す・場・生成・変化・消去・分割）
    pub actions: Vec<EffectOrder>,
}

impl AbilityOrder {
    /// 対象1体あたりの分割の数（分割は新しいメダルを生むため、生成と同じ予算・世代の制限を通す）。
    pub fn splits(&self) -> u32 {
        let splits = self
            .actions
            .iter()
            .filter(|a| matches!(a, EffectOrder::Apply(Effect::Split)));
        u32::try_from(splits.count()).unwrap_or(u32::MAX)
    }
}

/// 出来事の文脈。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occasion {
    pub happening: Happening,
    /// 衝突の相手がいる（範囲 `Touched` の効果に必要）
    pub partner: bool,
}

/// 計画が成り立たない理由。計画を作れなかったときは何も変更していない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanRejection {
    #[error("the game is not running")]
    NotRunning,
    #[error("the ability is not defined")]
    Undefined,
    #[error("the medal is sealed")]
    Sealed,
    #[error("the ability is used up")]
    UsedUp,
    #[error("the medal cannot revive any more")]
    NoRevive,
    #[error("a medal revives only on its own fall")]
    ReviveOutsideFall,
    #[error("the medal's generation cannot spawn")]
    Generation,
    #[error("there is no ability to replay")]
    NothingToReplay,
    #[error("the ability needs a partner it touched")]
    NoPartner,
    #[error("the ability does not fire on this happening")]
    WrongTrigger,
}

/// 計画を作った後に、セッションか個体の状態が変わった（別の確定が挟まった）。確定しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the plan was made from a state that has changed since")]
pub struct StalePlan;

/// 能力1つの計画。乱数は候補状態で計算済みで、確定するまでセッションには反映しない。
///
/// 確定は所有権ごと消費するため、同じ計画を2回確定できない（`Clone` も持たない）。
#[derive(Debug, PartialEq)]
pub struct AbilityPlan {
    pub source: AbilityRef,
    /// 自分の能力と、再生した能力（あれば）の指示。効果の残らない指示は含めない
    pub orders: Vec<AbilityOrder>,
    /// 確定時にゲージへ足すポイント
    pub gauge: u32,
    /// 落ちた自分を戻す位置（復活の計画）
    pub revive: Option<SpawnAt>,
    uses: Option<NonZeroU8>,
    /// 直前の能力として記録する（再生を含む能力は記録しない）
    record: bool,
    /// 確定後の乱数の状態
    rng: SessionRng,
    /// 計画を作った時点の状態（確定時に変わっていないことを確かめる）
    basis: Basis,
}

#[derive(Debug, PartialEq)]
struct Basis {
    rng: SessionRng,
    last: Option<AbilityRef>,
    unit: MedalUnit,
}

impl AbilityPlan {
    /// 生成するメダルの数（予算の予約に使う）。
    pub fn spawns(&self) -> u32 {
        self.actions()
            .map(|a| match a {
                EffectOrder::Spawn { count, .. } => u32::from(count.get()),
                _ => 0,
            })
            .sum()
    }

    /// 分割する数（確定前に範囲内の対象数を掛けて、生成と同じ予算に数える）。
    pub fn splits(&self) -> u32 {
        self.orders.iter().map(AbilityOrder::splits).sum()
    }

    /// 置く場の数（予算の予約に使う）。
    pub fn zones(&self) -> u32 {
        let zones = self
            .actions()
            .filter(|a| matches!(a, EffectOrder::Apply(Effect::Zone { .. })));
        u32::try_from(zones.count()).unwrap_or(u32::MAX)
    }

    /// 計画を作った時点から、セッションと個体の状態が変わっていないか（進行できることも含む）。
    pub fn is_fresh(&self, session: &GameSession, unit: &MedalUnit) -> bool {
        session.ensure_running().is_ok()
            && session.rng() == &self.basis.rng
            && session.last_ability() == self.basis.last.as_ref()
            && *unit == self.basis.unit
    }

    fn actions(&self) -> impl Iterator<Item = &EffectOrder> {
        self.orders.iter().flat_map(|o| &o.actions)
    }
}

/// 個体に起きた出来事で発動する能力の番号（定義順）。
pub fn triggered<'a>(
    specs: &'a SpecCatalog,
    unit: &'a MedalUnit,
    happening: Happening,
) -> impl Iterator<Item = usize> + 'a {
    specs
        .abilities()
        .triggered(unit.medal(), unit.level(), happening)
        .map(|(index, _)| index)
}

/// 能力1つの計画を作る。セッションも個体も変更しない。
///
/// - 進行できない・未定義・封印・回数切れ・復活できない（上限・落下以外）・世代の上限・
///   相手が要るのにいない・ 再生する履歴がない、のいずれかなら成り立たない
/// - 再生は直前に確定した非再生の能力を今回の文脈で実行する。再生先の前提（復活・世代・
///   相手）も同じく検証する
pub fn plan(
    session: &GameSession,
    specs: &SpecCatalog,
    unit: &MedalUnit,
    index: usize,
    occasion: Occasion,
) -> Result<AbilityPlan, PlanRejection> {
    session
        .ensure_running()
        .map_err(|_| PlanRejection::NotRunning)?;
    let book = specs.abilities();
    let ability = book
        .abilities(unit.medal(), unit.level())
        .get(index)
        .ok_or(PlanRejection::Undefined)?;
    // 呼び出し側の誤りで別の発動条件の能力を発動しない
    if !book
        .triggered(unit.medal(), unit.level(), occasion.happening)
        .any(|(i, _)| i == index)
    {
        return Err(PlanRejection::WrongTrigger);
    }
    if unit.modifiers(book).sealed {
        return Err(PlanRejection::Sealed);
    }
    if !unit.can_use(index, ability.uses) {
        return Err(PlanRejection::UsedUp);
    }
    check(ability, unit, occasion)?;
    let replaying = ability.effects.contains(&Effect::Replay);
    let replayed = if replaying {
        let last = session
            .last_ability()
            .ok_or(PlanRejection::NothingToReplay)?;
        let replayed = book
            .abilities(&last.medal, last.level)
            .get(last.index)
            .ok_or(PlanRejection::NothingToReplay)?;
        check(replayed, unit, occasion)?;
        Some(replayed)
    } else {
        None
    };
    let mut rng = session.rng().clone();
    let mut gauge = 0u32;
    let orders = std::iter::once(ability)
        .chain(replayed)
        .map(|a| resolve(session, specs, unit, a, &mut rng, &mut gauge))
        .filter(|o| !(o.states.is_empty() && o.actions.is_empty()))
        .collect();
    Ok(AbilityPlan {
        source: AbilityRef {
            medal: unit.medal().clone(),
            level: unit.level(),
            index,
        },
        orders,
        gauge,
        revive: revives(ability).or_else(|| replayed.and_then(revives)),
        uses: ability.uses,
        record: !replaying,
        rng,
        basis: Basis {
            rng: session.rng().clone(),
            last: session.last_ability().cloned(),
            unit: unit.clone(),
        },
    })
}

/// 能力の前提（自分の能力にも、再生する能力にも同じく）。
fn check(ability: &AbilityDef, unit: &MedalUnit, occasion: Occasion) -> Result<(), PlanRejection> {
    if revives(ability).is_some() {
        if occasion.happening != Happening::SelfFell {
            return Err(PlanRejection::ReviveOutsideFall);
        }
        if !unit.can_revive() {
            return Err(PlanRejection::NoRevive);
        }
    }
    // 相手に当てる効果は、今回の相手がいなければ成り立たない（最近傍で代替しない）
    if ability.area == Area::Touched && ability.target != Target::Itself && !occasion.partner {
        return Err(PlanRejection::NoPartner);
    }
    let spawns = ability
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Spawn { .. }));
    if spawns && !unit.can_spawn() {
        return Err(PlanRejection::Generation);
    }
    Ok(())
}

/// 落ちた自分を戻す位置（復活の能力なら）。
fn revives(ability: &AbilityDef) -> Option<SpawnAt> {
    ability.effects.iter().find_map(|e| match e {
        Effect::Revive { at } => Some(*at),
        _ => None,
    })
}

/// 能力を指示にする。ゲージは計画に積み、生成・変化のメダルは候補の乱数で決める。
fn resolve(
    session: &GameSession,
    specs: &SpecCatalog,
    unit: &MedalUnit,
    ability: &AbilityDef,
    rng: &mut SessionRng,
    gauge: &mut u32,
) -> AbilityOrder {
    let (mut states, mut actions) = (Vec::new(), Vec::new());
    for effect in &ability.effects {
        let order = match effect {
            state if state.is_state() => {
                states.push(state.clone());
                continue;
            },
            Effect::Gauge { points } => {
                *gauge = gauge.saturating_add(*points);
                continue;
            },
            // 再生は計画で解決済み、復活は計画の `revive` で扱う
            Effect::Replay | Effect::Revive { .. } => continue,
            Effect::Spawn { kind, count, at, mark, rewarded } => {
                let medal = match kind {
                    SpawnKind::Same => Some(unit.medal().clone()),
                    SpawnKind::Normal => Some(MedalCode::normal()),
                    SpawnKind::RandomElement => pick_element(rng, specs),
                };
                let Some(medal) = medal else {
                    continue;
                };
                let level = if *kind == SpawnKind::Same {
                    unit.level()
                } else {
                    owned_level(session, &medal)
                };
                EffectOrder::Spawn {
                    medal,
                    level,
                    count: *count,
                    at: *at,
                    mark: mark.clone(),
                    rewarded: *rewarded,
                }
            },
            Effect::Transform(into) => {
                let medal = match into {
                    TransformInto::Normal => Some(MedalCode::normal()),
                    TransformInto::RandomElement => pick_element(rng, specs),
                };
                let Some(medal) = medal else {
                    continue;
                };
                let level = owned_level(session, &medal);
                EffectOrder::Transform { medal, level }
            },
            other => EffectOrder::Apply(other.clone()),
        };
        actions.push(order);
    }
    AbilityOrder {
        target: ability.target,
        area: ability.area,
        states,
        actions,
    }
}

/// 生成・変化したメダルのレベル: 所持していればそのレベル、していなければ Lv.1。
fn owned_level(session: &GameSession, medal: &MedalCode) -> MedalLevel {
    session
        .medals()
        .level(medal)
        .unwrap_or(MedalLevel::MIN)
}

/// 計画を確定する: 使用回数・復活回数・乱数・直前の能力・ゲージ。ゲージで出た出来事を返す。
///
/// 呼び出し側の約束（基本設計 3.16）: 計画 → 対象の検索と適用前の検証 → 予算の予約 → ECSへの適用 →
/// 確定、を 1つの直列区間で行い、確定の後に失敗し得る処理を残さない。計画の後に状態が変わっていれば
/// （別の確定が挟まった・個体が変わった・進行できなくなった）`StalePlan` で、何も変更しない。
pub fn commit(
    session: &mut GameSession,
    specs: &SpecCatalog,
    unit: &mut MedalUnit,
    plan: AbilityPlan,
) -> Result<Vec<DomainEvent>, StalePlan> {
    if !plan.is_fresh(session, unit) {
        return Err(StalePlan);
    }
    unit.use_once(plan.source.index, plan.uses);
    if plan.revive.is_some() {
        unit.revive();
    }
    if plan.record {
        session.record_ability(plan.source);
    }
    session.set_rng(plan.rng);
    Ok(if plan.gauge > 0 {
        session.add_gauge(plan.gauge, specs)
    } else {
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        ability::{CollideWith, Direction, Partner, Trigger},
        difficulty::Difficulty,
        evolution::Evolution,
        id::{GameSessionId, GoalId, PusherCharacterId},
        mark::{MarkDef, Modifiers},
        motion::MotionEvent,
        positive::fixture::p,
        pusher::{MovementPattern, PusherCharacter, PusherStats},
        spec::{
            abilities::MedalAbilities,
            fixture::{fire, parts as spec_parts, sauna, water},
        },
        stage::{
            Stage,
            fixture::{parts, stage},
        },
        time::Tick,
    };

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    fn blast() -> Effect {
        Effect::Push { dir: Direction::Outward, strength: p(2.0) }
    }

    fn ability(on: Trigger, effects: Vec<Effect>, uses: Option<u8>) -> AbilityDef {
        AbilityDef {
            on,
            target: Target::Both,
            area: Area::Radius(p(1.0)),
            effects,
            uses: uses.and_then(NonZeroU8::new),
        }
    }

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

    fn spawn(kind: SpawnKind, rewarded: bool) -> Effect {
        Effect::Spawn {
            kind,
            count: NonZeroU8::MIN,
            at: SpawnAt::Here,
            mark: None,
            rewarded,
        }
    }

    /// 能力の番号（定義順）:
    /// - fire: 0 衝突で爆風＋3点 / 1 静止で同種を生成（2回）/ 2 着地でランダムな素材に変化 / 3
    ///   射出でノーマルを生成
    /// - water: 0 衝突で直前の能力を再生 / 1 衝突した相手を押す（相手が要る）
    /// - sauna: 0 自分が落ちたらカタパルトへ復活（2回）/ 1 射出で有価のノーマルを生成
    ///
    /// Easy は1枚4ポイント、10で満タン。
    #[fixture]
    fn specs() -> SpecCatalog {
        use crate::ability::Trigger::*;
        let mut parts = spec_parts();
        parts.abilities.marks = vec![sealed()];
        let touched = AbilityDef {
            area: Area::Touched,
            ..ability(Collide(CollideWith::Any), vec![blast()], None)
        };
        parts.abilities.medals = BTreeMap::from([
            (
                fire(),
                MedalAbilities {
                    base: vec![
                        ability(
                            Collide(CollideWith::Any),
                            vec![blast(), Effect::Gauge { points: 3 }],
                            None,
                        ),
                        ability(Rest, vec![spawn(SpawnKind::Same, false)], Some(2)),
                        AbilityDef {
                            target: Target::Medals,
                            ..ability(
                                Land,
                                vec![Effect::Transform(TransformInto::RandomElement)],
                                None,
                            )
                        },
                        ability(Launch, vec![spawn(SpawnKind::Normal, false)], None),
                    ],
                    awaken: vec![],
                },
            ),
            (
                water(),
                MedalAbilities {
                    base: vec![
                        ability(Collide(CollideWith::Any), vec![Effect::Replay], None),
                        touched,
                    ],
                    awaken: vec![],
                },
            ),
            (
                sauna(),
                MedalAbilities {
                    base: vec![
                        ability(SelfFell, vec![Effect::Revive { at: SpawnAt::Catapult }], Some(2)),
                        ability(Launch, vec![spawn(SpawnKind::Normal, true)], None),
                    ],
                    awaken: vec![],
                },
            ),
        ]);
        SpecCatalog::new(parts).expect("valid")
    }

    #[fixture]
    fn session(specs: SpecCatalog) -> (GameSession, Stage) {
        let character = PusherCharacter {
            id: PusherCharacterId::new("rabbit"),
            name: "Rabbit".into(),
            base_stats: PusherStats {
                speed: p(2.0),
                mass: p(10.0),
                push_force: p(100.0),
                push_width: p(1.5),
            },
            movement: MovementPattern::PingPong { pause_at_ends: 0.3 },
        };
        let stage = stage(parts());
        let (session, _) = GameSession::start(
            GameSessionId(1),
            &stage,
            &character,
            Difficulty::EASY,
            specs.evolution(),
            7,
        );
        (session, stage)
    }

    fn unit(medal: MedalCode) -> MedalUnit {
        MedalUnit::new(medal, lv(1))
    }

    fn on(happening: Happening) -> Occasion {
        Occasion { happening, partner: false }
    }

    fn collided() -> Occasion {
        Occasion {
            happening: Happening::Collided(Partner::Enemy),
            partner: true,
        }
    }

    /// 確定した計画の要点（計画は確定で消費されるため写しておく）。
    struct Fired {
        #[allow(dead_code)]
        gauge: u32,
        revive: Option<SpawnAt>,
    }

    /// 計画して、そのまま確定する。
    fn fire_now(
        session: &mut GameSession,
        specs: &SpecCatalog,
        unit: &mut MedalUnit,
        index: usize,
        occasion: Occasion,
    ) -> Result<(Fired, Vec<DomainEvent>), PlanRejection> {
        let planned = plan(session, specs, unit, index, occasion)?;
        let (gauge, revive) = (planned.gauge, planned.revive);
        let events = commit(session, specs, unit, planned).expect("a fresh plan");
        Ok((Fired { gauge, revive }, events))
    }

    #[rstest]
    fn triggered_lists_the_abilities_of_the_happening(specs: SpecCatalog) {
        let u = unit(fire());
        assert_eq!(
            triggered(&specs, &u, Happening::Collided(Partner::Medal)).collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(triggered(&specs, &u, Happening::Launched).collect::<Vec<_>>(), [3]);
    }

    #[rstest]
    fn planning_changes_nothing(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (session, _) = session;
        let u = unit(fire());
        let (before_session, before_unit) = (format!("{session:?}"), u.clone());
        let planned = plan(&session, &specs, &u, 0, collided()).expect("planned");
        assert_eq!(
            planned.orders,
            [AbilityOrder {
                target: Target::Both,
                area: Area::Radius(p(1.0)),
                states: vec![],
                actions: vec![EffectOrder::Apply(blast())],
            }]
        );
        assert_eq!(planned.gauge, 3);
        assert_eq!(format!("{session:?}"), before_session, "the session is untouched");
        assert_eq!(u, before_unit);
    }

    #[rstest]
    fn committing_adds_the_gauge_points(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        fire_now(&mut session, &specs, &mut unit(fire()), 0, collided()).expect("fired");
        assert_eq!(session.gauge().ratio(), 0.3);
    }

    #[rstest]
    fn a_full_gauge_is_reported_and_stops_later_plans(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        session.medals_fell(2.0, &specs);
        let (_, events) =
            fire_now(&mut session, &specs, &mut unit(fire()), 0, collided()).expect("fired");
        assert_eq!(events, [DomainEvent::EvolutionReady]);
        assert_eq!(
            plan(&session, &specs, &unit(fire()), 0, collided()),
            Err(PlanRejection::NotRunning)
        );
    }

    #[rstest]
    fn plans_are_rejected_for_these_reasons(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (session, _) = session;
        let mut sealed_unit = unit(fire());
        sealed_unit.conditions_mut().apply(&sealed(), None);
        let rested = on(Happening::Motion(MotionEvent::Rested));
        let mut used_up = unit(fire());
        used_up.use_once(1, NonZeroU8::new(2));
        used_up.use_once(1, NonZeroU8::new(2));
        let child = unit(fire()).offspring(fire(), lv(1), false);
        let mut revived = unit(sauna());
        assert!(revived.revive());
        let cases = [
            (unit(fire()), 9, collided(), PlanRejection::Undefined),
            (sealed_unit, 0, collided(), PlanRejection::Sealed),
            (used_up, 1, rested, PlanRejection::UsedUp),
            (child, 1, rested, PlanRejection::Generation),
            (revived, 0, on(Happening::SelfFell), PlanRejection::NoRevive),
            (unit(sauna()), 0, on(Happening::Launched), PlanRejection::WrongTrigger),
            (unit(fire()), 1, collided(), PlanRejection::WrongTrigger),
            (unit(water()), 0, collided(), PlanRejection::NothingToReplay),
            (
                unit(water()),
                1,
                Occasion {
                    happening: Happening::Collided(Partner::Medal),
                    partner: false,
                },
                PlanRejection::NoPartner,
            ),
        ];
        for (u, index, occasion, rejection) in cases {
            assert_eq!(
                plan(&session, &specs, &u, index, occasion),
                Err(rejection),
                "{u:?} #{index}"
            );
        }
    }

    #[rstest]
    fn using_up_and_reviving_happen_only_on_commit(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let mut u = unit(fire());
        let rested = on(Happening::Motion(MotionEvent::Rested));
        for _ in 0..3 {
            plan(&session, &specs, &u, 1, rested).expect("plans do not use anything up");
        }
        for _ in 0..2 {
            fire_now(&mut session, &specs, &mut u, 1, rested).expect("fired");
        }
        assert_eq!(plan(&session, &specs, &u, 1, rested), Err(PlanRejection::UsedUp));

        let mut zombie = unit(sauna());
        let (planned, _) = fire_now(&mut session, &specs, &mut zombie, 0, on(Happening::SelfFell))
            .expect("revives");
        assert_eq!(planned.revive, Some(SpawnAt::Catapult));
        assert_eq!(zombie.lineage().revives, 1);
    }

    #[rstest]
    fn spawns_resolve_kind_level_and_reward(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let rested = on(Happening::Motion(MotionEvent::Rested));
        let same =
            plan(&session, &specs, &MedalUnit::new(fire(), lv(3)), 1, rested).expect("planned");
        assert_eq!(
            same.orders[0].actions,
            [EffectOrder::Spawn {
                medal: fire(),
                level: lv(3),
                count: NonZeroU8::MIN,
                at: SpawnAt::Here,
                mark: None,
                rewarded: false
            }]
        );
        assert_eq!((same.spawns(), same.zones()), (1, 0));

        session.medals_fell(10.0, &specs);
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &specs)
            .expect("level up");
        let normal =
            plan(&session, &specs, &unit(sauna()), 1, on(Happening::Launched)).expect("planned");
        assert_eq!(
            normal.orders[0].actions,
            [EffectOrder::Spawn {
                medal: MedalCode::normal(),
                level: lv(2),
                count: NonZeroU8::MIN,
                at: SpawnAt::Here,
                mark: None,
                rewarded: true
            }]
        );
    }

    /// ランダムな変化は候補の乱数で計画し、確定したときだけセッションの乱数が進む。
    #[rstest]
    fn random_choices_advance_the_rng_only_on_commit(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let landed = on(Happening::Motion(MotionEvent::Landed));
        let u = unit(fire());
        let first = plan(&session, &specs, &u, 2, landed).expect("planned");
        assert_eq!(
            plan(&session, &specs, &u, 2, landed).expect("planned"),
            first,
            "same state, same plan"
        );
        let [EffectOrder::Transform { medal, level }] = first.orders[0].actions.as_slice() else {
            panic!("a transform: {first:?}");
        };
        assert!([fire(), water()].contains(medal));
        assert_eq!(*level, MedalLevel::MIN, "not owned yet");
        let before = session.rng().clone();
        commit(&mut session, &specs, &mut u.clone(), first).expect("a fresh plan");
        assert_ne!(session.rng(), &before, "the committed choice advanced the rng");
    }

    #[rstest]
    fn replay_uses_the_last_committed_ability_in_the_new_context(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        fire_now(&mut session, &specs, &mut unit(fire()), 0, collided()).expect("fired");
        let replay = plan(&session, &specs, &unit(water()), 0, collided()).expect("replays");
        assert_eq!(replay.orders.len(), 1, "the replay order itself is empty and dropped");
        assert_eq!(replay.orders[0].actions, [EffectOrder::Apply(blast())]);
        assert_eq!(replay.gauge, 3, "the replayed points count too");
    }

    #[rstest]
    fn a_replay_is_not_recorded(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        fire_now(&mut session, &specs, &mut unit(fire()), 0, collided()).expect("fired");
        let recorded = session.last_ability().cloned();
        fire_now(&mut session, &specs, &mut unit(water()), 0, collided()).expect("replayed");
        assert_eq!(session.last_ability().cloned(), recorded);
    }

    #[rstest]
    fn a_replayed_ability_needs_its_own_context(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        // 相手が要る能力（water #1）を確定してから、相手のいない文脈で再生する
        fire_now(&mut session, &specs, &mut unit(water()), 1, collided()).expect("fired");
        let lonely = Occasion {
            happening: Happening::Collided(Partner::Medal),
            partner: false,
        };
        assert_eq!(
            plan(&session, &specs, &unit(water()), 0, lonely),
            Err(PlanRejection::NoPartner)
        );
        // 復活は落下以外では再生しない
        fire_now(&mut session, &specs, &mut unit(sauna()), 0, on(Happening::SelfFell))
            .expect("revived");
        assert_eq!(
            plan(&session, &specs, &unit(water()), 0, collided()),
            Err(PlanRejection::ReviveOutsideFall)
        );
    }

    /// 同じ状態から作った2つの計画のうち、
    /// 後から確定するほうは古い（状態が変わった）ので確定しない。
    #[rstest]
    fn a_stale_plan_is_not_committed(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let mut u = unit(fire());
        let first = plan(&session, &specs, &u, 0, collided()).expect("planned");
        let second = plan(&session, &specs, &u, 0, collided()).expect("planned");
        commit(&mut session, &specs, &mut u, first).expect("fresh");
        let (before_session, before_unit) = (format!("{session:?}"), u.clone());
        assert_eq!(commit(&mut session, &specs, &mut u, second), Err(StalePlan));
        assert_eq!(format!("{session:?}"), before_session, "the gauge is not added twice");
        assert_eq!(u, before_unit);
    }

    #[rstest]
    fn a_plan_for_another_unit_state_is_stale(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let mut u = unit(fire());
        let rested = on(Happening::Motion(MotionEvent::Rested));
        let planned = plan(&session, &specs, &u, 1, rested).expect("planned");
        u.use_once(1, NonZeroU8::new(2));
        assert_eq!(commit(&mut session, &specs, &mut u, planned), Err(StalePlan));
    }

    #[rstest]
    fn a_plan_made_before_the_rng_moved_is_stale(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let mut u = unit(fire());
        let planned = plan(&session, &specs, &u, 2, on(Happening::Motion(MotionEvent::Landed)))
            .expect("planned");
        session.random_element(&specs);
        assert_eq!(commit(&mut session, &specs, &mut u, planned), Err(StalePlan));
    }
    #[rstest]
    fn splits_are_counted_per_target(specs: SpecCatalog) {
        let order = AbilityOrder {
            target: Target::Medals,
            area: Area::Radius(p(1.0)),
            states: vec![],
            actions: vec![EffectOrder::Apply(Effect::Split), EffectOrder::Apply(blast())],
        };
        assert_eq!(order.splits(), 1);
        let _ = specs;
    }
    #[rstest]
    fn nothing_is_planned_after_the_game(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, stage) = session;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        assert_eq!(
            plan(&session, &specs, &unit(fire()), 0, collided()),
            Err(PlanRejection::NotRunning)
        );
    }
}
