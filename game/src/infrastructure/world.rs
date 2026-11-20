//! Entityの生成（基本設計 5.3）。物理はAvianのComponent、見た目は `Appearance` で指定するだけ。
//! InGameで生成するEntityはすべて `DespawnOnExit(AppState::InGame)` を付ける。

use application::PusherSetup;
use avian3d::prelude::*;
use bevy::prelude::*;
use domain::{
    difficulty::Difficulty,
    evolution::MedalLevel,
    id::{EnemyId, GoalId, MedalCode, PusherCharacterId, RailId},
    mark::{Conditions, Modifiers},
    motion::MedalMotion,
    navigation::Route,
    pusher::{MovementState, PusherStats},
    spec::{EnemySpec, MedalSpec},
    stage::{Shape, Stage},
    unit::MedalUnit,
};

use super::{
    abilities::AbilityTimers,
    physics::{Layer, layers},
};
use crate::{
    port::{Appearance, MedalMotionState, Silhouette},
    state::AppState,
};

// ── Component（Domainの値を包むnewtype） ───────────────────

/// メダル個体。種類・レベル・印・能力の使用回数（Domain の値、基本設計 3.16）。
#[derive(Component, Debug, Clone)]
pub struct Medal(pub MedalUnit);

/// Domain ID と Bevy Entity の関連付けはここだけ（基本設計 3.7）。
#[derive(Component, Debug, Clone, Copy)]
pub struct Enemy {
    pub id: EnemyId,
}

/// 敵にかかっている印と、その効き目（歩行の速さ・進路に反映する、基本設計 3.14）。
#[derive(Component, Debug, Clone)]
pub struct EnemyMarks {
    pub conditions: Conditions,
    pub modifiers: Modifiers,
}

