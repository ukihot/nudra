//! メダルの能力の ECS 側（基本設計 3.14〜3.18）。
//!
//! - 収集（通常のSystem・並列Query）: 衝突・挙動・時限の出来事を [`AbilityEvents`]
//!   に積む。並べ替えて順序を固定する
//! - 確定（排他的System）: 出来事ごとに、能力1つずつ 「計画 → 対象の検索と適用前の検証 →
//!   承認（鮮度と予算） → Domainの確定 → Worldへの直接反映 → 通知」を完了させてから次へ進む。
//!   ループ内では `Commands` を使わない。次の能力は直前の生成・退場を反映したWorldを検索する
//! - 承認の後に失敗し得る処理を残さない。それでも失敗したら [`SessionAbort`]
//!   でセッションを安全に中断する（基本設計 8.5）
//!
//! ルール（発動の可否・印・反応・生成・世代・予算）は application / domain
//! が持ち、ここは物理と範囲の検索だけを行う。

use std::collections::HashSet;

use application::{
    AbilityOrder, Budget, EffectOrder, Occasion, PhysicsFact, approve_ability, commit_ability,
    expire_marks, hit_target, need_of, plan_ability, stand_in_zone,
};
use avian3d::prelude::*;
use bevy::{
    ecs::{entity::EntityHashSet, query::QueryData},
    prelude::*,
    utils::Parallel,
};
use domain::{
    ability::{Area, Direction, Effect, Happening, Partner, SpawnAt, Target, Trigger},
    firing::triggered,
    motion::{MedalMotion, Motion, MotionRules, MotionSample},
    spec::{MedalSpec, SpecCatalog},
    stage::{Shape, Stage},
    time::{Tick, Ticks},
    unit::{MedalExitReason, MedalUnit},
};

use super::{
    data::Specs,
    physics::PendingFacts,
    session::{ActiveGame, GameClock},
    world::{Enemy, EnemyMarks, Medal, MedalBody, in_game, medal_bundle},
};
use crate::port::{AbilityCue, Appearance, DomainEventMsg, MedalMotionState, Silhouette};

// ── 調整ノブ ──────────────────────────────────────────────

/// 上空から降らせる高さ（m）。
const SKY_HEIGHT: f32 = 6.0;
/// その場に戻す・生成するときの、盤面からの高さ（m）。
const SAFE_HEIGHT: f32 = 1.0;
/// 複数枚を生成するときの、1枚ごとのずらし（m）。
const SPAWN_SPACING: f32 = 0.35;
/// 場の表示の厚さ（m）。
const ZONE_HEIGHT: f32 = 0.04;

// ── Resource / Component ─────────────────────────────────

/// メダルに起きた出来事（収集した文脈つき）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbilityEvent {
    pub entity: Entity,
    pub happening: Happening,
    pub at: Vec3,
    pub forward: Vec3,
    /// 衝突の相手（範囲 `Touched` の対象）
    pub partner: Option<Entity>,
}

impl AbilityEvent {
    /// 確定の順序（基本設計 6.2）: 盤面に出た → 射出 → 衝突 → 挙動 → 時限。同じ種類はEntity順。
    fn order_key(&self) -> (u8, Entity) {
        let kind = match self.happening {
            Happening::Entered => 0,
            Happening::Launched => 1,
            Happening::Collided(_) => 2,
            Happening::Motion(_) => 3,
            Happening::Due(_) => 4,
            Happening::SelfFell | Happening::OtherFell => 5,
        };
        (kind, self.entity)
    }
}

/// このtickに収集した出来事。
#[derive(Resource, Debug, Default)]
pub struct AbilityEvents(pub Vec<AbilityEvent>);

/// 次のtickに処理する出来事（生成した子の `Entered`）。予算の待ち行列の枠を予約してから積む。
#[derive(Resource, Debug, Default)]
pub struct NextTickEvents(pub Vec<AbilityEvent>);

/// 能力の実行時予算（基本設計 3.18）。セッション開始時に初期配置の枚数で作る。
#[derive(Resource, Debug, Deref, DerefMut)]
pub struct AbilityBudget(pub Budget);

/// 承認の後に起きた予期しない失敗。立ったらゲーム進行を止め、勝敗を確定せずに選択へ戻す（基本設計
/// 8.5）。
#[derive(Resource, Debug, Default)]
pub struct SessionAbort(pub Option<String>);

/// このtickに奈落へ落ちたメダル（物理の収集が積む）。
#[derive(Resource, Debug, Default)]
pub struct FallenMedals(pub EntityHashSet);

/// このtickに撃破が確定した敵の数（`OtherFell` の通知に使う）。
#[derive(Resource, Debug, Default)]
pub struct DefeatedEnemies(pub usize);

/// このtickに盤面へ触れた、飛行中のメダル（着地の判定に使う）。
#[derive(Resource, Debug, Default)]
pub struct Touching(EntityHashSet);

/// 時限・間隔の能力の予定（個体ごとに期限の Tick を持つ）。
#[derive(Component, Debug, Clone, Default)]
pub struct AbilityTimers(Vec<Due>);

#[derive(Debug, Clone, Copy)]
struct Due {
    index: usize,
    at: Tick,
    every: Option<Ticks>,
}

impl AbilityTimers {
    pub fn new(specs: &SpecCatalog, unit: &MedalUnit, now: Tick) -> Self {
        Self(
            specs
                .abilities()
                .timers(unit.medal(), unit.level())
                .map(|(index, after, every)| Due { index, at: now + after, every })
                .collect(),
        )
    }

    /// 進化で増えた能力（覚醒）の予定だけを足す。既存の予定は期限も処理済みも変えない。
    pub fn upgrade(
        &mut self,
        old_count: usize,
        timers: impl IntoIterator<Item = (usize, Ticks, Option<Ticks>)>,
        now: Tick,
    ) {
        self.0.extend(
            timers
                .into_iter()
                .filter(|(index, ..)| *index >= old_count)
                .map(|(index, after, every)| Due { index, at: now + after, every }),
        );
    }
}

/// 場。中にいる対象へ印を付ける（基本設計 3.14）。
#[derive(Component, Debug, Clone)]
pub struct ZoneField {
    effect: Effect,
    target: Target,
    radius: f32,
    until: Tick,
}

/// メダルの物理特性を、種類・レベル・印から作り直すためのまとまり（通常のSystem用）。
#[derive(QueryData)]
#[query_data(mutable)]
pub struct MedalBodyData {
    entity: Entity,
    pub medal: &'static mut Medal,
    mass: &'static mut Mass,
    collider: &'static mut Collider,
    friction: &'static mut Friction,
    restitution: &'static mut Restitution,
    /// 変更不可の Component のため、変わったときだけ `Commands` で差し替える
    layers: &'static CollisionLayers,
    appearance: &'static mut Appearance,
}

