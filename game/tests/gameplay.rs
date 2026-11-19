//! ヘッドレスの結合テスト（基本設計 10章 Bevy統合、6.4 処理のライフサイクル）。
//!
//! - 実際の定義データ（game/assets/data）を読み込み、固定ステップを1tickずつ手動で進める。
//! - `FallbackErrorHandler` を panic
//!   にし、実行時エラーがエラーハンドラに届いた時点でテストを失敗させる。
//! - セーブ先は必ず一時ディレクトリにする（実ユーザーのデータ領域へ書かない）。

use std::time::Duration;

use application::PhysicsFact;
use avian3d::prelude::*;
use bevy::{
    ecs::error::{FallbackErrorHandler, panic},
    prelude::*,
    state::app::StatesPlugin,
    time::TimeUpdateStrategy,
};
use domain::{event::DomainEvent, id::PusherCharacterId, time::Tick};
use game::{
    infrastructure::{
        physics::PendingFacts,
        progress::SaveJob,
        requests::RequestLedger,
        save::{FileProgressRepository, decode},
        session::{ActiveGame, GameClock},
        world::{Enemy, MedalKind, Pusher},
    },
    port::{
        CharacterRoster, ControlIntent, DomainEventMsg, GameResult, RejectReason, Request,
        RequestOutcome, RequestStatus, SubmitRequest,
    },
    state::{AppState, GamePhase},
};

/// stage_01 の初期メダル数（12列 × 7行 × 3段）
const INITIAL_MEDALS: usize = 12 * 7 * 3;
/// stage_01 のカタパルトのクールダウン 250ms を 60Hz で切り上げたTick数
const COOLDOWN_TICKS: u64 = 15;
/// stage_01 の最初の敵の出現 1000ms を 60Hz で切り上げたTick
const FIRST_SPAWN_TICK: u64 = 60;
const GOAL: Vec3 = Vec3::new(0.0, 0.45, 7.4);
const ABYSS: Vec3 = Vec3::new(0.0, -10.0, 0.0);

#[derive(Resource, Default)]
struct Recorded {
    events: Vec<DomainEvent>,
    outcomes: Vec<RequestOutcome>,
}

fn record(
    mut events: MessageReader<DomainEventMsg>,
    mut outcomes: MessageReader<RequestOutcome>,
    mut recorded: ResMut<Recorded>,
) {
    recorded
        .events
        .extend(events.read().map(|m| m.0.clone()));
    recorded.outcomes.extend(outcomes.read().cloned());
}

/// テストの間だけ存在するセーブ先。Appと一緒に破棄される。
#[derive(Resource)]
struct TempSaveDir {
    _dir: tempfile::TempDir,
}

fn app() -> App {
    app_with_save_dir(tempfile::tempdir().expect("temp dir"))
}

fn app_with_save_dir(dir: tempfile::TempDir) -> App {
    let mut app = App::new();
    app.insert_resource(FallbackErrorHandler(panic))
        .insert_resource(FileProgressRepository::in_dir(dir.path()))
        .insert_resource(TempSaveDir { _dir: dir })
        .add_plugins((
            MinimalPlugins,
            StatesPlugin,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::new(FixedPostUpdate),
        ))
        .add_plugins(game::core_plugin)
        .init_resource::<Recorded>()
        .add_systems(Last, record);
    // App::run と同じく Plugin の finish / cleanup を呼ぶ（Avian などが finish で Resource
    // を登録する）
    app.finish();
    app.cleanup();
    // 1回の update でちょうど1回の固定ステップ（=1Tick）を進める
    let step = app.world().resource::<Time<Fixed>>().timestep();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
    app
}

/// 条件を満たすまで更新する。アセット読込とセーブは別スレッドのため、少し待ちながら回す。
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

fn app_state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn phase(app: &App) -> Option<GamePhase> {
    app.world()
        .get_resource::<State<GamePhase>>()
        .map(|s| *s.get())
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<(), With<C>>()
        .iter(app.world())
        .count()
}

fn recorded(app: &App) -> &Recorded {
    app.world().resource::<Recorded>()
}

fn launches(app: &App) -> usize {
    recorded(app)
        .events
        .iter()
        .filter(|e| matches!(e, DomainEvent::MedalLaunched { .. }))
        .count()
}

