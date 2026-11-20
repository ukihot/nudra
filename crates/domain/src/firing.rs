//! 能力の発動（ドメインサービス、基本設計 3.16）。
//!
//! セッション（進行中か・直前の能力・ゲージ・乱数）、能力の定義、メダル個体（封印・
//! 回数）の3つにまたがる操作。 結果は ECS への指示 [`AbilityOrder`]
//! として返す。範囲内の対象の検索と物理は ECS が行う。

use std::num::NonZeroU8;

use crate::{
    ability::{
        AbilityDef, AbilityRef, Area, Effect, Happening, SpawnAt, SpawnKind, Target, TransformInto,
    },
    event::DomainEvent,
    evolution::MedalLevel,
    id::{MarkId, MedalCode},
    session::GameSession,
    spec::SpecCatalog,
    unit::MedalUnit,
};

/// ECS が実行する効果。生成・変化のメダル種とレベルは解決済み。
#[derive(Debug, Clone, PartialEq)]
pub enum EffectOrder {
    /// 対象に当てる・その場に置く効果（押す・場・消去・分割・復活）
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

/// 発動したメダルの位置で実行する能力1つ。
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityOrder {
    pub target: Target,
    pub area: Area,
    /// 範囲内の対象それぞれに当てる状態の効果（印・浄化）。
    /// 対象ごとに選り分けずに済むよう分けておく
    pub states: Vec<Effect>,
    /// それ以外（押す・場・生成・変化・消去・分割・復活）
    pub actions: Vec<EffectOrder>,
}

/// 発動の結果。ゲージの加算はセッションに適用済みで、その出来事を `events` に入れる。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Firing {
    pub orders: Vec<AbilityOrder>,
    pub events: Vec<DomainEvent>,
}

/// メダル個体に起きた出来事で、その個体の能力を発動する。
///
/// - 封印されている個体は何も発動しない
/// - 回数を使い切った能力は発動しない。発動できたときだけ回数を数える
/// - 進行できない間（終了後・進化の選択中）は何も発動しない。ゲージが満タンになったら、
///   同じ呼び出しの残りも止まる
/// - 再生は直前の能力も同じ位置で実行する。効果の残らない指示（再生だけの能力など）は返さない
pub fn fire(
    session: &mut GameSession,
    specs: &SpecCatalog,
    unit: &mut MedalUnit,
    happening: Happening,
) -> Firing {
    let mut out = Firing::default();
    let book = specs.abilities();
    if unit.modifiers(book).sealed {
        return out;
    }
    let triggered: Vec<_> = book
        .triggered(unit.medal(), unit.level(), happening)
        .collect();
    for (index, ability) in triggered {
        // 復活は能力の回数に加えて、個体通算の上限（基本設計 3.18）
        if !unit.can_use(index, ability.uses) || (revives(ability) && !unit.can_revive()) {
            continue;
        }
        let fired = AbilityRef {
            medal: unit.medal().clone(),
            level: unit.level(),
            index,
        };
        let Some(fired) = session.fire_ability(&fired, specs) else {
            continue;
        };
        unit.use_once(index, ability.uses);
        if revives(ability) {
            unit.revive();
        }
        // 再生で復活を迂回しない（落下以外・上限超過の復活は再生しない）
        let replayed = fired
            .replayed
            .filter(|r| !revives(r) || (happening == Happening::SelfFell && unit.revive()));
        for ability in std::iter::once(fired.ability).chain(replayed) {
            let order = resolve(session, specs, unit, ability, &mut out.events);
            if !(order.states.is_empty() && order.actions.is_empty()) {
                out.orders.push(order);
            }
        }
    }
    out
}