impl MedalBodyDataItem<'_, '_> {
    /// 定義の値に印の効き目を掛けた物理特性と見た目を反映する（変わったときに1回だけ呼ぶ）。
    pub fn refresh(&mut self, specs: &SpecCatalog, commands: &mut Commands) {
        let Some(body) = body_of(specs, &self.medal.0) else {
            return;
        };
        let key = Appearance::medal_key(self.medal.0.medal());
        *self.mass = body.mass;
        *self.collider = body.collider;
        *self.friction = body.friction;
        *self.restitution = body.restitution;
        if *self.layers != body.layers {
            commands.entity(self.entity).insert(body.layers);
        }
        if self.appearance.key != key {
            self.appearance.key = key.into();
        }
        self.appearance.silhouette = body.silhouette;
    }
}

fn body_of(specs: &SpecCatalog, unit: &MedalUnit) -> Option<MedalBody> {
    let Some(spec) = specs.medal(unit.medal(), unit.level()) else {
        warn!(medal = %unit.medal(), "medal spec is missing");
        return None;
    };
    Some(MedalBody::new(spec, &unit.modifiers(specs.abilities())))
}

/// 排他的System用: 物理特性と見た目をWorldへ直接反映する。
fn refresh_in_world(world: &mut World, entity: Entity, specs: &SpecCatalog) {
    let Some((body, key)) = world
        .get::<Medal>(entity)
        .and_then(|m| body_of(specs, &m.0).map(|b| (b, Appearance::medal_key(m.0.medal()))))
    else {
        return;
    };
    let mut e = world.entity_mut(entity);
    if let Some(mut mass) = e.get_mut::<Mass>() {
        *mass = body.mass;
    }
    if let Some(mut collider) = e.get_mut::<Collider>() {
        *collider = body.collider;
    }
    if let Some(mut friction) = e.get_mut::<Friction>() {
        *friction = body.friction;
    }
    if e.get::<CollisionLayers>() != Some(&body.layers) {
        e.insert(body.layers);
    }
    if let Some(mut appearance) = e.get_mut::<Appearance>() {
        if appearance.key != key {
            appearance.key = key.into();
        }
        appearance.silhouette = body.silhouette;
    }
}

// ── 位置と範囲 ────────────────────────────────────────────

/// 速度の向き（止まっていれば奥）。前方の範囲と押す向きの基準。
fn forward(velocity: Vec3) -> Vec3 {
    Vec3::new(velocity.x, 0.0, velocity.z).normalize_or(Vec3::NEG_Z)
}

/// カタパルトは横からメダルを転がす（基本設計 5.8）。水平に動いていれば、縁で立てて
/// （円柱の軸を進行方向の横へ）滑らずに転がる角速度を与える。止まっていれば寝かせたまま。
fn rolling(velocity: Vec3, radius: f32) -> (Quat, Vec3) {
    let flat = Vec3::new(velocity.x, 0.0, velocity.z);
    match flat.try_normalize() {
        Some(dir) => (
            Quat::from_rotation_arc(Vec3::Y, Vec3::Y.cross(dir)),
            Vec3::Y.cross(flat) / radius,
        ),
        None => (Quat::IDENTITY, Vec3::ZERO),
    }
}

/// 生成するメダルの描画用Transformと物理の初期状態（位置・姿勢・速度）。
/// Avianが古いTransformから戻さないように両方をそろえる。
fn entry(
    position: Vec3,
    velocity: Vec3,
    spec: &MedalSpec,
) -> (Transform, (Position, Rotation, LinearVelocity, AngularVelocity)) {
    let (rotation, spin) = rolling(velocity, spec.radius.get());
    (
        Transform::from_translation(position).with_rotation(rotation),
        (
            Position(position),
            Rotation(rotation),
            LinearVelocity(velocity),
            AngularVelocity(spin),
        ),
    )
}

/// 水平位置が盤面の上か（盤面の中心は原点）。
fn on_board(stage: &Stage, p: Vec3) -> bool {
    match stage.geometry().shape {
        Shape::Rectangle { width, depth } => {
            p.x.abs() <= width.get() / 2.0 && p.z.abs() <= depth.get() / 2.0
        },
        Shape::Circle { radius } => Vec2::new(p.x, p.z).length() <= radius.get(),
    }
}

/// 生成・復活の位置。`Here` は水平位置の真上の安全な高さ、`Sky` は上空。
/// 盤面の外（奈落の上など、足場のない位置）ならカタパルトへ戻し、今の向き `yaw` で転がし出す
/// （基本設計 3.18。カタパルトの口は盤面の外にあるので、止めて置くと奈落へ落ちる）。
/// 戻り値は（位置, 速度）。
fn spawn_point(at: SpawnAt, here: Vec3, stage: &Stage, yaw: f32) -> (Vec3, Vec3) {
    let floor = stage.geometry().floor_y;
    match at {
        SpawnAt::Here if on_board(stage, here) => {
            (Vec3::new(here.x, floor + SAFE_HEIGHT, here.z), Vec3::ZERO)
        },
        SpawnAt::Sky if on_board(stage, here) => {
            (Vec3::new(here.x, floor + SKY_HEIGHT, here.z), Vec3::ZERO)
        },
        _ => (stage.catapult().position(), stage.catapult().launch_velocity(yaw)),
    }
}

fn in_area(area: Area, at: Vec3, forward: Vec3, p: Vec3) -> bool {
    match area {
        Area::Touched => false,
        Area::Radius(r) => p.distance_squared(at) <= r.get() * r.get(),
        Area::Front { length, width } => {
            let d = p - at;
            let along = d.dot(forward);
            (0.0..=length.get()).contains(&along)
                && (d - forward * along).length() <= width.get() / 2.0
        },
        Area::Board => true,
    }
}

/// 押す向き（大きさ1）。
fn push_direction(dir: Direction, at: Vec3, forward: Vec3, p: Vec3) -> Vec3 {
    let outward = Vec3::new(p.x - at.x, 0.0, p.z - at.z).normalize_or(forward);
    match dir {
        Direction::Outward => outward,
        Direction::Inward => -outward,
        Direction::Forward => forward,
        Direction::Backward => -forward,
        Direction::Up => Vec3::Y,
        Direction::Swirl => Vec3::Y.cross(outward),
        // ponytail: 奈落の形が未決のため、盤面の中心から外へ押す。
        // 形が決まったら最寄りの奈落へ向ける
        Direction::TowardAbyss => Vec3::new(p.x, 0.0, p.z).normalize_or(outward),
    }
}

/// 衝突の能力を持つか（持たないメダルは、着地した後は衝突の出来事を受け取らない。
/// 数千枚の性能のため）。
fn listens_to_collisions(specs: &SpecCatalog, unit: &MedalUnit) -> bool {
    specs
        .abilities()
        .abilities(unit.medal(), unit.level())
        .iter()
        .any(|a| matches!(a.on, Trigger::Collide(_)))
}