fn lost(app: &App) -> bool {
    recorded(app)
        .events
        .iter()
        .any(|e| matches!(e, DomainEvent::GameLost { .. }))
}

fn now(app: &App) -> Tick {
    app.world().resource::<GameClock>().now()
}

fn submit(app: &mut App, request: Request) {
    app.world_mut().write_message(SubmitRequest(request));
}

fn set_fire(app: &mut App, fire: bool) {
    app.world_mut().resource_mut::<ControlIntent>().fire = fire;
}

fn loaded() -> App {
    let mut app = app();
    run_until(&mut app, 3000, |a| app_state(a) == AppState::CharacterSelect);
    app
}

fn started() -> App {
    let mut app = loaded();
    submit(
        &mut app,
        Request::Start { character: PusherCharacterId::new("mop_rabbit") },
    );
    run_until(&mut app, 10, |a| phase(a) == Some(GamePhase::Running));
    app
}

/// 最初の敵が出現するまで進め、そのEntityを返す。
fn first_enemy(app: &mut App) -> Entity {
    run_until(app, 200, |a| count::<Enemy>(a) > 0);
    app.world_mut()
        .query_filtered::<Entity, With<Enemy>>()
        .iter(app.world())
        .next()
        .expect("enemy")
}

fn teleport(app: &mut App, entity: Entity, to: Vec3) {
    let mut e = app.world_mut().entity_mut(entity);
    e.insert((Position(to), Transform::from_translation(to), LinearVelocity::ZERO));
}

fn save_settled(app: &mut App) -> bool {
    let path = app.world().resource::<FileProgressRepository>().path();
    app.world().resource::<SaveJob>().is_idle() && path.exists()
}

fn saved(app: &App) -> domain::progress::PlayerProgress {
    let path = app
        .world()
        .resource::<FileProgressRepository>()
        .path()
        .to_owned();
    decode(&std::fs::read(path).expect("save file exists")).expect("valid save")
}

// ── 読込・開始 ──

#[test]
fn definition_data_loads_into_character_select() {
    let app = loaded();
    let roster = app.world().resource::<CharacterRoster>();
    let ids: Vec<_> = roster.0.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["iron_bear", "mop_rabbit"]);
}

#[test]
fn start_request_completes_and_builds_the_stage() {
    let mut app = started();
    app.update();
    assert_eq!(count::<MedalKind>(&mut app), INITIAL_MEDALS);
    assert_eq!(count::<Pusher>(&mut app), 1);
    assert!(matches!(
        recorded(&app).events.first(),
        Some(DomainEvent::GameStarted { .. })
    ));
    assert!(matches!(
        recorded(&app).outcomes.as_slice(),
        [RequestOutcome {
            request: Request::Start { .. },
            status: RequestStatus::Completed,
            ..
        }]
    ));
}

// ── 指図のライフサイクル ──

#[test]
fn duplicate_start_requests_are_rejected_after_the_first() {
    let mut app = loaded();
    let start = || Request::Start { character: PusherCharacterId::new("mop_rabbit") };
    submit(&mut app, start());
    submit(&mut app, start());
    run_until(&mut app, 10, |a| recorded(a).outcomes.len() == 2);
    let statuses: Vec<_> = recorded(&app)
        .outcomes
        .iter()
        .map(|o| o.status.clone())
        .collect();
    assert_eq!(
        statuses,
        [
            RequestStatus::Rejected(RejectReason::Duplicate),
            RequestStatus::Completed
        ]
    );
}

#[test]
fn unknown_character_is_rejected_and_stays_in_select() {
    let mut app = loaded();
    submit(
        &mut app,
        Request::Start { character: PusherCharacterId::new("nobody") },
    );
    run_until(&mut app, 10, |a| !recorded(a).outcomes.is_empty());
    assert!(matches!(
        recorded(&app).outcomes[0].status,
        RequestStatus::Rejected(RejectReason::Invalid(_))
    ));
    app.update();
    assert_eq!(app_state(&app), AppState::CharacterSelect);
}

#[test]
fn request_in_the_wrong_state_is_rejected() {
    let mut app = started();
    submit(&mut app, Request::Menu);
    run_until(&mut app, 5, |a| recorded(a).outcomes.len() == 2);
    assert_eq!(
        recorded(&app).outcomes[1].status,
        RequestStatus::Rejected(RejectReason::WrongState)
    );
    assert_eq!(app.world().resource::<RequestLedger>().pending_len(), 0);
}

