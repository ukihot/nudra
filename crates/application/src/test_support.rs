//! applicationのテスト用フィクスチャとモック。

use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU32,
};

use domain::{
    difficulty::Difficulty,
    id::{
        BridgeId, EnemyTypeId, GameSessionId, GoalId, MedalCode, PusherCharacterId, RailId, StageId,
    },
    math::Vec3,
    positive::Positive,
    pusher::{MovementPattern, PusherCharacter, PusherStats},
    repository::{PusherCharacterRepository, StageRepository},
    session::GameSession,
    spec::{
        EnemySpec, EvolutionRules, MedalDef, MedalLevels, MedalSpec, MedalTier, SpecCatalog,
        SpecParts,
    },
    stage::{
        Abyss, Bridge, Catapult, CatapultParts, Geometry, Goal, InitialMedalPlacement, Rail, Shape,
        SpawnEntry, Stage, StageParts, Wave, WavePlan,
    },
    time::Ticks,
};
use rstest::fixture;

mockall::mock! {
    pub Stages {}
    impl StageRepository for Stages {
        fn find(&self, id: &StageId) -> Option<Stage>;
    }
}

mockall::mock! {
    pub Characters {}
    impl PusherCharacterRepository for Characters {
        fn find(&self, id: &PusherCharacterId) -> Option<PusherCharacter>;
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

pub fn spawn(at_tick: u64) -> SpawnEntry {
    SpawnEntry {
        at: Ticks(at_tick),
        enemy: EnemyTypeId::new("grunt"),
        bridge: BridgeId(1),
        goal: GoalId(1),
    }
}

/// 4Wave: 第1Waveは通常2体（Tick 0,
/// 60）と中ボス（120）、第2・3Waveは中ボスのみ、第4Waveは最終ボス。
#[fixture]
pub fn stage() -> Stage {
    Stage::new(StageParts {
        id: StageId::new("stage_01"),
        geometry: Geometry {
            shape: Shape::Rectangle { width: p(10.0), depth: p(10.0) },
            floor_y: 0.0,
        },
        catapult: Catapult::new(CatapultParts {
            position: Vec3::new(0.0, 1.0, 5.0),
            base_yaw: 0.0,
            yaw_range: (-0.5, 0.5),
            initial_yaw: 0.0,
            pitch: 0.2,
            speed: 10.0,
            fire_interval: Ticks(12),
        })
        .expect("valid catapult"),
        bridges: vec![Bridge {
            id: BridgeId(1),
            entry: Vec3::X * 10.0,
            exit: Vec3::X * 5.0,
            width: p(2.0),
        }],
        abysses: vec![
            Abyss::new(Vec3::splat(-50.0), Vec3::new(50.0, -2.0, 50.0)).expect("valid abyss"),
        ],
        rails: vec![
            Rail::new(RailId(7), vec![Vec3::new(-2.0, 0.5, 0.0), Vec3::new(2.0, 0.5, 0.0)])
                .expect("valid rail"),
            Rail::new(RailId(8), vec![Vec3::ZERO, Vec3::Z]).expect("valid rail"),
        ],
        goals: vec![Goal {
            id: GoalId(1),
            position: Vec3::new(0.0, 0.0, -4.0),
            radius: p(1.0),
            available_from: Difficulty::EASY,
        }],
        initial_medals: vec![InitialMedalPlacement::grid(Vec3::ZERO, 2, 2, 1, Vec3::ONE)],
        wave_plan: WavePlan::new(vec![
            Wave::new(vec![spawn(0), spawn(60)], spawn(120)),
            Wave::new(vec![], spawn(0)),
            Wave::new(vec![], spawn(0)),
            Wave::boss_only(spawn(0)),
        ]),
    })
    .expect("valid stage")
}

#[fixture]
pub fn character() -> PusherCharacter {
    PusherCharacter {
        id: PusherCharacterId::new("mop_rabbit"),
        name: "Mop Rabbit".into(),
        base_stats: PusherStats {
            speed: p(2.0),
            mass: p(10.0),
            push_force: p(100.0),
            push_width: p(1.5),
        },
        movement: MovementPattern::PingPong { pause_at_ends: 0.3 },
    }
}

/// Lv.1 のメダル
pub fn medal_spec() -> MedalSpec {
    MedalSpec {
        mass: p(1.0),
        radius: p(0.3),
        thickness: p(0.08),
        friction: 0.5,
        restitution: 0.1,
    }
}

/// レベルが上がるごとに質量 +1
pub fn levels() -> MedalLevels {
    [1.0, 2.0, 3.0, 4.0].map(|mass| MedalSpec { mass: p(mass), ..medal_spec() })
}

fn def(tier: MedalTier) -> MedalDef {
    MedalDef { tier, levels: levels() }
}

/// ノーマル、素材 fire・water、錬成 sauna（fire + water）。ゲージは10で満タン、Easy
/// は1枚10ポイント。
pub fn spec_parts() -> SpecParts {
    let enemy = EnemySpec {
        mass: p(2.0),
        radius: p(0.3),
        height: p(0.8),
        max_speed: 1.5,
        walk_force: 10.0,
        friction: 0.5,
    };
    SpecParts {
        medals: BTreeMap::from([
            (MedalCode::normal(), def(MedalTier::Normal)),
            (fire(), def(MedalTier::Element)),
            (water(), def(MedalTier::Element)),
            (sauna(), def(MedalTier::Fusion)),
        ]),
        enemies: HashMap::from([(EnemyTypeId::new("grunt"), enemy)]),
        fusions: vec![(fire(), water(), sauna())],
        evolution: EvolutionRules {
            threshold: NonZeroU32::new(10).expect("non-zero"),
            points_per_medal: [10, 5, 2, 1],
        },
        abilities: Default::default(),
    }
}

#[fixture]
pub fn specs() -> SpecCatalog {
    SpecCatalog::new(spec_parts()).expect("valid specs")
}

#[fixture]
pub fn session(stage: Stage, character: PusherCharacter, specs: SpecCatalog) -> GameSession {
    GameSession::start(
        GameSessionId(1),
        &stage,
        &character,
        Difficulty::EASY,
        specs.evolution(),
        1,
    )
    .0
}

/// テスト用の簡潔な生成。不正値ならパニックする。
pub fn p(value: f32) -> Positive {
    Positive::new(value).expect("positive")
}

pub fn lv(n: u8) -> domain::evolution::MedalLevel {
    domain::evolution::MedalLevel::new(n).expect("valid level")
}

/// 印: 濡れ（滑る）・しびれ（足止め）・焦げ（切れると消える）。濡れ + しびれ → 跳ね上がる。
/// fire: 衝突で爆風と濡れ（60tick）。
pub fn ability_specs() -> SpecCatalog {
    use std::num::NonZeroU8;

    use domain::{
        ability::{AbilityDef, Area, CollideWith, Direction, Effect, Target, Trigger},
        id::MarkId,
        mark::{MarkDef, Modifiers},
        reaction::ReactionDef,
        spec::abilities::MedalAbilities,
    };

    let mark = |id: &str, vanish, modifiers| {
        MarkDef::new(MarkId::new(id), false, NonZeroU8::MIN, vanish, modifiers).expect("valid mark")
    };
    let mut parts = spec_parts();
    parts.abilities.marks = vec![
        mark("wet", false, Modifiers { friction: Some(0.05), ..Modifiers::NONE }),
        mark("shock", false, Modifiers { speed: 0.0, ..Modifiers::NONE }),
        mark("burn", true, Modifiers::NONE),
    ];
    parts.abilities.reactions = vec![ReactionDef {
        on: MarkId::new("wet"),
        by: MarkId::new("shock"),
        effects: vec![Effect::Push { dir: Direction::Up, strength: p(3.0) }],
        consume: false,
    }];
    parts.abilities.medals = BTreeMap::from([(
        fire(),
        MedalAbilities {
            base: vec![AbilityDef {
                on: Trigger::Collide(CollideWith::Any),
                target: Target::Both,
                area: Area::Radius(p(1.0)),
                effects: vec![
                    Effect::Push { dir: Direction::Outward, strength: p(2.0) },
                    Effect::Mark {
                        mark: MarkId::new("wet"),
                        duration: Some(Ticks(60)),
                    },
                ],
                uses: None,
            }],
            awaken: vec![],
        },
    )]);
    SpecCatalog::new(parts).expect("valid specs")
}