/// 衝突の受信を、能力構成と挙動に合わせて見直す（変化・進化の後。基本設計 5.4）。
pub(super) fn resubscribe(
    commands: &mut Commands,
    entity: Entity,
    specs: &SpecCatalog,
    unit: &MedalUnit,
    motion: Motion,
) {
    if listens_to_collisions(specs, unit) || motion == Motion::Flying {
        commands.entity(entity).insert(CollisionEventsEnabled);
    } else {
        commands
            .entity(entity)
            .remove::<CollisionEventsEnabled>();
    }
}

// ── 盤面に出す（能力の確定の外。射出・テスト） ───────────────

/// メダルを宙に出し、`happenings`（射出なら `Entered` と `Launched`）を今tickの出来事に積む。
/// 盤面の生存数の予約は呼び出し側が済ませる（射出: `Budget::reserve_launch`）。
#[allow(clippy::too_many_arguments)]
pub fn spawn_medal(
    commands: &mut Commands,
    specs: &SpecCatalog,
    unit: MedalUnit,
    position: Vec3,
    velocity: Vec3,
    happenings: &[Happening],
    now: Tick,
    events: &mut AbilityEvents,
) -> Option<Entity> {
    let spec = specs.medal(unit.medal(), unit.level()).copied()?;
    let timers = AbilityTimers::new(specs, &unit, now);
    let modifiers = unit.modifiers(specs.abilities());
    let (transform, physics) = entry(position, velocity, &spec);
    let entity = commands
        .spawn((
            medal_bundle(unit, &spec, &modifiers, transform, MedalMotion::launched()),
            physics,
            CollisionEventsEnabled,
        ))
        .id();
    // 予定は既定（空）を上書きする
    commands.entity(entity).insert(timers);
    events
        .0
        .extend(happenings.iter().map(|&happening| AbilityEvent {
            entity,
            happening,
            at: position,
            forward: forward(velocity),
            partner: None,
        }));
    Some(entity)
}

/// 排他的System内での生成（能力の生成・分割）。位置・速度を明示し、子の `Entered` は次tickへ。
fn spawn_child(
    world: &mut World,
    specs: &SpecCatalog,
    unit: MedalUnit,
    (position, velocity): (Vec3, Vec3),
    now: Tick,
    next: &mut Vec<AbilityEvent>,
) {
    let Some(spec) = specs.medal(unit.medal(), unit.level()).copied() else {
        return;
    };
    let timers = AbilityTimers::new(specs, &unit, now);
    let modifiers = unit.modifiers(specs.abilities());
    let (transform, physics) = entry(position, velocity, &spec);
    let entity = world
        .spawn((
            medal_bundle(unit, &spec, &modifiers, transform, MedalMotion::launched()),
            physics,
            CollisionEventsEnabled,
        ))
        .id();
    world.entity_mut(entity).insert(timers);
    next.push(AbilityEvent {
        entity,
        happening: Happening::Entered,
        at: position,
        forward: Vec3::NEG_Z,
        partner: None,
    });
}

// ── 1能力の確定 ───────────────────────────────────────────

/// 確定の結果。
enum Committed {
    /// 適用まで完了した。`revive` は復活の位置
    Applied { revive: Option<SpawnAt> },
    /// 計画・検証・承認で成り立たなかった（何も変えていない）
    Rejected,
}

/// 範囲内の対象（メダル・敵）。同tickに退場が確定した個体は除く。Entity順。
fn targets(
    world: &mut World,
    order: &AbilityOrder,
    ev: &AbilityEvent,
    gone: &EntityHashSet,
) -> (Vec<Entity>, Vec<Entity>) {
    let wants = |medal: bool| match order.target {
        Target::Itself => false,
        Target::Medals => medal,
        Target::Enemies => !medal,
        Target::Both => true,
    };
    let alive = |e: &Entity| !gone.contains(e);
    if order.target == Target::Itself || order.area == Area::Touched {
        let one = if order.target == Target::Itself {
            Some(ev.entity)
        } else {
            ev.partner
        };
        let Some(one) = one.filter(alive) else {
            return (Vec::new(), Vec::new());
        };
        let is_medal = world.get::<Medal>(one).is_some();
        let is_enemy = world.get::<Enemy>(one).is_some();
        return match (is_medal, is_enemy) {
            (true, _) if order.target == Target::Itself || wants(true) => (vec![one], Vec::new()),
            (_, true) if wants(false) => (Vec::new(), vec![one]),
            _ => (Vec::new(), Vec::new()),
        };
    }
    let hit = |p: Vec3| in_area(order.area, ev.at, ev.forward, p);
    let medals = if wants(true) {
        let mut q = world.query_filtered::<(Entity, &Position), With<Medal>>();
        let mut found: Vec<Entity> = q
            .iter(world)
            .filter(|(e, p)| *e != ev.entity && alive(e) && hit(p.0))
            .map(|(e, _)| e)
            .collect();
        found.sort_unstable();
        found
    } else {
        Vec::new()
    };
    let enemies = if wants(false) {
        let mut q = world.query_filtered::<(Entity, &Position), (With<Enemy>, Without<Medal>)>();
        let mut found: Vec<Entity> = q
            .iter(world)
            .filter(|(e, p)| alive(e) && hit(p.0))
            .map(|(e, _)| e)
            .collect();
        found.sort_unstable();
        found
    } else {
        Vec::new()
    };
    (medals, enemies)
}

/// 押す（速度へ直接。質量で割る）。上へ押されたメダルは飛行中に戻り、着地でまた発動する。
fn push(world: &mut World, target: Entity, dir: Direction, strength: f32, ev: &AbilityEvent) {
    let Some(position) = world.get::<Position>(target).map(|p| p.0) else {
        return;
    };
    let mass = world
        .get::<Mass>(target)
        .map_or(1.0, |m| m.0.max(f32::EPSILON));
    if let Some(mut velocity) = world.get_mut::<LinearVelocity>(target) {
        velocity.0 += push_direction(dir, ev.at, ev.forward, position) * strength / mass;
    }
    if dir == Direction::Up && world.get::<Medal>(target).is_some() {
        if let Some(mut motion) = world.get_mut::<MedalMotionState>(target) {
            motion.0.lift();
        }
        world.entity_mut(target).insert(CollisionEventsEnabled);
    }
}

