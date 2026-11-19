//! ヘッドレスの結合テスト（基本設計 10章 Bevy統合）。
//! 実際の定義データ（game/assets/data）を読み込み、固定ステップを1tickずつ手動で進める。

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use domain::{event::DomainEvent, id::PusherCharacterId};
use game::{
    infrastructure::{
        session::ActiveSession,
        world::{Enemy, MedalKind, Pusher},
    },
    port::{CharacterRoster, DomainEventMsg, GameResult, LaunchRequested, StartRequested},
    state::AppState,
};

/// 固定ステップの既定値（64Hz）。1回の `update` でちょうど1tick進める。
const TICK: Duration = Duration::from_micros(15_625);
/// stage_01 の初期メダル数（12列 × 7行 × 3段）
const INITIAL_MEDALS: usize = 12 * 7 * 3;

#[derive(Resource, Default)]
struct Recorded(Vec<DomainEvent>);

fn record(mut messages: MessageReader<DomainEventMsg>, mut recorded: ResMut<Recorded>) {
    recorded.0.extend(messages.read().map(|m| m.0.clone()));
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        AssetPlugin::default(),
        TransformPlugin,
        PhysicsPlugins::new(FixedPostUpdate),
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(TICK))
    .add_plugins(game::core_plugin)
    .init_resource::<Recorded>()
    .add_systems(Update, record);
    app
}

/// 条件を満たすまで更新する。アセット読込は別スレッドのため、少し待ちながら回す。
fn run_until(app: &mut App, max_updates: usize, mut done: impl FnMut(&mut App) -> bool) {
    for _ in 0..max_updates {
        app.update();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("condition not met within {max_updates} updates");
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query_filtered::<(), With<C>>().iter(app.world()).count()
}

fn recorded(app: &App) -> &[DomainEvent] {
    &app.world().resource::<Recorded>().0
}

fn started() -> App {
    let mut app = app();
    run_until(&mut app, 3000, |a| state(a) == AppState::CharacterSelect);
    app.world_mut().write_message(StartRequested { character: PusherCharacterId::new("mop_rabbit") });
    run_until(&mut app, 10, |a| {
        state(a) == AppState::InGame && a.world().contains_resource::<ActiveSession>()
    });
    app
}

/// 最初の敵が出現するまで進め、そのEntityを返す。
fn first_enemy(app: &mut App) -> Entity {
    run_until(app, 200, |a| count::<Enemy>(a) > 0);
    app.world_mut().query_filtered::<Entity, With<Enemy>>().iter(app.world()).next().expect("enemy")
}

fn teleport(app: &mut App, entity: Entity, to: Vec3) {
    let mut e = app.world_mut().entity_mut(entity);
    e.insert((Position(to), Transform::from_translation(to), LinearVelocity::ZERO));
}

#[test]
fn definition_data_loads_into_character_select() {
    let mut app = app();
    run_until(&mut app, 3000, |a| state(a) == AppState::CharacterSelect);
    let roster = app.world().resource::<CharacterRoster>();
    let ids: Vec<_> = roster.0.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["iron_bear", "mop_rabbit"]);
}

#[test]
fn start_builds_the_stage_from_data() {
    let mut app = started();
    app.update();
    assert_eq!(count::<MedalKind>(&mut app), INITIAL_MEDALS);
    assert_eq!(count::<Pusher>(&mut app), 1);
    assert!(matches!(recorded(&app).first(), Some(DomainEvent::GameStarted { .. })));
}

#[test]
fn launch_request_spawns_one_medal_per_cooldown() {
    let mut app = started();
    app.update();
    let before = count::<MedalKind>(&mut app);

    app.world_mut().write_message(LaunchRequested);
    app.world_mut().write_message(LaunchRequested);
    app.update();

    assert_eq!(count::<MedalKind>(&mut app), before + 1, "second request hits the cooldown");
    let launched = recorded(&app).iter().filter(|e| matches!(e, DomainEvent::MedalLaunched { .. }));
    assert_eq!(launched.count(), 1);
}

#[test]
fn enemy_falling_into_abyss_is_defeated() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    let id = app.world().get::<Enemy>(enemy).expect("enemy").id;

    teleport(&mut app, enemy, Vec3::new(0.0, -10.0, 0.0));
    run_until(&mut app, 20, |a| a.world().get_entity(enemy).is_err());

    assert!(recorded(&app).contains(&DomainEvent::EnemyDefeated { enemy: id }));
    assert_eq!(state(&app), AppState::InGame);
}

#[test]
fn enemy_reaching_goal_loses_the_game() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    let id = app.world().get::<Enemy>(enemy).expect("enemy").id;

    teleport(&mut app, enemy, Vec3::new(0.0, 0.45, 7.4));
    run_until(&mut app, 20, |a| state(a) == AppState::Result);

    assert!(matches!(
        app.world().get_resource::<GameResult>(),
        Some(GameResult::Lost { enemy, .. }) if *enemy == id
    ));
    assert!(recorded(&app).iter().any(|e| matches!(e, DomainEvent::GameLost { .. })));
}

#[test]
fn launch_in_the_losing_tick_is_rejected() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    teleport(&mut app, enemy, Vec3::new(0.0, 0.45, 7.4));
    // この更新の物理ステップでGoal接触が検出される
    app.update();
    let medals = count::<MedalKind>(&mut app);

    // 次のtickでは Resolve（敗北）→ Command（射出）の順に処理されるため、射出は拒否される
    app.world_mut().write_message(LaunchRequested);
    app.update();

    assert!(recorded(&app).iter().any(|e| matches!(e, DomainEvent::GameLost { .. })));
    assert_eq!(count::<MedalKind>(&mut app), medals);
}