#[test]
fn level_up_request_completes_and_updates_medal_mass() {
    let mass = |app: &mut App| -> f32 {
        app.world_mut()
            .query::<(&MedalKind, &Mass)>()
            .iter(app.world())
            .map(|(_, m)| m.0)
            .sum()
    };
    let mut app = started();
    app.update();
    let before = mass(&mut app);
    app.world_mut()
        .resource_mut::<PendingFacts>()
        .0
        .push(PhysicsFact::MedalsFell(25));
    run_until(&mut app, 5, |a| phase(a) == Some(GamePhase::Evolving));
    let medal = domain::id::MedalId::normal();
    submit(
        &mut app,
        Request::Evolve(domain::evolution::Evolution::LevelUp(medal)),
    );
    run_until(&mut app, 5, |a| recorded(a).outcomes.len() == 2);

    assert_eq!(recorded(&app).outcomes[1].status, RequestStatus::Completed);
    let after = mass(&mut app);
    assert!((after - before * 2.0).abs() < 1e-3, "{before} -> {after}");
}

fn enter_evolution(app: &mut App) {
    app.world_mut()
        .resource_mut::<PendingFacts>()
        .0
        .push(PhysicsFact::MedalsFell(25));
    run_until(app, 5, |a| phase(a) == Some(GamePhase::Evolving));
}