/// 対象のメダル1枚に、状態の効果と動作を当てる。
#[allow(clippy::too_many_arguments)]
fn apply_to_medal(
    world: &mut World,
    specs: &SpecCatalog,
    budget: &mut Budget,
    order: &AbilityOrder,
    ev: &AbilityEvent,
    target: Entity,
    now: Tick,
    gone: &mut EntityHashSet,
    next: &mut Vec<AbilityEvent>,
) {
    let mut reactions = Vec::new();
    if !order.states.is_empty() {
        let Some(out) = world
            .get_mut::<Medal>(target)
            .map(|mut medal| hit_target(specs, medal.0.conditions_mut(), &order.states, now))
        else {
            return;
        };
        if out.modifiers.is_some() {
            refresh_in_world(world, target, specs);
        }
        reactions = out.reactions;
    }
    let actions = order
        .actions
        .iter()
        .cloned()
        .chain(reactions.into_iter().map(EffectOrder::Apply));
    for action in actions {
        if gone.contains(&target) {
            return;
        }
        match action {
            EffectOrder::Apply(Effect::Push { dir, strength }) => {
                push(world, target, dir, strength.get(), ev);
            },
            EffectOrder::Apply(Effect::Remove) => {
                // 能力による消滅（DestroyedByAbility）。報酬・落下の通知なし
                if world.despawn(target) {
                    budget.medal_left();
                    gone.insert(target);
                }
            },
            EffectOrder::Apply(Effect::Split) => {
                let half = world.get::<Medal>(target).map(|m| m.0.split_off());
                let position = world.get::<Position>(target).map(|p| p.0);
                if let (Some(half), Some(position)) = (half, position) {
                    let at = (position + Vec3::Y * 0.2, Vec3::ZERO);
                    spawn_child(world, specs, half, at, now, next);
                }
            },
            EffectOrder::Transform { medal, level } => {
                let listens = {
                    let Some(mut m) = world.get_mut::<Medal>(target) else {
                        return;
                    };
                    m.0.transform(medal, level);
                    listens_to_collisions(specs, &m.0)
                };
                refresh_in_world(world, target, specs);
                let flying = world
                    .get::<MedalMotionState>(target)
                    .is_some_and(|m| m.0.state() == Motion::Flying);
                if listens || flying {
                    world.entity_mut(target).insert(CollisionEventsEnabled);
                } else {
                    world
                        .entity_mut(target)
                        .remove::<CollisionEventsEnabled>();
                }
            },
            _ => {},
        }
    }
}

/// 対象の敵1体に、状態の効果と押す動作を当てる（敵を消す効果は読込時に拒否済み）。
fn apply_to_enemy(
    world: &mut World,
    specs: &SpecCatalog,
    order: &AbilityOrder,
    ev: &AbilityEvent,
    target: Entity,
    now: Tick,
) {
    let mut reactions = Vec::new();
    if !order.states.is_empty() {
        let Some(mut marks) = world.get_mut::<EnemyMarks>(target) else {
            return;
        };
        let out = hit_target(specs, &mut marks.conditions, &order.states, now);
        if let Some(modifiers) = out.modifiers {
            marks.modifiers = modifiers;
        }
        reactions = out.reactions;
    }
    let pushes: Vec<_> = order
        .actions
        .iter()
        .filter_map(|a| match a {
            EffectOrder::Apply(Effect::Push { dir, strength }) => Some((*dir, *strength)),
            _ => None,
        })
        .chain(reactions.iter().filter_map(|e| match e {
            Effect::Push { dir, strength } => Some((*dir, *strength)),
            _ => None,
        }))
        .collect();
    for (dir, strength) in pushes {
        push(world, target, dir, strength.get(), ev);
    }
}

/// 能力1つを、計画から適用まで直列に完了させる。`Err`
/// は承認後の予期しない失敗（セッションを中断する）。
#[allow(clippy::too_many_arguments)]
fn try_commit_one(
    world: &mut World,
    specs: &SpecCatalog,
    game: &mut ActiveGame,
    budget: &mut Budget,
    ev: &AbilityEvent,
    index: usize,
    gone: &mut EntityHashSet,
    next: &mut Vec<AbilityEvent>,
) -> Result<Committed, String> {
    // 1. 計画（何も変えない）
    let Some(unit) = world.get::<Medal>(ev.entity).map(|m| m.0.clone()) else {
        return Ok(Committed::Rejected);
    };
    let occasion = Occasion {
        happening: ev.happening,
        partner: ev.partner.is_some(),
    };
    let Ok(planned) = plan_ability(&game.session, specs, &unit, index, occasion) else {
        return Ok(Committed::Rejected);
    };
    // 2. 対象の検索と適用前の検証（直前の能力の結果を反映したWorldを検索する）
    let mut found = Vec::with_capacity(planned.orders.len());
    for order in &planned.orders {
        let (medals, enemies) = targets(world, order, ev, gone);
        // 分割は対象の世代で検証する（世代の上限の個体は分割できない。能力全体を不成立にする）
        if order.splits() > 0
            && medals
                .iter()
                .any(|&m| world.get::<Medal>(m).is_some_and(|u| !u.0.can_spawn()))
        {
            return Ok(Committed::Rejected);
        }
        found.push((medals, enemies));
    }
    // 対象の要る効果しかなく、
    // どの指示にも対象がいなければ不成立（単独で成り立つ効果があれば実行する）
    let lone = planned.gauge > 0
        || planned.revive.is_some()
        || planned
            .orders
            .iter()
            .flat_map(|o| &o.actions)
            .any(|a| {
                matches!(
                    a,
                    EffectOrder::Spawn { .. }
                        | EffectOrder::Apply(Effect::Zone { .. } | Effect::Consume)
                )
            });
    if !lone && found.iter().all(|(m, e)| m.is_empty() && e.is_empty()) {
        return Ok(Committed::Rejected);
    }
    let counts: Vec<u32> = found
        .iter()
        .map(|(m, e)| u32::try_from(m.len() + e.len()).unwrap_or(u32::MAX))
        .collect();
    // 3. 承認（鮮度の確認と予算の予約。どちらかが足りなければ何も変えない）
    let need = need_of(&planned, &counts);
    let Ok(approved) = approve_ability(&game.session, &unit, planned, budget, need) else {
        return Ok(Committed::Rejected);
    };
    let orders = approved.plan().orders.clone();
    let source = approved.plan().source.clone();
    let revive = approved.plan().revive;
    // 4. Domainの確定（ここから先に失敗し得る検証を残さない）
    let events = {
        let mut medal = world
            .get_mut::<Medal>(ev.entity)
            .ok_or_else(|| format!("{:?} vanished during an approved ability", ev.entity))?;
        commit_ability(&mut game.session, specs, &mut medal.0, approved)
            .map_err(|stale| format!("an approved ability became stale: {stale}"))?
    };
    // 5. Worldへの直接反映
    let now = world.resource::<GameClock>().now();
    for (order, (medals, enemies)) in orders.iter().zip(found) {
        for action in &order.actions {
            match action {
                EffectOrder::Apply(effect @ Effect::Zone { mark, radius, duration, .. }) => {
                    world.spawn((
                        ZoneField {
                            effect: effect.clone(),
                            target: order.target,
                            radius: radius.get(),
                            until: now + *duration,
                        },
                        Appearance::new(
                            format!("zone/{mark}"),
                            Silhouette::Cylinder { radius: radius.get(), height: ZONE_HEIGHT },
                        ),
                        Transform::from_translation(ev.at),
                        in_game(),
                    ));
                },
                EffectOrder::Spawn { medal, level, count, at, mark, rewarded } => {
                    for i in 0..count.get() {
                        let mut child = unit.offspring(medal.clone(), *level, *rewarded);
                        if let Some(def) = mark.as_ref().and_then(|m| specs.abilities().mark(m)) {
                            child.conditions_mut().apply(def, None);
                        }
                        let offset = Vec3::X * SPAWN_SPACING * f32::from(i);
                        let (position, velocity) =
                            spawn_point(*at, ev.at, &game.stage, game.catapult.yaw());
                        spawn_child(world, specs, child, (position + offset, velocity), now, next);
                    }
                },
                _ => {},
            }
        }
        for target in medals {
            apply_to_medal(world, specs, budget, order, ev, target, now, gone, next);
        }
        for target in enemies {
            apply_to_enemy(world, specs, order, ev, target, now);
        }
    }
    // 自分を消費する能力（爆弾）:
    // 効果を適用し終えてから、同じ確定単位で退場する（Consumed。報酬・落下の通知なし）
    let consumes = orders
        .iter()
        .flat_map(|o| &o.actions)
        .any(|a| matches!(a, EffectOrder::Apply(effect) if effect.consumes_source()));
    if consumes && !gone.contains(&ev.entity) && world.despawn(ev.entity) {
        budget.medal_left();
        gone.insert(ev.entity);
    }
    // 6. 通知（完了した効果だけ）
    for event in events {
        world.write_message(DomainEventMsg(event));
    }
    world.write_message(AbilityCue {
        medal: source.medal,
        index: source.index,
        at: ev.at,
    });
    Ok(Committed::Applied { revive })
}

