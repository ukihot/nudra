//! メダルの能力の ECS 側（基本設計 3.14〜3.16）。
//!
//! 出来事（盤面に出た・射出・衝突・着地・静止・再始動・時限・落下）を検知して application の
//! ユースケースに渡し、返った指示を実行する。ここで行うのは範囲内の対象の検索と物理だけで、
//! 発動の可否・印・反応・生成するメダルのルールは持たない。
//!
//! 発動した指示は [`PendingOrders`] に積み、[`execute_orders`]
//! がまとめて実行する（検知と実行を分ける）。

use std::collections::HashSet;

use application::{
    AbilityOrder, EffectOrder, Firing, PhysicsFact, expire_marks, fire_abilities, hit_target,
    stand_in_zone,
};
use avian3d::prelude::*;
use bevy::{
    ecs::{entity::EntityHashSet, query::QueryData},
    prelude::*,
    utils::Parallel,
};
use domain::{
    ability::{Area, Direction, Effect, Happening, Partner, SpawnAt, Target, Trigger},
    mark::Modifiers,
    motion::{MedalMotion, Motion, MotionEvent, MotionRules, MotionSample},
    session::GameSession,
    spec::SpecCatalog,
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
use crate::port::{Appearance, DomainEventMsg, MedalMotionState, Silhouette};

// ── 調整ノブ ──────────────────────────────────────────────

/// 上空から降らせる高さ（m）。
const SKY_HEIGHT: f32 = 6.0;
/// その場に戻す・生成するときの、盤面からの高さ（m）。
const SAFE_HEIGHT: f32 = 1.0;
/// その場に生成するときの、1枚ごとのずらし（m）。
const SPAWN_SPACING: f32 = 0.35;
/// 場の表示の厚さ（m）。
const ZONE_HEIGHT: f32 = 0.04;

// ── Resource / Component ─────────────────────────────────

/// 実行待ちの指示。
#[derive(Resource, Debug, Default)]
pub struct PendingOrders(Vec<Placed>);

/// 位置の付いた指示。`touched` は衝突の相手（範囲 `Touched` の対象）。
#[derive(Debug, Clone)]
struct Placed {
    order: AbilityOrder,
    at: Vec3,
    forward: Vec3,
    source: Option<Entity>,
    touched: Option<Entity>,
}

/// このtickに奈落へ落ちたメダル（物理の収集が積み、[`medal_falls`] が消費する）。
#[derive(Resource, Debug, Default)]
pub struct FallenMedals(pub EntityHashSet);

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
}

/// 場。中にいる対象へ印を付け続ける（基本設計 3.14）。
#[derive(Component, Debug, Clone)]
pub struct ZoneField {
    effect: Effect,
    target: Target,
    radius: f32,
    until: Tick,
}

/// メダルの物理特性を、種類・レベル・印から作り直すためのまとまり。
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
        let unit = &self.medal.0;
        let Some(spec) = specs.medal(unit.medal(), unit.level()) else {
            warn!(medal = %unit.medal(), "medal spec is missing");
            return;
        };
        let body = MedalBody::new(spec, &unit.modifiers(specs.abilities()));
        let key = Appearance::medal_key(unit.medal());
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

// ── 発動と指示の積み込み ──────────────────────────────────

fn queue(
    firing: Firing,
    placed: (Vec3, Vec3),
    source: Option<Entity>,
    touched: Option<Entity>,
    orders: &mut PendingOrders,
    events: &mut MessageWriter<DomainEventMsg>,
) {
    let (at, forward) = placed;
    events.write_batch(firing.events.into_iter().map(DomainEventMsg));
    orders
        .0
        .extend(firing.orders.into_iter().map(|order| Placed {
            order,
            at,
            forward,
            source,
            touched,
        }));
}