#[test]
fn evolution_freezes_clock_physics_and_firing_until_a_valid_choice() {
    let mut app = started();
    set_fire(&mut app, true);
    let launches_before = launches(&app);
    enter_evolution(&mut app);
    let tick = now(&app);
    let positions = |app: &mut App| {
        app.world_mut()
            .query::<(Entity, &Position)>()
            .iter(app.world())
            .map(|(e, p)| (e, p.0))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = positions(&mut app);
    let offer = app
        .world()
        .resource::<ActiveGame>()
        .session
        .evolution_offer()
        .cloned();
    for _ in 0..15 {
        app.update();
    }
    assert_eq!(now(&app), tick);
    assert_eq!(positions(&mut app), before);
    assert_eq!(launches(&app), launches_before, "no extra shot in the resolve tick");

    submit(
        &mut app,
        Request::Evolve(domain::evolution::Evolution::Acquire(domain::id::MedalId::new(
            "unknown",
        ))),
    );
    run_until(&mut app, 5, |a| recorded(a).outcomes.len() == 2);
    assert!(matches!(
        recorded(&app).outcomes[1].status,
        RequestStatus::Rejected(RejectReason::Invalid(_))
    ));
    assert_eq!(phase(&app), Some(GamePhase::Evolving));
    assert_eq!(now(&app), tick);
    assert_eq!(
        app.world()
            .resource::<ActiveGame>()
            .session
            .evolution_offer(),
        offer.as_ref()
    );

    let action = || {
        Request::Evolve(domain::evolution::Evolution::Acquire(
            offer.clone().expect("new medal"),
        ))
    };
    submit(&mut app, action());
    submit(&mut app, action());
    run_until(&mut app, 5, |a| phase(a) == Some(GamePhase::Running));
    assert_eq!(
        recorded(&app)
            .outcomes
            .iter()
            .filter(|o| o.status == RequestStatus::Completed)
            .count(),
        2,
        "start and one evolution"
    );
    assert!(
        recorded(&app)
            .outcomes
            .iter()
            .any(|o| o.status == RequestStatus::Rejected(RejectReason::Duplicate))
    );
    assert!(!app.world().resource::<Time<Physics>>().is_paused());
    assert!(now(&app) > tick);
}

#[rstest::rstest]
#[case::abyss(-10.0)]
#[case::safety_plane(-31.0)]
fn an_actual_medal_fall_is_counted_once(#[case] depth: f32) {
    let mut app = started();
    let medal = app
        .world_mut()
        .query_filtered::<Entity, With<MedalKind>>()
        .iter(app.world())
        .next()
        .expect("medal");
    teleport(&mut app, medal, Vec3::new(0.0, depth, 0.0));
    run_until(&mut app, 5, |a| a.world().get_entity(medal).is_err());
    assert_eq!(
        app.world()
            .resource::<ActiveGame>()
            .session
            .gauge()
            .ratio(),
        0.04
    );
}

#[test]
fn duplicate_sensor_results_and_safety_plane_count_the_same_medal_once() {
    let mut app = started();
    let medal = app
        .world_mut()
        .query_filtered::<Entity, With<MedalKind>>()
        .iter(app.world())
        .next()
        .expect("medal");
    let abyss = app
        .world_mut()
        .query_filtered::<Entity, With<game::infrastructure::world::AbyssZone>>()
        .iter(app.world())
        .next()
        .expect("abyss");
    teleport(&mut app, medal, Vec3::new(0.0, -31.0, 0.0));
    for _ in 0..2 {
        app.world_mut().write_message(CollisionStart {
            collider1: abyss,
            collider2: medal,
            body1: Some(abyss),
            body2: Some(medal),
        });
    }
    app.update();
    assert_eq!(
        app.world()
            .resource::<ActiveGame>()
            .session
            .gauge()
            .ratio(),
        0.04
    );
}

#[test]
fn fusion_consumes_owned_materials_and_resets_the_selected_medal() {
    use domain::{evolution::Evolution, id::MedalId};
    let mut app = started();
    let m = |id: &str| MedalId::new(id);
    let owns = |app: &App, id: &str| {
        app.world()
            .resource::<ActiveGame>()
            .session
            .medals()
            .level(&m(id))
            .is_some()
    };
    let choose = |app: &mut App, evolution| {
        submit(app, Request::Evolve(evolution));
        run_until(app, 5, |a| phase(a) == Some(GamePhase::Running));
        assert_eq!(
            recorded(app).outcomes.last().expect("outcome").status,
            RequestStatus::Completed
        );
    };
    // New! は未所持の素材から選ぶため、最大16回で両素材が揃う。
    for _ in 0..16 {
        if owns(&app, "fire") && owns(&app, "water") {
            break;
        }
        enter_evolution(&mut app);
        let offered = app
            .world()
            .resource::<ActiveGame>()
            .session
            .evolution_offer()
            .cloned()
            .expect("offered");
        choose(&mut app, Evolution::Acquire(offered));
    }
    for id in ["fire", "water"] {
        for _ in 0..3 {
            enter_evolution(&mut app);
            choose(&mut app, Evolution::LevelUp(m(id)));
        }
    }
    submit(&mut app, Request::SelectMedal(m("fire")));
    run_until(&mut app, 5, |a| {
        a.world().resource::<ActiveGame>().catapult.selected() == &m("fire")
    });
    set_fire(&mut app, true);
    app.update();
    set_fire(&mut app, false);
    let fire_medals = |app: &mut App| {
        app.world_mut()
            .query::<&MedalKind>()
            .iter(app.world())
            .filter(|kind| kind.0 == m("fire"))
            .count()
    };
    assert_eq!(fire_medals(&mut app), 1);
    enter_evolution(&mut app);
    choose(&mut app, Evolution::Fuse(m("fire"), m("water")));
    assert!(!owns(&app, "fire") && !owns(&app, "water"));
    assert!(owns(&app, "sauna"));
    assert_eq!(fire_medals(&mut app), 1, "existing physical medals stay on the board");
    assert_eq!(
        app.world()
            .resource::<ActiveGame>()
            .catapult
            .selected(),
        &MedalId::normal()
    );
    submit(&mut app, Request::SelectMedal(m("fire")));
    let outcomes = recorded(&app).outcomes.len();
    run_until(&mut app, 5, |a| recorded(a).outcomes.len() > outcomes);
    assert!(matches!(
        recorded(&app).outcomes.last().expect("outcome").status,
        RequestStatus::Rejected(RejectReason::Invalid(_))
    ));
}

#[test]
fn final_boss_defeat_wins_and_records_the_result() {
    let mut app = started();
    for wave in 0..4 {
        // 既存の通常敵を残したまま、Waveの全出現時刻をDomainへ進める。
        let boss = {
            let mut game = app.world_mut().resource_mut::<ActiveGame>();
            let ActiveGame { session, stage, .. } = &mut *game;
            let at = Tick((wave + 1) * 10_000);
            let first = session.advance(stage, at);
            let second = session.advance(stage, Tick(at.0 + 10_000));
            first
                .into_iter()
                .chain(second)
                .last()
                .expect("boss order")
                .enemy
        };
        app.world_mut()
            .resource_mut::<PendingFacts>()
            .0
            .push(PhysicsFact::EnemyEnteredAbyss(boss));
        app.update();
    }
    run_until(&mut app, 10, |a| app_state(a) == AppState::Result);
    assert!(matches!(
        app.world().get_resource::<GameResult>(),
        Some(GameResult::Won { defeated: 4, .. })
    ));
    assert_eq!(
        recorded(&app)
            .events
            .iter()
            .filter(|e| matches!(e, DomainEvent::GameWon))
            .count(),
        1
    );
    run_until(&mut app, 500, save_settled);
    let progress = saved(&app);
    assert_eq!(progress.games_played(), 1);
    assert_eq!(
        progress
            .record(&domain::id::StageId::new("stage_01"))
            .expect("record")
            .best_defeated(),
        4
    );
}

// ── 論理時刻 ──

#[test]
fn clock_starts_at_zero_and_advances_once_per_fixed_step() {
    let mut app = started();
    let start = now(&app);
    for _ in 0..10 {
        app.update();
    }
    assert!(start <= Tick(1), "the session starts at tick 0: {start:?}");
    assert_eq!(now(&app), Tick(start.0 + 10));
}

/// 出現は「開始Tick + オフセット ≦ 現在のTick」で決まり、そのTickのSystemはすべて同じ時刻を見る。
#[test]
fn first_enemy_spawns_at_its_scheduled_tick() {
    let mut app = started();
    let spawned = |a: &mut App| {
        recorded(a)
            .events
            .iter()
            .any(|e| matches!(e, DomainEvent::EnemySpawned { .. }))
    };
    run_until(&mut app, 200, spawned);
    // そのTickの処理の最後に時計が進むため、出現したTickの次を指している
    assert_eq!(now(&app), Tick(FIRST_SPAWN_TICK + 1));
}

// ── 発射（意図） ──

/// 発射ボタンを押し続けても、発射はクールダウンごとにしか起きない。却下の通知も出ない。
#[test]
fn holding_fire_launches_once_per_cooldown() {
    let mut app = started();
    app.update();
    let medals = count::<MedalKind>(&mut app);
    set_fire(&mut app, true);
    for _ in 0..=COOLDOWN_TICKS * 2 {
        app.update();
    }
    assert_eq!(launches(&app), 3, "ticks t, t+15, t+30");
    assert_eq!(count::<MedalKind>(&mut app), medals + 3);
    assert_eq!(
        recorded(&app).outcomes.len(),
        1,
        "only the start request has an outcome"
    );
}

#[test]
fn releasing_fire_stops_launching() {
    let mut app = started();
    set_fire(&mut app, true);
    app.update();
    set_fire(&mut app, false);
    for _ in 0..COOLDOWN_TICKS * 3 {
        app.update();
    }
    assert_eq!(launches(&app), 1);
}

// ── 撃破・敗北（Resolve） ──

#[test]
fn enemy_falling_into_abyss_is_defeated() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    let id = app.world().get::<Enemy>(enemy).expect("enemy").id;

    teleport(&mut app, enemy, ABYSS);
    run_until(&mut app, 20, |a| a.world().get_entity(enemy).is_err());

    assert!(
        recorded(&app)
            .events
            .contains(&DomainEvent::EnemyDefeated { enemy: id })
    );
    assert_eq!(phase(&app), Some(GamePhase::Running));
}