/// 出来事の能力を順に確定する。進行できなくなったら（満タン・終局）残りを確定しない。
#[allow(clippy::too_many_arguments)]
fn resolve_events(
    world: &mut World,
    specs: &SpecCatalog,
    game: &mut ActiveGame,
    budget: &mut Budget,
    events: &[AbilityEvent],
    gone: &mut EntityHashSet,
    next: &mut Vec<AbilityEvent>,
) -> Result<(), String> {
    for ev in events {
        if gone.contains(&ev.entity) {
            continue;
        }
        let Some(unit) = world.get::<Medal>(ev.entity).map(|m| m.0.clone()) else {
            continue;
        };
        let indices: Vec<usize> = triggered(specs, &unit, ev.happening).collect();
        for index in indices {
            if game.session.ensure_running().is_err() {
                return Ok(());
            }
            try_commit_one(world, specs, game, budget, ev, index, gone, next)?;
        }
    }
    Ok(())
}

/// 排他的Systemの共通の入口: Specs・ActiveGame・予算を借りて `body`
/// を実行し、失敗したら中断を記録する。
fn with_session(
    world: &mut World,
    body: impl FnOnce(&mut World, &SpecCatalog, &mut ActiveGame, &mut Budget) -> Result<(), String>,
) {
    let result = world.resource_scope(|world, specs: Mut<Specs>| {
        world.resource_scope(|world, mut game: Mut<ActiveGame>| {
            world.resource_scope(|world, mut budget: Mut<AbilityBudget>| {
                body(world, &specs, &mut game, &mut budget)
            })
        })
    });
    if let Err(reason) = result {
        error!(%reason, "aborting the session after an unexpected ability failure");
        world.resource_mut::<SessionAbort>().0 = Some(reason);
    }
}

// ── 落下（Resolve。排他的System） ─────────────────────────

/// 落ちたメダル: 自分が落ちた能力（復活なら盤面へ戻す）を判定してから退場を確定し、
/// 報酬資格のある落下だけの価値をゲージの事実にする。復活しなかった個体ごとに `OtherFell`
/// を通知する。 必須の退場・報酬は予算や満タンで延期しない（基本設計 3.18）。
pub fn resolve_falls(world: &mut World) {
    let mut falling: Vec<Entity> = world
        .resource_mut::<FallenMedals>()
        .0
        .drain()
        .collect();
    if falling.is_empty() {
        return;
    }
    falling.sort_unstable();
    with_session(world, |world, specs, game, budget| {
        let book = specs.abilities();
        let (mut value, mut fell) = (0.0, 0usize);
        let mut gone = EntityHashSet::default();
        let mut next = Vec::new();
        for entity in falling {
            let Some((unit, at)) = world
                .get::<Medal>(entity)
                .map(|m| m.0.clone())
                .zip(world.get::<Position>(entity).map(|p| p.0))
            else {
                continue;
            };
            let ev = AbilityEvent {
                entity,
                happening: Happening::SelfFell,
                at,
                forward: Vec3::NEG_Z,
                partner: None,
            };
            let mut revive = None;
            for index in triggered(specs, &unit, Happening::SelfFell).collect::<Vec<_>>() {
                if let Committed::Applied { revive: Some(to) } =
                    try_commit_one(world, specs, game, budget, &ev, index, &mut gone, &mut next)?
                {
                    revive = Some(to);
                }
            }
            match revive {
                Some(to) => {
                    // 位置・姿勢・速度を描画用Transformとそろえて戻す
                    let (to, velocity) = spawn_point(to, at, &game.stage, game.catapult.yaw());
                    let spec = world
                        .get::<Medal>(entity)
                        .and_then(|m| specs.medal(m.0.medal(), m.0.level()).copied());
                    if let Some(spec) = spec {
                        let (transform, physics) = entry(to, velocity, &spec);
                        world.entity_mut(entity).insert((
                            transform,
                            physics,
                            CollisionEventsEnabled,
                        ));
                    }
                    if let Some(mut motion) = world.get_mut::<MedalMotionState>(entity) {
                        motion.0.lift();
                    }
                },
                None => {
                    value += world
                        .get::<Medal>(entity)
                        .map_or(0.0, |m| m.0.exit_reward(MedalExitReason::FellIntoAbyss, book));
                    world.despawn(entity);
                    budget.medal_left();
                    gone.insert(entity);
                    fell += 1;
                },
            }
        }
        if value > 0.0 {
            world
                .resource_mut::<PendingFacts>()
                .0
                .push(PhysicsFact::MedalsFell(value));
        }
        notify_other_fell(world, specs, game, budget, fell, &mut gone, &mut next)?;
        world.resource_mut::<NextTickEvents>().0.extend(next);
        Ok(())
    });
}

/// 敵の撃破が確定した数だけ、`OtherFell` を通知する（基本設計 4.3）。
pub fn enemy_falls(world: &mut World) {
    let times = std::mem::take(&mut world.resource_mut::<DefeatedEnemies>().0);
    if times == 0 {
        return;
    }
    with_session(world, |world, specs, game, budget| {
        let (mut gone, mut next) = (EntityHashSet::default(), Vec::new());
        notify_other_fell(world, specs, game, budget, times, &mut gone, &mut next)?;
        world.resource_mut::<NextTickEvents>().0.extend(next);
        Ok(())
    });
}