impl Default for EnemyMarks {
    fn default() -> Self {
        Self {
            conditions: Conditions::default(),
            modifiers: Modifiers::NONE,
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct Navigation(pub Route);

#[derive(Component, Debug, Clone, Copy)]
pub struct Walker {
    pub max_speed: f32,
    pub walk_force: f32,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Pusher {
    pub stats: PusherStats,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PusherMotion {
    pub rail: RailId,
    pub state: MovementState,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct AbyssZone;

#[derive(Component, Debug, Clone, Copy)]
pub struct GoalZone(pub GoalId);

// ── 寸法（調整ノブ） ──────────────────────────────────────

const FLOOR_THICKNESS: f32 = 0.5;
const GOAL_HEIGHT: f32 = 2.0;
const PUSHER_HEIGHT: f32 = 0.8;
const PUSHER_THICKNESS: f32 = 0.5;
const CATAPULT_SIZE: Vec3 = Vec3::new(1.2, 0.6, 1.2);

pub(super) fn in_game() -> DespawnOnExit<AppState> {
    DespawnOnExit(AppState::InGame)
}

fn static_box(key: &'static str, size: Vec3, transform: Transform, layer: Layer) -> impl Bundle {
    (
        Appearance::new(key, Silhouette::Cuboid { size }),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        layers(layer),
        transform,
        in_game(),
    )
}

/// 地形・橋・出現領域・奈落・Goal・カタパルトを生成する。
pub fn spawn_stage(commands: &mut Commands, stage: &Stage, difficulty: Difficulty) {
    let floor = stage.geometry();
    let floor_center = Vec3::new(0.0, floor.floor_y - FLOOR_THICKNESS / 2.0, 0.0);
    match floor.shape {
        Shape::Rectangle { width, depth } => {
            let size = Vec3::new(width.get(), FLOOR_THICKNESS, depth.get());
            commands.spawn(static_box(
                "stage/floor",
                size,
                Transform::from_translation(floor_center),
                Layer::Stage,
            ));
        },
        Shape::Circle { radius } => {
            commands.spawn((
                Appearance::new(
                    "stage/floor",
                    Silhouette::Cylinder { radius: radius.get(), height: FLOOR_THICKNESS },
                ),
                RigidBody::Static,
                Collider::cylinder(radius.get(), FLOOR_THICKNESS),
                layers(Layer::Stage),
                Transform::from_translation(floor_center),
                in_game(),
            ));
        },
    }

    for bridge in stage.bridges() {
        let span = bridge.exit - bridge.entry;
        let width = bridge.width.get();
        let rotation = Quat::from_rotation_arc(Vec3::X, span.normalize_or(Vec3::X));
        let center = (bridge.entry + bridge.exit) / 2.0 - Vec3::Y * FLOOR_THICKNESS / 2.0;
        commands.spawn(static_box(
            "stage/bridge",
            Vec3::new(span.length(), FLOOR_THICKNESS, width),
            Transform::from_translation(center).with_rotation(rotation),
            Layer::Stage,
        ));
        // 出現領域。敵とだけ衝突する床
        commands.spawn(static_box(
            "stage/spawn",
            Vec3::new(width * 2.0, FLOOR_THICKNESS, width * 2.0),
            Transform::from_translation(bridge.entry - Vec3::Y * FLOOR_THICKNESS / 2.0),
            Layer::SpawnFloor,
        ));
    }

    for abyss in stage.abysses() {
        let size = abyss.size();
        commands.spawn((
            AbyssZone,
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Sensor,
            CollisionEventsEnabled,
            layers(Layer::Abyss),
            Transform::from_translation(abyss.center()),
            in_game(),
        ));
    }

    for goal in stage.active_goals(difficulty) {
        let radius = goal.radius.get();
        commands.spawn((
            GoalZone(goal.id),
            Appearance::new("stage/goal", Silhouette::Cylinder { radius, height: 0.05 }),
            RigidBody::Static,
            Collider::cylinder(radius, GOAL_HEIGHT),
            Sensor,
            CollisionEventsEnabled,
            layers(Layer::Goal),
            Transform::from_translation(goal.position + Vec3::Y * GOAL_HEIGHT / 2.0),
            in_game(),
        ));
    }

    // カタパルトは表示のみ（射出したメダルと干渉させない）
    commands.spawn((
        Appearance::new("catapult", Silhouette::Cuboid { size: CATAPULT_SIZE }),
        Transform::from_translation(stage.catapult().position() - Vec3::Y * CATAPULT_SIZE.y),
        in_game(),
    ));
}

/// メダルの物理特性（定義の値に印の効き目を掛けたもの）。
pub struct MedalBody {
    pub mass: Mass,
    pub collider: Collider,
    pub friction: Friction,
    pub restitution: Restitution,
    pub layers: CollisionLayers,
    pub silhouette: Silhouette,
}

impl MedalBody {
    /// すり抜け（`ghost`）のメダルは他のメダルと衝突しない。
    pub fn new(spec: &MedalSpec, m: &Modifiers) -> Self {
        let (radius, thickness) =
            (spec.radius.get() * m.size.get(), spec.thickness.get() * m.size.get());
        let layers = if m.ghost {
            let base = layers(Layer::Medal);
            CollisionLayers::new(base.memberships, base.filters & !LayerMask::from(Layer::Medal))
        } else {
            layers(Layer::Medal)
        };
        Self {
            mass: Mass(spec.mass.get() * m.mass.get()),
            collider: Collider::cylinder(radius, thickness),
            friction: Friction::new(m.friction.unwrap_or(spec.friction)),
            restitution: Restitution::new(spec.restitution),
            layers,
            silhouette: Silhouette::Cylinder { radius, height: thickness },
        }
    }
}

/// メダル1枚。`motion` は射出なら飛行中、置いたなら静止から始める。
pub fn medal_bundle(
    unit: MedalUnit,
    spec: &MedalSpec,
    modifiers: &Modifiers,
    transform: Transform,
    motion: MedalMotion,
) -> impl Bundle {
    let body = MedalBody::new(spec, modifiers);
    (
        Appearance::new(Appearance::medal_key(unit.medal()), body.silhouette),
        Medal(unit),
        MedalMotionState(motion),
        AbilityTimers::default(),
        RigidBody::Dynamic,
        body.collider,
        body.mass,
        body.friction,
        body.restitution,
        body.layers,
        transform,
        in_game(),
    )
}

pub fn spawn_initial_medals(
    commands: &mut Commands,
    stage: &Stage,
    level: MedalLevel,
    spec: &MedalSpec,
) {
    for p in stage
        .initial_medals()
        .iter()
        .flat_map(|placement| placement.positions())
    {
        commands.spawn(medal_bundle(
            MedalUnit::new(MedalCode::normal(), level),
            spec,
            &Modifiers::NONE,
            Transform::from_translation(p),
            MedalMotion::placed(),
        ));
    }
}
pub fn spawn_enemy(
    commands: &mut Commands,
    id: EnemyId,
    key: String,
    spec: &EnemySpec,
    route: Route,
) {
    let (radius, height) = (spec.radius.get(), spec.height.get());
    let length = (height - radius * 2.0).max(0.0);
    let start = route.start() + Vec3::Y * height / 2.0;
    commands.spawn((
        Enemy { id },
        EnemyMarks::default(),
        Navigation(route),
        Walker {
            max_speed: spec.max_speed,
            walk_force: spec.walk_force,
        },
        Appearance::new(key, Silhouette::Capsule { radius, length }),
        RigidBody::Dynamic,
        Collider::capsule(radius, length),
        Mass(spec.mass.get()),
        Friction::new(spec.friction),
        LockedAxes::ROTATION_LOCKED,
        layers(Layer::Enemy),
        Transform::from_translation(start),
        in_game(),
    ));
}

/// Pusherは押し出し面がRailの進行方向を向くよう配置し、回転と高さを固定する。
pub fn spawn_pusher(
    commands: &mut Commands,
    stage: &Stage,
    character: &PusherCharacterId,
    setup: &PusherSetup,
) {
    let stats = setup.stats;
    let size = Vec3::new(PUSHER_THICKNESS, PUSHER_HEIGHT, stats.push_width.get());
    let heading = stage
        .rail(setup.rail)
        .and_then(|rail| rail.points().windows(2).next().map(|w| w[1] - w[0]))
        .map(|d| Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::X))
        .unwrap_or(Vec3::X);
    let floor_y = stage.geometry().floor_y;
    let position = Vec3::new(
        setup.position.x,
        floor_y + PUSHER_HEIGHT / 2.0 + 0.02,
        setup.position.z,
    );
    commands.spawn((
        Pusher { stats },
        PusherMotion { rail: setup.rail, state: setup.motion },
        Appearance::new(Appearance::pusher_key(character), Silhouette::Cuboid { size }),
        RigidBody::Dynamic,
        Collider::cuboid(size.x, size.y, size.z),
        Mass(stats.mass.get()),
        Friction::new(0.2),
        LockedAxes::ROTATION_LOCKED.lock_translation_y(),
        layers(Layer::Pusher),
        Transform::from_translation(position)
            .with_rotation(Quat::from_rotation_arc(Vec3::X, heading)),
        in_game(),
    ));
}