#[test]
fn enemy_reaching_goal_ends_the_game_with_play_time() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    let id = app.world().get::<Enemy>(enemy).expect("enemy").id;

    teleport(&mut app, enemy, GOAL);
    run_until(&mut app, 20, |a| app_state(a) == AppState::Result);

    let Some(GameResult::Lost { enemy, play_time, .. }) = app.world().get_resource::<GameResult>()
    else {
        panic!("game result is recorded");
    };
    assert_eq!(*enemy, id);
    assert!(
        *play_time >= Duration::from_secs(1),
        "first enemy spawns after 1s: {play_time:?}"
    );
}

/// 同じTick内の順序: Resolve（敗北）→ Command（発射）。敗北が確定したTickには発射しない。
#[test]
fn no_launch_in_the_tick_the_game_is_lost() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    teleport(&mut app, enemy, GOAL);
    // この更新の物理ステップでGoal接触が検出される
    app.update();
    assert!(!lost(&app));
    let launched = launches(&app);

    // 次のTickでは、発射の意図があっても Resolve が先に敗北を確定させる
    set_fire(&mut app, true);
    app.update();

    assert!(lost(&app));
    assert_eq!(launches(&app), launched);
}

/// 同じTick内の順序: 落下と到達が同時に届いたら落下（撃破）が先。敗北しない。
#[test]
fn fall_and_goal_in_the_same_tick_resolve_as_defeat() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    let id = app.world().get::<Enemy>(enemy).expect("enemy").id;
    let goal = app.world().resource::<ActiveGame>().stage.goals()[0].id;

    let mut facts = app.world_mut().resource_mut::<PendingFacts>();
    facts.0.push(PhysicsFact::EnemyEnteredGoal(id, goal));
    facts.0.push(PhysicsFact::EnemyEnteredAbyss(id));
    app.update();

    assert!(
        recorded(&app)
            .events
            .contains(&DomainEvent::EnemyDefeated { enemy: id })
    );
    assert!(!lost(&app));
    assert_eq!(phase(&app), Some(GamePhase::Running));
}