/// 退場した件数×購読能力を遅延列挙し、検査枠を1消費してから確定へ進める（基本設計 3.18）。
/// 購読者は通知を始める時点のスナップショット（Entity・能力番号順）。上限・
/// 満タンで残りを列挙しない。
#[allow(clippy::too_many_arguments)]
fn notify_other_fell(
    world: &mut World,
    specs: &SpecCatalog,
    game: &mut ActiveGame,
    budget: &mut Budget,
    times: usize,
    gone: &mut EntityHashSet,
    next: &mut Vec<AbilityEvent>,
) -> Result<(), String> {
    if times == 0 {
        return Ok(());
    }
    let book = specs.abilities();
    let mut q = world.query::<(Entity, &Medal, &Position, &LinearVelocity)>();
    let mut subscribers: Vec<(Entity, usize, Vec3, Vec3)> = q
        .iter(world)
        .filter(|(e, ..)| !gone.contains(e))
        .flat_map(|(e, m, p, v)| {
            book.triggered(m.0.medal(), m.0.level(), Happening::OtherFell)
                .map(move |(i, _)| (e, i, p.0, forward(v.0)))
        })
        .collect();
    subscribers.sort_unstable_by_key(|(e, i, ..)| (*e, *i));
    for _ in 0..times {
        for &(entity, index, at, forward) in &subscribers {
            if game.session.ensure_running().is_err() || !budget.check_other_fell() {
                return Ok(());
            }
            if gone.contains(&entity) {
                continue;
            }
            let ev = AbilityEvent {
                entity,
                happening: Happening::OtherFell,
                at,
                forward,
                partner: None,
            };
            try_commit_one(world, specs, game, budget, &ev, index, gone, next)?;
        }
    }
    Ok(())
}

// ── 支えの接触による着地の判定（基本設計 5.4、P1-5） ─────────

/// 支えのしきい値【仮決定】: 法線の上向き成分・接触の隙間（m）・離れる速さ（m/s）。
const SUPPORT_MIN_UP: f32 = 0.5;
const SUPPORT_MAX_GAP: f32 = 0.002;
const SUPPORT_MAX_SEPARATION: f32 = 0.2;

/// 相手から自分へ向かう法線 `normal`、接触点 `point`、自分の中心 `center`、めり込み
/// `penetration`（負は隙間）、 法線方向に離れていく相対速度 `separation`
/// の接触が、自分を下から支えているか。 壁・天井・遠い投機的な接触・跳ね上がり中は支えにしない。
fn supports(normal: Vec3, point: Vec3, center: Vec3, penetration: f32, separation: f32) -> bool {
    normal.dot(Vec3::Y) >= SUPPORT_MIN_UP
        && point.y < center.y
        && penetration >= -SUPPORT_MAX_GAP
        && separation <= SUPPORT_MAX_SEPARATION
}

/// 飛行中のメダルだけを調べ、継続中の接触から支えを見つける（接触開始を逃しても着地できる）。
pub fn observe_support(
    collisions: Collisions,
    medals: Query<(Entity, &Position, &LinearVelocity, &MedalMotionState), With<Medal>>,
    velocities: Query<&LinearVelocity>,
    sensors: Query<(), With<Sensor>>,
    mut touching: ResMut<Touching>,
) {
    touching.0.clear();
    for (entity, position, velocity, motion) in &medals {
        if motion.0.state() != Motion::Flying {
            continue;
        }
        let supported = collisions.collisions_with(entity).any(|pair| {
            let (other, sign) = if pair.collider1 == entity {
                (pair.collider2, -1.0)
            } else {
                (pair.collider1, 1.0)
            };
            if sensors.contains(other) {
                return false;
            }
            let other_velocity = velocities.get(other).map_or(Vec3::ZERO, |v| v.0);
            pair.manifolds.iter().any(|m| {
                let normal = m.normal * sign;
                let separation = (velocity.0 - other_velocity).dot(normal);
                m.points
                    .iter()
                    .any(|p| supports(normal, p.point, position.0, p.penetration, separation))
            })
        });
        if supported {
            touching.0.insert(entity);
        }
    }
}
// ── 出来事の収集（Abilities。通常のSystem） ────────────────

/// 衝突: メダル・敵とぶつかった出来事（接触開始）。
/// 着地の判定は支えの接触で別に行う（[`observe_support`]）。 同じ接触開始のペアはtick内で1回だけ。
/// 両側をそれぞれ判定する。
pub fn collisions(
    mut collisions: MessageReader<CollisionStart>,
    medals: Query<(&Position, &LinearVelocity), With<Medal>>,
    enemies: Query<(), With<Enemy>>,
    mut events: ResMut<AbilityEvents>,
    mut seen: Local<HashSet<(Entity, Entity)>>,
) {
    seen.clear();
    for c in collisions.read() {
        let pair = if c.collider1 < c.collider2 {
            (c.collider1, c.collider2)
        } else {
            (c.collider2, c.collider1)
        };
        if !seen.insert(pair) {
            continue;
        }
        for (me, other) in [(c.collider1, c.collider2), (c.collider2, c.collider1)] {
            let Ok((position, velocity)) = medals.get(me) else {
                continue;
            };
            let partner = if enemies.contains(other) {
                Partner::Enemy
            } else if medals.contains(other) {
                Partner::Medal
            } else {
                continue;
            };
            events.0.push(AbilityEvent {
                entity: me,
                happening: Happening::Collided(partner),
                at: position.0,
                forward: forward(velocity.0),
                partner: Some(other),
            });
        }
    }
}

/// 挙動ステータスの観測（数千枚を並列に）。状態が変わったメダルの出来事を積む。
pub fn observe_motion(
    mut medals: Query<(Entity, &mut MedalMotionState, &LinearVelocity, &Position)>,
    touching: Res<Touching>,
    clock: Res<GameClock>,
    mut changed: Local<Parallel<Vec<AbilityEvent>>>,
    mut events: ResMut<AbilityEvents>,
) {
    let now = clock.now();
    let rules = MotionRules::default();
    medals
        .par_iter_mut()
        .for_each(|(entity, mut motion, velocity, position)| {
            let sample = MotionSample {
                speed: velocity.length(),
                grounded: touching.0.contains(&entity),
            };
            // 状態が変わらない大多数のメダルで変更を立てないよう、写しで観測する
            let mut next = motion.0;
            if let Some(event) = next.observe(sample, now, &rules) {
                motion.0 = next;
                changed.scope(|v| {
                    v.push(AbilityEvent {
                        entity,
                        happening: Happening::Motion(event),
                        at: position.0,
                        forward: forward(velocity.0),
                        partner: None,
                    });
                });
            } else if next != motion.0 {
                motion.bypass_change_detection().0 = next;
            }
        });
    events.0.extend(changed.drain());
}

