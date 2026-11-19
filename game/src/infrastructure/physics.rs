//! Avian との接続（基本設計 5.4）。Avianへの依存はこのファイルと world.rs に閉じ込める。

use application::PhysicsFact;
use avian3d::{dynamics::rigid_body::forces::ForcesItem, prelude::*};
use bevy::prelude::*;
use domain::{id::EnemyId, pusher::MovementPattern, stage::Rail};

use super::world::{AbyssZone, Enemy, GoalZone, MedalKind, Navigation, Pusher, PusherMotion, Walker};

/// 衝突レイヤー。組み合わせは [`layers`] で定義する。
#[derive(PhysicsLayer, Default, Clone, Copy, Debug)]
pub enum Layer {
    #[default]
    Stage,
    Medal,
    Enemy,
    Pusher,
    Abyss,
    Goal,
    /// 敵の出現領域の床。メダルの物理作用が及ばないよう敵とだけ衝突する（要件4.4）
    SpawnFloor,
}

/// レイヤーの組み合わせ（基本設計 5.4 の表）。
pub fn layers(layer: Layer) -> CollisionLayers {
    use Layer::*;
    let filters: &[Layer] = match layer {
        Stage => &[Medal, Enemy],
        Medal => &[Stage, Medal, Enemy, Pusher, Abyss],
        Enemy => &[Stage, Medal, Enemy, Pusher, Abyss, Goal, SpawnFloor],
        // 床と接触させない（並進Yを固定して浮かせる）ことで床の摩擦を受けない
        Pusher => &[Medal, Enemy],
        Abyss => &[Medal, Enemy],
        Goal => &[Enemy],
        SpawnFloor => &[Enemy],
    };
    CollisionLayers::new(layer, filters.iter().fold(LayerMask::NONE, |mask, &l| mask | l))
}

// ── 調整ノブ ──────────────────────────────────────────────

/// 床面からこの深さより下に落ちた物体は除去する（センサーをすり抜けた場合の安全策）。
pub const KILL_DEPTH: f32 = 30.0;
/// Pusherの追従制御のゲイン（1/s²）。減衰は臨界減衰になるよう決める。
const PUSHER_STIFFNESS: f32 = 80.0;
/// 目標点がPusherからこれ以上離れたら、目標点の前進を止める（押し負けたとき）。
const PUSHER_MAX_LEAD: f32 = 0.6;

// ── 物理結果の収集 ────────────────────────────────────────

/// センサーの接触開始と安全高度から、ゲーム判定に必要な事実を集める。
/// 奈落に落ちたメダルはDomainへ渡さず、その場で除去する（基本設計 5.4）。
pub fn collect_facts(
    mut collisions: MessageReader<CollisionStart>,
    zones: Query<(Has<AbyssZone>, Option<&GoalZone>)>,
    enemies: Query<&Enemy>,
    medals: Query<(), With<MedalKind>>,
    fallen: Query<(Entity, &Position, Option<&Enemy>), Or<(With<Enemy>, With<MedalKind>)>>,
    kill_y: Res<KillPlane>,
    mut facts: ResMut<PendingFacts>,
    mut commands: Commands,
) {
    for c in collisions.read() {
        for (zone, other) in [(c.collider1, c.collider2), (c.collider2, c.collider1)] {
            let Ok((is_abyss, goal)) = zones.get(zone) else { continue };
            if let Ok(enemy) = enemies.get(other) {
                match goal {
                    Some(goal) => facts.0.push(PhysicsFact::EnemyEnteredGoal(enemy.id, goal.0)),
                    None if is_abyss => facts.0.push(PhysicsFact::EnemyEnteredAbyss(enemy.id)),
                    None => {},
                }
            } else if is_abyss && medals.contains(other) {
                commands.entity(other).try_despawn();
            }
        }
    }
    for (entity, position, enemy) in &fallen {
        if position.y < kill_y.0 {
            match enemy {
                Some(enemy) => facts.0.push(PhysicsFact::EnemyEnteredAbyss(enemy.id)),
                None => commands.entity(entity).try_despawn(),
            }
        }
    }
}

/// これより下に落ちた物体を除去する高さ。
#[derive(Resource, Debug, Clone, Copy)]
pub struct KillPlane(pub f32);

/// Resolveで消費される、物理ステップ由来の事実。
#[derive(Resource, Debug, Default)]
pub struct PendingFacts(pub Vec<PhysicsFact>);

pub fn despawn_enemies(defeated: &[EnemyId], enemies: &Query<(Entity, &Enemy)>, commands: &mut Commands) {
    for (entity, enemy) in enemies {
        if defeated.contains(&enemy.id) {
            commands.entity(entity).try_despawn();
        }
    }
}

// ── 行動の制御（物理ステップの前） ─────────────────────────

/// 敵: 経路の進行方向へ、最大速度に達するまで歩行力を加える。
/// 速度を直接上書きしないため、メダルに押し流される挙動が残る（要件7.1）。
pub fn walk_enemies(mut enemies: Query<(&mut Navigation, &Walker, Forces)>) {
    for (mut nav, walker, mut forces) in &mut enemies {
        let direction = nav.0.steer(forces.position());
        if forces.linear_velocity().dot(direction) < walker.max_speed {
            forces.apply_force(direction * walker.walk_force);
        }
    }
}

/// Pusher: Rail上の目標点へ、最大推力＝押し出し力の制御力で追従する（基本設計 5.4）。
pub fn drive_pusher(
    pusher: &Pusher,
    motion: &mut PusherMotion,
    rail: &Rail,
    pattern: &MovementPattern,
    dt: f32,
    forces: &mut ForcesItem<'_, '_>,
) {
    let position = forces.position();
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    // 押し負けて離されたら目標点を止め、追いつくのを待つ
    if flat(motion.state.target(rail) - position).length() < PUSHER_MAX_LEAD {
        motion.state = motion.state.advance(rail, pattern, pusher.stats.speed.get(), dt);
    }
    let error = flat(motion.state.target(rail) - position);
    let damping = 2.0 * PUSHER_STIFFNESS.sqrt();
    let acceleration = error * PUSHER_STIFFNESS - flat(forces.linear_velocity()) * damping;
    let force = (acceleration * pusher.stats.mass.get()).clamp_length_max(pusher.stats.push_force.get());
    forces.apply_force(force);
}