/// 速度の向き（止まっていれば奥）。前方の範囲と押す向きの基準。
fn forward(velocity: Vec3) -> Vec3 {
    Vec3::new(velocity.x, 0.0, velocity.z).normalize_or(Vec3::NEG_Z)
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

/// メダルを盤面に出す。予定と衝突の受信を整え、盤面に出た（射出なら射出も）能力を発動する。
#[allow(clippy::too_many_arguments)]
pub fn spawn_medal(
    commands: &mut Commands,
    session: &mut GameSession,
    specs: &SpecCatalog,
    mut unit: MedalUnit,
    position: Vec3,
    velocity: Vec3,
    happenings: &[Happening],
    now: Tick,
    orders: &mut PendingOrders,
    events: &mut MessageWriter<DomainEventMsg>,
) -> Option<Entity> {
    let Some(spec) = specs.medal(unit.medal(), unit.level()).copied() else {
        warn!(medal = %unit.medal(), "cannot spawn a medal without a spec");
        return None;
    };
    let entity = commands.spawn_empty().id();
    for &happening in happenings {
        let firing = fire_abilities(session, specs, &mut unit, happening);
        queue(
            firing,
            (position, forward(velocity)),
            Some(entity),
            None,
            orders,
            events,
        );
    }
    let timers = AbilityTimers::new(specs, &unit, now);
    let modifiers = unit.modifiers(specs.abilities());
    // 射出も生成も宙から始まる。着地を検知するため、飛行中は衝突を受け取る（着地後、
    // 衝突の能力が無ければ外す）
    commands.entity(entity).insert((
        medal_bundle(
            unit,
            &spec,
            &modifiers,
            Transform::from_translation(position),
            MedalMotion::launched(),
        ),
        LinearVelocity(velocity),
        CollisionEventsEnabled,
    ));
    // 予定は既定（空）を上書きする
    commands.entity(entity).insert(timers);
    Some(entity)
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
/// 盤面の外（奈落の上など、足場のない位置）ならカタパルトへ（基本設計 3.18）。
fn spawn_point(at: SpawnAt, here: Vec3, stage: &Stage) -> Vec3 {
    let floor = stage.geometry().floor_y;
    match at {
        SpawnAt::Here if on_board(stage, here) => Vec3::new(here.x, floor + SAFE_HEIGHT, here.z),
        SpawnAt::Sky if on_board(stage, here) => Vec3::new(here.x, floor + SKY_HEIGHT, here.z),
        _ => stage.catapult().position(),
    }
}

// ── 落下（Resolve） ───────────────────────────────────────

/// 落下の処理で使うメダルの Query（自分が落ちた・他が落ちたの両方で使う）。
pub type FallMedals<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Medal,
        &'static mut Position,
        &'static mut LinearVelocity,
        &'static mut MedalMotionState,
    ),
>;

/// このtickに撃破が確定した敵の数（Domain の `enemy_fell` の結果。`OtherFell` の通知に使う）。
#[derive(Resource, Debug, Default)]
pub struct DefeatedEnemies(pub usize);

/// 落ちたメダル: 自分が落ちた能力（復活なら盤面へ戻す）を判定してから退場を確定し、
/// 報酬資格のある落下だけの価値をゲージの事実にする。復活しなかった個体ごとに `OtherFell`
/// を通知する。 並列収集・ハッシュ集合の順序に依存しないよう、Entity順に処理する（基本設計 6.2）。
#[allow(clippy::too_many_arguments)]
pub fn medal_falls(
    mut fallen: ResMut<FallenMedals>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut medals: FallMedals,
    mut facts: ResMut<PendingFacts>,
    mut orders: ResMut<PendingOrders>,
    mut events: MessageWriter<DomainEventMsg>,
    mut commands: Commands,
) {
    if fallen.0.is_empty() {
        return;
    }
    let mut falling: Vec<Entity> = fallen.0.drain().collect();
    falling.sort_unstable();
    let ActiveGame { session, stage, .. } = &mut *game;
    let book = specs.abilities();
    let mut value = 0.0;
    let mut fell = Vec::with_capacity(falling.len());
    for entity in falling {
        let Ok((entity, mut medal, mut position, mut velocity, mut motion)) =
            medals.get_mut(entity)
        else {
            continue;
        };
        // 落下地点（自分が落ちた能力の位置、`Here` の復活の基準）
        let at = position.0;
        let firing = fire_abilities(session, &specs, &mut medal.0, Happening::SelfFell);
        let revive = firing
            .orders
            .iter()
            .flat_map(|o| &o.actions)
            .find_map(|a| match a {
                EffectOrder::Apply(Effect::Revive { at }) => Some(*at),
                _ => None,
            });
        queue(
            firing,
            (at, Vec3::NEG_Z),
            Some(entity),
            None,
            &mut orders,
            &mut events,
        );
        match revive {
            Some(revive_at) => {
                // 位置・描画用Transform・
                // 速度をそろえて戻す（Avianが古いTransformから位置を戻さないように）
                let to = spawn_point(revive_at, at, stage);
                position.0 = to;
                velocity.0 = Vec3::ZERO;
                motion.0.lift();
                commands
                    .entity(entity)
                    .insert((Transform::from_translation(to), CollisionEventsEnabled));
            },
            // 復活しなかった: 奈落への退場を確定する。報酬は資格のある個体だけ（基本設計 3.18）
            None => {
                value += medal
                    .0
                    .exit_reward(MedalExitReason::FellIntoAbyss, book);
                fell.push(entity);
                commands.entity(entity).try_despawn();
            },
        }
    }
    notify_other_fell(
        fell.len(),
        &fell,
        &mut medals,
        session,
        &specs,
        &mut orders,
        &mut events,
    );
    if value > 0.0 {
        facts.0.push(PhysicsFact::MedalsFell(value));
    }
}

/// 敵の撃破が確定した数だけ、生存しているメダルへ `OtherFell` を通知する（基本設計 4.3）。
pub fn enemy_falls(
    mut defeated: ResMut<DefeatedEnemies>,
    mut medals: FallMedals,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut orders: ResMut<PendingOrders>,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let times = std::mem::take(&mut defeated.0);
    if times > 0 {
        notify_other_fell(
            times,
            &[],
            &mut medals,
            &mut game.session,
            &specs,
            &mut orders,
            &mut events,
        );
    }
}

/// 退場した数だけ、`OtherFell` の能力を持つ生存メダルへ通知する。退場した本人（`exclude`）は除く。
fn notify_other_fell(
    times: usize,
    exclude: &[Entity],
    medals: &mut FallMedals,
    session: &mut GameSession,
    specs: &SpecCatalog,
    orders: &mut PendingOrders,
    events: &mut MessageWriter<DomainEventMsg>,
) {
    let book = specs.abilities();
    for _ in 0..times {
        for (entity, mut medal, position, velocity, _) in medals.iter_mut() {
            if exclude.contains(&entity)
                || book
                    .triggered(medal.0.medal(), medal.0.level(), Happening::OtherFell)
                    .next()
                    .is_none()
            {
                continue;
            }
            let firing = fire_abilities(session, specs, &mut medal.0, Happening::OtherFell);
            queue(
                firing,
                (position.0, forward(velocity.0)),
                Some(entity),
                None,
                orders,
                events,
            );
        }
    }
}

// ── 出来事の検知（Abilities） ─────────────────────────────

/// 衝突: 飛行中のメダルが盤面に触れたこと（着地の判定）と、メダル・敵とぶつかった能力。
#[allow(clippy::too_many_arguments)]
pub fn collisions(
    mut collisions: MessageReader<CollisionStart>,
    mut medals: Query<(&mut Medal, &Position, &LinearVelocity, &MedalMotionState)>,
    enemies: Query<(), With<Enemy>>,
    sensors: Query<(), With<Sensor>>,
    mut seen: Local<HashSet<(Entity, Entity)>>,
    mut touching: ResMut<Touching>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut orders: ResMut<PendingOrders>,
    mut events: MessageWriter<DomainEventMsg>,
) {
    touching.0.clear();
    seen.clear();
    for c in collisions.read() {
        // 同じ接触開始のペアはtick内で1回だけ（両側をそれぞれ判定する）
        let pair = if c.collider1 < c.collider2 {
            (c.collider1, c.collider2)
        } else {
            (c.collider2, c.collider1)
        };
        if !seen.insert(pair) {
            continue;
        }
        for (me, other) in [(c.collider1, c.collider2), (c.collider2, c.collider1)] {
            let partner = if enemies.contains(other) {
                Some(Partner::Enemy)
            } else if medals.contains(other) {
                Some(Partner::Medal)
            } else {
                None
            };
            let Ok((mut medal, position, velocity, motion)) = medals.get_mut(me) else {
                continue;
            };
            if motion.0.state() == Motion::Flying && !sensors.contains(other) {
                touching.0.insert(me);
            }
            let Some(partner) = partner else {
                continue;
            };
            let firing = fire_abilities(
                &mut game.session,
                &specs,
                &mut medal.0,
                Happening::Collided(partner),
            );
            queue(
                firing,
                (position.0, forward(velocity.0)),
                Some(me),
                Some(other),
                &mut orders,
                &mut events,
            );
        }
    }
}

/// 挙動ステータスの観測（数千枚を並列に）。状態が変わったメダルの能力を発動する。
#[allow(clippy::too_many_arguments)]
pub fn observe_motion(
    mut medals: Query<(Entity, &mut MedalMotionState, &LinearVelocity, &mut Medal, &Position)>,
    touching: Res<Touching>,
    clock: Res<GameClock>,
    mut changed: Local<Parallel<Vec<(Entity, MotionEvent)>>>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut orders: ResMut<PendingOrders>,
    mut events: MessageWriter<DomainEventMsg>,
    mut commands: Commands,
) {
    let now = clock.now();
    let rules = MotionRules::default();
    medals
        .par_iter_mut()
        .for_each(|(entity, mut motion, velocity, _, _)| {
            let sample = MotionSample {
                speed: velocity.length(),
                grounded: touching.0.contains(&entity),
            };
            // 状態が変わらない大多数のメダルで変更を立てないよう、写しで観測する
            let mut next = motion.0;
            if let Some(event) = next.observe(sample, now, &rules) {
                motion.0 = next;
                changed.scope(|v| v.push((entity, event)));
            } else if next != motion.0 {
                motion.bypass_change_detection().0 = next;
            }
        });
    // 並列収集の順序に依存しないよう、Entity順に発動する（基本設計 6.2）
    let mut changed: Vec<_> = changed.drain().collect();
    changed.sort_unstable_by_key(|(entity, _)| *entity);
    for (entity, event) in changed {
        let Ok((_, _, velocity, mut medal, position)) = medals.get_mut(entity) else {
            continue;
        };
        let firing =
            fire_abilities(&mut game.session, &specs, &mut medal.0, Happening::Motion(event));
        queue(
            firing,
            (position.0, forward(velocity.0)),
            Some(entity),
            None,
            &mut orders,
            &mut events,
        );
        if event == MotionEvent::Landed && !listens_to_collisions(&specs, &medal.0) {
            commands
                .entity(entity)
                .remove::<CollisionEventsEnabled>();
        }
    }
}

/// 時限・間隔の能力。
#[allow(clippy::too_many_arguments)]
pub fn run_timers(
    mut medals: Query<(Entity, &mut AbilityTimers, &mut Medal, &Position, &LinearVelocity)>,
    clock: Res<GameClock>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut orders: ResMut<PendingOrders>,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let now = clock.now();
    for (entity, mut timers, mut medal, position, velocity) in &mut medals {
        if !timers.0.iter().any(|d| d.at <= now) {
            continue;
        }
        for due in timers.0.iter_mut().filter(|d| d.at <= now) {
            let firing =
                fire_abilities(&mut game.session, &specs, &mut medal.0, Happening::Due(due.index));
            queue(
                firing,
                (position.0, forward(velocity.0)),
                Some(entity),
                None,
                &mut orders,
                &mut events,
            );
            if let Some(every) = due.every {
                due.at = now + every;
            }
        }
        timers.0.retain(|d| d.at > now);
    }
}

// ── 指示の実行（Abilities） ───────────────────────────────

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

type MedalTargets<'w, 's> =
    Query<'w, 's, (Entity, MedalBodyData, Forces, &'static mut MedalMotionState), Without<Enemy>>;
type EnemyPositions<'w, 's> =
    Query<'w, 's, (&'static mut EnemyMarks, &'static Position), (With<Enemy>, Without<Medal>)>;
type EnemyTargets<'w, 's> =
    Query<'w, 's, (Entity, &'static mut EnemyMarks, Forces), (With<Enemy>, Without<Medal>)>;

/// 範囲内の対象（メダル・敵）。
fn targets(
    placed: &Placed,
    medals: &MedalTargets,
    enemies: &EnemyTargets,
) -> (Vec<Entity>, Vec<Entity>) {
    let order = &placed.order;
    let wants = |medal: bool| match order.target {
        Target::Itself => false,
        Target::Medals => medal,
        Target::Enemies => !medal,
        Target::Both => true,
    };
    if order.target == Target::Itself || order.area == Area::Touched {
        let one = if order.target == Target::Itself {
            placed.source
        } else {
            placed.touched
        };
        let Some(one) = one else {
            return (Vec::new(), Vec::new());
        };
        let is_medal = medals.contains(one);
        return match (is_medal, enemies.contains(one)) {
            (true, _) if order.target == Target::Itself || wants(true) => (vec![one], Vec::new()),
            (_, true) if wants(false) => (Vec::new(), vec![one]),
            _ => (Vec::new(), Vec::new()),
        };
    }
    let hit = |p: Vec3| in_area(order.area, placed.at, placed.forward, p);
    let medal_targets = if wants(true) {
        medals
            .iter()
            .filter(|(e, ..)| Some(*e) != placed.source)
            .filter(|(_, _, f, _)| hit(f.position()))
            .map(|(e, ..)| e)
            .collect()
    } else {
        Vec::new()
    };
    let enemy_targets = if wants(false) {
        enemies
            .iter()
            .filter(|(_, _, f)| hit(f.position()))
            .map(|(e, ..)| e)
            .collect()
    } else {
        Vec::new()
    };
    (medal_targets, enemy_targets)
}

/// 積まれた指示を実行する。場と生成は指示ごとに1回、それ以外は範囲内の対象それぞれに。
#[allow(clippy::too_many_arguments)]
pub fn execute_orders(
    mut orders: ResMut<PendingOrders>,
    mut medals: MedalTargets,
    mut enemies: EnemyTargets,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    clock: Res<GameClock>,
    mut events: MessageWriter<DomainEventMsg>,
    mut commands: Commands,
) {
    if orders.0.is_empty() {
        return;
    }
    let now = clock.now();
    // 実行中に生成したメダルの指示は次のtickに実行する
    let placed_orders = std::mem::take(&mut orders.0);
    for placed in placed_orders {
        let order = &placed.order;
        for action in &order.actions {
            match action {
                EffectOrder::Apply(effect @ Effect::Zone { mark, radius, duration, .. }) => {
                    commands.spawn((
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
                        Transform::from_translation(placed.at),
                        in_game(),
                    ));
                },
                EffectOrder::Spawn { medal, level, count, at, mark, rewarded } => {
                    // 子は親の世代+1、報酬資格は生成の定義どおり（基本設計 3.18）
                    let parent = placed
                        .source
                        .and_then(|s| medals.get(s).ok())
                        .map_or_else(
                            || MedalUnit::new(medal.clone(), *level),
                            |(_, body, ..)| body.medal.0.clone(),
                        );
                    let ActiveGame { session, stage, .. } = &mut *game;
                    for i in 0..count.get() {
                        let mut unit = parent.offspring(medal.clone(), *level, *rewarded);
                        if let Some(def) = mark.as_ref().and_then(|m| specs.abilities().mark(m)) {
                            unit.conditions_mut().apply(def, None);
                        }
                        let offset = Vec3::X * SPAWN_SPACING * f32::from(i);
                        let position = spawn_point(*at, placed.at, stage) + offset;
                        spawn_medal(
                            &mut commands,
                            session,
                            &specs,
                            unit,
                            position,
                            Vec3::ZERO,
                            &[Happening::Entered],
                            now,
                            &mut orders,
                            &mut events,
                        );
                    }
                },
                _ => {},
            }
        }
        let (medal_targets, enemy_targets) = targets(&placed, &medals, &enemies);
        for entity in medal_targets {
            let Ok((entity, mut body, mut forces, mut motion)) = medals.get_mut(entity) else {
                continue;
            };
            let mut reactions = Vec::new();
            if !order.states.is_empty() {
                let out = hit_target(&specs, body.medal.0.conditions_mut(), &order.states, now);
                if out.modifiers.is_some() {
                    body.refresh(&specs, &mut commands);
                }
                reactions = out.reactions;
            }
            let position = forces.position();
            let actions = order
                .actions
                .iter()
                .cloned()
                .chain(reactions.into_iter().map(EffectOrder::Apply));
            for action in actions {
                match action {
                    EffectOrder::Apply(Effect::Push { dir, strength }) => {
                        forces.apply_linear_impulse(
                            push_direction(dir, placed.at, placed.forward, position)
                                * strength.get(),
                        );
                        if (Effect::Push { dir, strength }).lifts() {
                            motion.0.lift();
                            commands.entity(entity).insert(CollisionEventsEnabled);
                        }
                    },
                    EffectOrder::Apply(Effect::Remove) => commands.entity(entity).try_despawn(),
                    EffectOrder::Apply(Effect::Split) => {
                        // 分割も同じ系譜・報酬の制限を通す（素の複製で迂回しない）
                        let unit = body.medal.0.split_off();
                        if let Some(spec) = specs.medal(unit.medal(), unit.level()) {
                            let modifiers = unit.modifiers(specs.abilities());
                            commands.spawn(medal_bundle(
                                unit,
                                spec,
                                &modifiers,
                                Transform::from_translation(position + Vec3::Y * 0.2),
                                motion.0,
                            ));
                        }
                    },
                    EffectOrder::Transform { medal, level } => {
                        body.medal.0.transform(medal, level);
                        body.refresh(&specs, &mut commands);
                        resubscribe(&mut commands, entity, &specs, &body.medal.0, motion.0.state());
                    },
                    _ => {},
                }
            }
        }
        for entity in enemy_targets {
            let Ok((_, mut marks, mut forces)) = enemies.get_mut(entity) else {
                continue;
            };
            let mut reactions = Vec::new();
            if !order.states.is_empty() {
                let out = hit_target(&specs, &mut marks.conditions, &order.states, now);
                if let Some(modifiers) = out.modifiers {
                    marks.modifiers = modifiers;
                }
                reactions = out.reactions;
            }
            let position = forces.position();
            let pushes = order
                .actions
                .iter()
                .filter_map(|a| match a {
                    EffectOrder::Apply(Effect::Push { dir, strength }) => Some((*dir, *strength)),
                    _ => None,
                })
                .chain(reactions.iter().filter_map(|e| match e {
                    Effect::Push { dir, strength } => Some((*dir, *strength)),
                    _ => None,
                }));
            for (dir, strength) in pushes {
                forces.apply_linear_impulse(
                    push_direction(dir, placed.at, placed.forward, position) * strength.get(),
                );
            }
        }
    }
}

/// 場: 期限が来たら消し、中にいる対象へ印を付ける。
#[allow(clippy::too_many_arguments)]
pub fn zones(
    fields: Query<(Entity, &ZoneField, &Transform)>,
    mut medals: Query<(MedalBodyData, &Position), Without<Enemy>>,
    mut enemies: EnemyPositions,
    specs: Res<Specs>,
    clock: Res<GameClock>,
    mut commands: Commands,
) {
    let now = clock.now();
    for (entity, field, transform) in &fields {
        if now >= field.until {
            commands.entity(entity).try_despawn();
            continue;
        }
        let inside =
            |p: &Position| p.distance_squared(transform.translation) <= field.radius * field.radius;
        if matches!(field.target, Target::Medals | Target::Both) {
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
        if matches!(field.target, Target::Enemies | Target::Both) {
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
pub fn expire(
    mut medals: Query<(Entity, MedalBodyData), Without<Enemy>>,
    mut enemies: Query<&mut EnemyMarks, (With<Enemy>, Without<Medal>)>,
    specs: Res<Specs>,
    clock: Res<GameClock>,
    mut commands: Commands,
) {
    let now = clock.now();
    for (entity, mut body) in &mut medals {
        if body.medal.0.conditions().is_empty() {
            continue;
        }
        let out = expire_marks(&specs, body.medal.0.conditions_mut(), now);
        // 期限で消える印は能力による消滅（DestroyedByAbility）。通常の報酬は出さない（基本設計
        // 3.18）
        if out.vanish {
            commands.entity(entity).try_despawn();
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

/// 敵の歩行に印の効き目を掛ける（速さ・進路のずれ）。
pub fn enemy_walk(modifiers: Option<&EnemyMarks>) -> Modifiers {
    modifiers.map_or(Modifiers::NONE, |m| m.modifiers)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::infrastructure::data::StageAsset;

    /// stage_01: 幅8m・奥行16mの長方形、床の高さ0、カタパルトは (0, 0.6, 5.8)。
    fn stage() -> Stage {
        ron::from_str::<StageAsset>(include_str!("../../assets/data/stages/stage_01.stage.ron"))
            .expect("valid RON")
            .to_domain()
            .expect("valid stage")
    }

    const CATAPULT: Vec3 = Vec3::new(0.0, 0.6, 5.8);

    #[rstest]
    #[case::here_on_the_board(SpawnAt::Here, Vec3::new(1.0, -20.0, 2.0), Vec3::new(1.0, SAFE_HEIGHT, 2.0))]
    #[case::sky_on_the_board(SpawnAt::Sky, Vec3::new(-3.0, 4.0, -7.0), Vec3::new(-3.0, SKY_HEIGHT, -7.0))]
    #[case::here_off_the_board(SpawnAt::Here, Vec3::new(4.5, -20.0, 0.0), CATAPULT)]
    #[case::sky_off_the_board(SpawnAt::Sky, Vec3::new(0.0, 0.0, 8.5), CATAPULT)]
    #[case::catapult(SpawnAt::Catapult, Vec3::new(1.0, 0.0, 1.0), CATAPULT)]
    fn spawn_points_stay_above_the_board(
        #[case] at: SpawnAt,
        #[case] here: Vec3,
        #[case] expected: Vec3,
    ) {
        assert_eq!(spawn_point(at, here, &stage()), expected);
    }
}