/// 時限・間隔の能力の期限。発動の成否にかかわらず、周期は次の本来の期限へ進め、時限は消化済みにする
/// （棄却された出来事を再試行しない。基本設計 3.18）。
pub fn run_timers(
    mut medals: Query<(Entity, &mut AbilityTimers, &Position, &LinearVelocity)>,
    clock: Res<GameClock>,
    mut events: ResMut<AbilityEvents>,
) {
    let now = clock.now();
    for (entity, mut timers, position, velocity) in &mut medals {
        if !timers.0.iter().any(|d| d.at <= now) {
            continue;
        }
        for due in timers.0.iter_mut().filter(|d| d.at <= now) {
            events.0.push(AbilityEvent {
                entity,
                happening: Happening::Due(due.index),
                at: position.0,
                forward: forward(velocity.0),
                partner: None,
            });
            if let Some(every) = due.every {
                due.at = now + every;
            }
        }
        timers.0.retain(|d| d.at > now);
    }
}

// ── 確定（Abilities。排他的System） ───────────────────────

/// このtickの出来事と、前のtickから回した子の `Entered` を、順序を固定して確定する。
pub fn resolve_abilities(world: &mut World) {
    let mut events = std::mem::take(&mut world.resource_mut::<AbilityEvents>().0);
    let queued = std::mem::take(&mut world.resource_mut::<NextTickEvents>().0);
    if events.is_empty() && queued.is_empty() {
        return;
    }
    let dequeued = u32::try_from(queued.len()).unwrap_or(u32::MAX);
    events.extend(queued);
    events.sort_by_key(AbilityEvent::order_key);
    with_session(world, |world, specs, game, budget| {
        budget.dequeued(dequeued);
        let (mut gone, mut next) = (EntityHashSet::default(), Vec::new());
        resolve_events(world, specs, game, budget, &events, &mut gone, &mut next)?;
        if game.session.ensure_running().is_err() {
            // 満タン・終局: 未確定の出来事と待ち行列を持ち越さない（基本設計 3.16）
            budget.dequeued(u32::try_from(next.len()).unwrap_or(u32::MAX));
        } else {
            world.resource_mut::<NextTickEvents>().0.extend(next);
        }
        Ok(())
    });
}

/// tickの始め: 予算のtickごとの枠を戻す。
pub fn begin_tick(mut budget: ResMut<AbilityBudget>) {
    budget.next_tick();
}

/// 敵の印と位置（場の対象の検索）。
type EnemyPositions<'w, 's> =
    Query<'w, 's, (&'static mut EnemyMarks, &'static Position), (With<Enemy>, Without<Medal>)>;

/// 場: 期限が来たら消し、中にいる対象へ印を付ける。各tickの場1つを適用単位とし、
/// 枠不足ならそのtickは省略する。
#[allow(clippy::too_many_arguments)]
pub fn zones(
    fields: Query<(Entity, &ZoneField, &Transform)>,
    mut medals: Query<(MedalBodyData, &Position), Without<Enemy>>,
    mut enemies: EnemyPositions,
    specs: Res<Specs>,
    clock: Res<GameClock>,
    mut budget: ResMut<AbilityBudget>,
    mut commands: Commands,
) {
    let now = clock.now();
    for (entity, field, transform) in &fields {
        if now >= field.until {
            commands.entity(entity).try_despawn();
            budget.zone_ended();
            continue;
        }
        let inside =
            |p: &Position| p.distance_squared(transform.translation) <= field.radius * field.radius;
        let wants_medals = matches!(field.target, Target::Medals | Target::Both);
        let wants_enemies = matches!(field.target, Target::Enemies | Target::Both);
        let count = medals
            .iter()
            .filter(|(_, p)| wants_medals && inside(p))
            .count()
            + enemies
                .iter()
                .filter(|(_, p)| wants_enemies && inside(p))
                .count();
        if count == 0
            || budget
                .reserve_effects(u32::try_from(count).unwrap_or(u32::MAX))
                .is_err()
        {
            continue;
        }
        if wants_medals {
            for (mut body, position) in &mut medals {
                if inside(position)
                    && stand_in_zone(&specs, body.medal.0.conditions_mut(), &field.effect, now)
                        .modifiers
                        .is_some()
                {
                    body.refresh(&specs, &mut commands);
                }
            }
        }
        if wants_enemies {
            for (mut marks, position) in &mut enemies {
                if inside(position)
                    && let Some(modifiers) =
                        stand_in_zone(&specs, &mut marks.conditions, &field.effect, now).modifiers
                {
                    marks.modifiers = modifiers;
                }
            }
        }
    }
}

/// 印の期限。持ち主が消える印（焼き尽くす・毒）なら消す（能力による消滅で、報酬なし）。
/// 予算で延期しない。
pub fn expire(
    mut medals: Query<(Entity, MedalBodyData), Without<Enemy>>,
    mut enemies: Query<&mut EnemyMarks, (With<Enemy>, Without<Medal>)>,
    specs: Res<Specs>,
    clock: Res<GameClock>,
    mut budget: ResMut<AbilityBudget>,
    mut commands: Commands,
) {
    let now = clock.now();
    for (entity, mut body) in &mut medals {
        if body.medal.0.conditions().is_empty() {
            continue;
        }
        let out = expire_marks(&specs, body.medal.0.conditions_mut(), now);
        if out.vanish {
            commands.entity(entity).try_despawn();
            budget.medal_left();
        } else if out.modifiers.is_some() {
            body.refresh(&specs, &mut commands);
        }
    }
    for mut marks in &mut enemies {
        if marks.conditions.is_empty() {
            continue;
        }
        if let Some(modifiers) = expire_marks(&specs, &mut marks.conditions, now).modifiers {
            marks.modifiers = modifiers;
        }
    }
}

/// 敵の歩行に掛ける印の効き目（速さ・進路のずれ）。
pub fn enemy_walk(marks: Option<&EnemyMarks>) -> domain::mark::Modifiers {
    marks.map_or(domain::mark::Modifiers::NONE, |m| m.modifiers)
}

#[cfg(test)]
mod tests {
    use domain::motion::MotionEvent;
    use rstest::rstest;

    use super::*;
    use crate::infrastructure::data::StageAsset;

    /// stage_01: 幅8m・奥行16mの長方形、床の高さ0、カタパルトは盤面の右外 (4.4, 0.85, -4.5)。
    fn stage() -> Stage {
        ron::from_str::<StageAsset>(include_str!("../../assets/data/stages/stage_01.stage.ron"))
            .expect("valid RON")
            .to_domain()
            .expect("valid stage")
    }