/// 能力を ECS への指示にする。ゲージはセッションに足し、生成・変化のメダルを決める。
fn resolve(
    session: &mut GameSession,
    specs: &SpecCatalog,
    unit: &MedalUnit,
    ability: &AbilityDef,
    events: &mut Vec<DomainEvent>,
) -> AbilityOrder {
    let (mut states, mut actions) = (Vec::new(), Vec::new());
    for effect in &ability.effects {
        let order = match effect {
            state if state.is_state() => {
                states.push(state.clone());
                continue;
            },
            Effect::Gauge { points } => {
                events.extend(session.add_gauge(*points, specs));
                continue;
            },
            Effect::Replay => continue,
            Effect::Spawn { kind, count, at, mark, rewarded } => {
                let medal = match kind {
                    SpawnKind::Same => Some(unit.medal().clone()),
                    SpawnKind::Normal => Some(MedalCode::normal()),
                    SpawnKind::RandomElement => session.random_element(specs),
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
                    TransformInto::RandomElement => session.random_element(specs),
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

/// 落ちた自分を戻す能力か。
fn revives(ability: &AbilityDef) -> bool {
    ability
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Revive { .. }))
}

/// 生成・変化したメダルのレベル: 所持していればそのレベル、していなければ Lv.1。
fn owned_level(session: &GameSession, medal: &MedalCode) -> MedalLevel {
    session
        .medals()
        .level(medal)
        .unwrap_or(MedalLevel::MIN)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        ability::{CollideWith, Direction, Partner},
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
        time::{Tick, Ticks},
    };

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    fn blast() -> Effect {
        Effect::Push { dir: Direction::Outward, strength: p(2.0) }
    }

    fn ability(on: crate::ability::Trigger, effects: Vec<Effect>, uses: Option<u8>) -> AbilityDef {
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

    /// fire: 衝突で爆風＋3ポイント／静止で同種を生成（2回まで）／着地で変化／射出でノーマルを生成。
    /// water: 衝突で直前の能力を再生。Easy は1枚4ポイント、10で満タン。
    #[fixture]
    fn specs() -> SpecCatalog {
        use crate::ability::Trigger::*;
        let spawn = |kind| Effect::Spawn {
            kind,
            count: NonZeroU8::MIN,
            at: SpawnAt::Here,
            mark: None,
            rewarded: false,
        };
        let mut parts = spec_parts();
        parts.abilities.marks = vec![sealed()];
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
                        ability(Rest, vec![spawn(SpawnKind::Same)], Some(2)),
                        AbilityDef {
                            target: Target::Medals,
                            ..ability(
                                Land,
                                vec![Effect::Transform(TransformInto::RandomElement)],
                                None,
                            )
                        },
                        ability(Launch, vec![spawn(SpawnKind::Normal)], None),
                    ],
                    awaken: vec![],
                },
            ),
            (
                water(),
                MedalAbilities {
                    base: vec![ability(Collide(CollideWith::Any), vec![Effect::Replay], None)],
                    awaken: vec![],
                },
            ),
            (
                sauna(),
                MedalAbilities {
                    base: vec![
                        ability(SelfFell, vec![Effect::Revive { at: SpawnAt::Catapult }], Some(2)),
                        ability(
                            Launch,
                            vec![Effect::Spawn {
                                kind: SpawnKind::Normal,
                                count: NonZeroU8::MIN,
                                at: SpawnAt::Sky,
                                mark: None,
                                rewarded: true,
                            }],
                            None,
                        ),
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

    fn collided() -> Happening {
        Happening::Collided(Partner::Enemy)
    }

    fn unit(medal: MedalCode) -> MedalUnit {
        MedalUnit::new(medal, lv(1))
    }

    #[rstest]
    fn a_collision_fires_the_blast_and_adds_points(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let out = super::fire(&mut session, &specs, &mut unit(fire()), collided());
        assert_eq!(
            out,
            Firing {
                orders: vec![AbilityOrder {
                    target: Target::Both,
                    area: Area::Radius(p(1.0)),
                    states: vec![],
                    actions: vec![EffectOrder::Apply(blast())],
                }],
                events: vec![],
            }
        );
        assert_eq!(session.gauge().ratio(), 0.3);
    }

    #[rstest]
    fn marks_are_separated_from_actions(session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let mut parts = spec_parts();
        let wet = crate::mark::fixture::wet();
        parts.abilities.marks = vec![wet.clone()];
        let mark = Effect::Mark { mark: wet.id().clone(), duration: None };
        parts.abilities.medals = BTreeMap::from([(
            fire(),
            MedalAbilities {
                base: vec![ability(
                    crate::ability::Trigger::Launch,
                    vec![mark.clone(), blast()],
                    None,
                )],
                awaken: vec![],
            },
        )]);
        let specs = SpecCatalog::new(parts).expect("valid");
        let out = super::fire(&mut session, &specs, &mut unit(fire()), Happening::Launched);
        assert_eq!(out.orders[0].states, [mark]);
        assert_eq!(out.orders[0].actions, [EffectOrder::Apply(blast())]);
    }

    #[rstest]
    fn a_rewarded_spawn_passes_its_reward_on(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let out = super::fire(&mut session, &specs, &mut unit(sauna()), Happening::Launched);
        assert!(matches!(
            out.orders[0].actions.as_slice(),
            [EffectOrder::Spawn { rewarded: true, .. }]
        ));
    }

    /// 能力の `uses` が2でも、個体通算の復活は1回まで。使った復活は戻らない。
    #[rstest]
    fn revival_stops_at_the_unit_limit(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let mut zombie = unit(sauna());
        let first = super::fire(&mut session, &specs, &mut zombie, Happening::SelfFell);
        assert_eq!(
            first.orders[0].actions,
            [EffectOrder::Apply(Effect::Revive { at: SpawnAt::Catapult })]
        );
        assert_eq!(zombie.lineage().revives, 1);
        let before = zombie.clone();
        assert_eq!(
            super::fire(&mut session, &specs, &mut zombie, Happening::SelfFell),
            Firing::default()
        );
        assert_eq!(zombie, before, "the refused revival uses nothing");
    }
    #[rstest]
    fn other_happenings_fire_nothing(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let out = super::fire(
            &mut session,
            &specs,
            &mut unit(fire()),
            Happening::Motion(MotionEvent::Woke),
        );
        assert_eq!(out, Firing::default());
    }

    #[rstest]
    fn a_full_gauge_stops_further_firing(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        session.medals_fell(2.0, &specs);
        let out = super::fire(&mut session, &specs, &mut unit(fire()), collided());
        assert_eq!(out.events, [DomainEvent::EvolutionReady]);
        let paused = super::fire(&mut session, &specs, &mut unit(fire()), collided());
        assert_eq!(paused, Firing::default(), "waiting for the evolution choice");
    }

    #[rstest]
    fn a_sealed_medal_fires_nothing(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        let mut sealed_unit = unit(fire());
        sealed_unit.conditions_mut().apply(&sealed(), None);
        assert_eq!(
            super::fire(&mut session, &specs, &mut sealed_unit, collided()),
            Firing::default()
        );
        assert_eq!(session.gauge().ratio(), 0.0);
    }

    #[rstest]
    fn limited_abilities_run_out_and_spawn_the_same_kind(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let mut u = MedalUnit::new(fire(), lv(3));
        let rested = Happening::Motion(MotionEvent::Rested);
        for _ in 0..2 {
            let out = super::fire(&mut session, &specs, &mut u, rested);
            assert_eq!(
                out.orders[0].actions,
                [EffectOrder::Spawn {
                    medal: fire(),
                    level: lv(3),
                    count: NonZeroU8::MIN,
                    at: SpawnAt::Here,
                    mark: None,
                    rewarded: false,
                }]
            );
        }
        assert_eq!(
            super::fire(&mut session, &specs, &mut u, rested),
            Firing::default(),
            "used up"
        );
    }

    #[rstest]
    fn normal_medals_spawn_at_the_owned_level(specs: SpecCatalog, session: (GameSession, Stage)) {
        let (mut session, _) = session;
        session.medals_fell(10.0, &specs);
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &specs)
            .expect("level up");
        let out = super::fire(&mut session, &specs, &mut unit(fire()), Happening::Launched);
        assert_eq!(
            out.orders[0].actions,
            [EffectOrder::Spawn {
                medal: MedalCode::normal(),
                level: lv(2),
                count: NonZeroU8::MIN,
                at: SpawnAt::Here,
                mark: None,
                rewarded: false,
            }]
        );
    }

    #[rstest]
    fn random_transforms_pick_an_element_at_its_owned_level(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        let landed = Happening::Motion(MotionEvent::Landed);
        let out = super::fire(&mut session, &specs, &mut unit(fire()), landed);
        let [EffectOrder::Transform { medal, level }] = out.orders[0].actions.as_slice() else {
            panic!("a transform: {out:?}");
        };
        assert!([fire(), water()].contains(medal));
        assert_eq!(*level, MedalLevel::MIN, "not owned yet");
    }

    #[rstest]
    fn replay_runs_the_last_ability_and_drops_its_empty_order(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, _) = session;
        super::fire(&mut session, &specs, &mut unit(fire()), collided());
        let out = super::fire(&mut session, &specs, &mut unit(water()), collided());
        assert_eq!(out.orders.len(), 1);
        assert_eq!(out.orders[0].actions, [EffectOrder::Apply(blast())]);
        assert_eq!(session.gauge().ratio(), 0.6, "the replayed gauge points count too");
    }

    #[rstest]
    fn nothing_fires_or_is_used_up_after_the_game(
        specs: SpecCatalog,
        session: (GameSession, Stage),
    ) {
        let (mut session, stage) = session;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        let mut u = unit(fire());
        let before = u.clone();
        assert_eq!(
            super::fire(&mut session, &specs, &mut u, Happening::Motion(MotionEvent::Rested)),
            Firing::default()
        );
        assert_eq!(u, before);
        let _ = Ticks(0);
    }
}