// ── セーブ（オフラインファースト） ──

#[test]
fn selected_character_is_saved_locally() {
    let mut app = started();
    run_until(&mut app, 200, save_settled);
    assert_eq!(
        saved(&app).last_character(),
        Some(&PusherCharacterId::new("mop_rabbit"))
    );
}

#[test]
fn game_over_is_saved_locally() {
    let mut app = started();
    let enemy = first_enemy(&mut app);
    teleport(&mut app, enemy, GOAL);
    run_until(&mut app, 20, |a| app_state(a) == AppState::Result);
    run_until(&mut app, 200, |a| save_settled(a) && saved(a).games_played() == 1);

    let stage = domain::id::StageId::new("stage_01");
    assert_eq!(saved(&app).record(&stage).map(|r| r.plays()), Some(1));
    assert!(matches!(
        app.world().get_resource::<GameResult>(),
        Some(GameResult::Lost { new_record: true, .. })
    ));
}

#[test]
fn broken_save_does_not_block_play() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("progress.sav"), b"not a save file").expect("write");
    let mut app = app_with_save_dir(dir);
    run_until(&mut app, 3000, |a| app_state(a) == AppState::CharacterSelect);
    submit(
        &mut app,
        Request::Start { character: PusherCharacterId::new("iron_bear") },
    );
    run_until(&mut app, 10, |a| phase(a) == Some(GamePhase::Running));
    run_until(&mut app, 200, save_settled);
    assert_eq!(
        saved(&app).last_character(),
        Some(&PusherCharacterId::new("iron_bear"))
    );
}

/// 書込中・未書き込みの保存は、終了時に書き切ってから終わる。
#[test]
fn pending_save_is_flushed_on_exit() {
    let mut app = started();
    app.world_mut().write_message(AppExit::Success);
    app.update();
    assert!(app.world().resource::<SaveJob>().is_idle());
    assert_eq!(
        saved(&app).last_character(),
        Some(&PusherCharacterId::new("mop_rabbit"))
    );
}

// ── ソークテスト ──

/// 疑似乱数の入力で長くプレイさせても、実行時エラー（パニック・エラーハンドラ到達）が0で、
/// 提出した指図はすべて終端に至り、台帳が空に戻る。
#[test]
fn soak_random_play_has_no_runtime_errors() {
    let mut app = started();
    let mut seed: u64 = 0x5EED_1234_ABCD_0042;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut submitted = 1; // 開始の指図
    for _ in 0..1500 {
        let roll = next();
        {
            let mut intent = app.world_mut().resource_mut::<ControlIntent>();
            intent.fire = roll % 3 != 0;
            intent.aim += ((roll >> 8) % 21) as f32 * 0.01 - 0.1;
        }
        if roll % 97 == 0 {
            submit(&mut app, Request::Menu);
            submitted += 1;
        }
        if roll % 131 == 0 {
            submit(
                &mut app,
                Request::Evolve(domain::evolution::Evolution::LevelUp(
                    domain::id::MedalId::normal(),
                )),
            );
            submitted += 1;
        }
        app.update();
    }
    // 残っている指図も期限までに必ず終端に至る
    for _ in 0..game::infrastructure::requests::TTL_FRAMES {
        app.update();
    }
    assert_eq!(app.world().resource::<RequestLedger>().pending_len(), 0);
    assert_eq!(recorded(&app).outcomes.len(), submitted);
}