    /// 盤面の上なら止めて置く（`Some(位置)`）。外ならカタパルトの口から、
    /// 今の向きで転がし出す（`None`）。
    #[rstest]
    #[case::here_on_the_board(SpawnAt::Here, Vec3::new(1.0, -20.0, 2.0), Some(Vec3::new(1.0, SAFE_HEIGHT, 2.0)))]
    #[case::sky_on_the_board(SpawnAt::Sky, Vec3::new(-3.0, 4.0, -7.0), Some(Vec3::new(-3.0, SKY_HEIGHT, -7.0)))]
    #[case::here_off_the_board(SpawnAt::Here, Vec3::new(4.5, -20.0, 0.0), None)]
    #[case::sky_off_the_board(SpawnAt::Sky, Vec3::new(0.0, 0.0, 8.5), None)]
    #[case::catapult(SpawnAt::Catapult, Vec3::new(1.0, 0.0, 1.0), None)]
    fn spawn_points_stay_above_the_board(
        #[case] at: SpawnAt,
        #[case] here: Vec3,
        #[case] placed: Option<Vec3>,
    ) {
        let stage = stage();
        let yaw = 0.3;
        let c = stage.catapult();
        let expected = placed.map_or((c.position(), c.launch_velocity(yaw)), |p| (p, Vec3::ZERO));
        assert_eq!(spawn_point(at, here, &stage, yaw), expected);
    }

    /// カタパルトの口から転がし出したメダルは、盤面の内側へ向かう。
    #[rstest]
    #[case::near_end(-0.6)]
    #[case::center(0.0)]
    #[case::far_end(0.6)]
    fn the_catapult_rolls_medals_onto_the_board(#[case] yaw: f32) {
        let stage = stage();
        let (from, velocity) = spawn_point(SpawnAt::Catapult, Vec3::ZERO, &stage, yaw);
        let ahead = from + Vec3::new(velocity.x, 0.0, velocity.z).normalize() * 2.0;
        assert!(on_board(&stage, ahead), "{ahead} is on the board");
    }

    /// 転がるメダル: 軸は水平で進行方向に直交し、接地点が止まる（v = ω × 上向きの半径）。
    #[rstest]
    #[case::to_the_left(Vec3::new(-4.0, -0.5, 0.0))]
    #[case::diagonal(Vec3::new(-3.0, 0.0, 2.0))]
    fn a_rolling_medal_stands_on_its_edge_without_slipping(#[case] velocity: Vec3) {
        let radius = 0.28;
        let (rotation, angular) = rolling(velocity, radius);
        let axis = rotation * Vec3::Y;
        let flat = Vec3::new(velocity.x, 0.0, velocity.z);
        assert!(axis.y.abs() < 1e-5, "axis is horizontal: {axis}");
        assert!(axis.dot(flat).abs() < 1e-4, "axis is across the motion: {axis}");
        let contact = angular.cross(Vec3::Y * radius);
        assert!(contact.distance(flat) < 1e-4, "rolls without slipping: {contact}");
    }

    #[rstest]
    #[case::still(Vec3::ZERO)]
    #[case::straight_down(Vec3::new(0.0, -3.0, 0.0))]
    fn a_medal_without_horizontal_motion_lies_flat(#[case] velocity: Vec3) {
        assert_eq!(rolling(velocity, 0.28), (Quat::IDENTITY, Vec3::ZERO));
    }

    fn due(index: usize, at: u64, every: Option<u64>) -> (usize, Tick, Option<Ticks>) {
        (index, Tick(at), every.map(Ticks))
    }

    fn dues(timers: &AbilityTimers) -> Vec<(usize, Tick, Option<Ticks>)> {
        timers
            .0
            .iter()
            .map(|d| (d.index, d.at, d.every))
            .collect()
    }

    /// 能力0（周期）は次回期限100のまま、能力1（時限）は処理済みで戻さず、
    /// 覚醒の能力2だけを今から足す。
    #[rstest]
    fn evolving_keeps_existing_timers_and_adds_only_awakened_ones() {
        let mut timers =
            AbilityTimers(vec![Due { index: 0, at: Tick(100), every: Some(Ticks(30)) }]);
        let level_four = [
            (0, Ticks(30), Some(Ticks(30))),
            (1, Ticks(90), None),
            (2, Ticks(60), None),
        ];
        timers.upgrade(2, level_four, Tick(70));
        assert_eq!(dues(&timers), [due(0, 100, Some(30)), due(2, 130, None)]);
        timers.upgrade(3, level_four, Tick(80));
        assert_eq!(dues(&timers).len(), 2, "reapplying the same level adds nothing");
    }

    #[rstest]
    #[case::floor(Vec3::Y, Vec3::new(0.0, -0.04, 0.0), 0.0, 0.0, true)]
    #[case::gentle_slope(Vec3::new(0.7, 0.71, 0.0), Vec3::new(0.0, -0.04, 0.0), 0.0, 0.0, true)]
    #[case::wall(Vec3::X, Vec3::new(-0.28, 0.0, 0.0), 0.0, 0.0, false)]
    #[case::ceiling(Vec3::NEG_Y, Vec3::new(0.0, 0.04, 0.0), 0.0, 0.0, false)]
    #[case::point_above_the_center(Vec3::Y, Vec3::new(0.0, 0.01, 0.0), 0.0, 0.0, false)]
    #[case::speculative_gap(Vec3::Y, Vec3::new(0.0, -0.04, 0.0), -0.01, 0.0, false)]
    #[case::small_gap(Vec3::Y, Vec3::new(0.0, -0.04, 0.0), -0.001, 0.0, true)]
    #[case::bouncing_up(Vec3::Y, Vec3::new(0.0, -0.04, 0.0), 0.0, 1.0, false)]
    #[case::settling(Vec3::Y, Vec3::new(0.0, -0.04, 0.0), 0.0, 0.1, true)]
    fn only_contacts_from_below_support_a_medal(
        #[case] normal: Vec3,
        #[case] point: Vec3,
        #[case] penetration: f32,
        #[case] separation: f32,
        #[case] supported: bool,
    ) {
        assert_eq!(
            supports(normal, point, Vec3::ZERO, penetration, separation),
            supported
        );
    }
    fn ev(world: &mut World, happening: Happening) -> AbilityEvent {
        AbilityEvent {
            entity: world.spawn_empty().id(),
            happening,
            at: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            partner: None,
        }
    }

    /// 収集の順序に依存せず、種類順（盤面に出た → 射出 → 衝突 → 挙動 → 時限）→ Entity順に確定する。
    #[rstest]
    fn events_are_resolved_in_a_fixed_order() {
        let mut world = World::new();
        let [a, b, c, d, e, f] = [
            Happening::Entered,
            Happening::Launched,
            Happening::Collided(Partner::Enemy),
            Happening::Collided(Partner::Medal),
            Happening::Motion(MotionEvent::Landed),
            Happening::Due(0),
        ]
        .map(|h| ev(&mut world, h));
        let sorted = |mut events: Vec<AbilityEvent>| {
            events.sort_by_key(AbilityEvent::order_key);
            events
        };
        let one = sorted(vec![f, e, d, c, b, a]);
        assert_eq!(one, sorted(vec![c, a, f, d, b, e]), "the input order does not matter");
        let kinds: Vec<_> = one.iter().map(|ev| ev.order_key().0).collect();
        assert_eq!(kinds, [0, 1, 2, 2, 3, 4]);
        let collisions = [one[2].entity, one[3].entity];
        assert!(collisions[0] < collisions[1], "same kind: by entity");
    }
}
