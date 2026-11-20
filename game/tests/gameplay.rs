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
        world::{Enemy, Medal, Pusher},
    },
    port::{
        CharacterRoster, ControlIntent, DomainEventMsg, GameResult, RejectReason, Request,
        RequestOutcome, RequestStatus, SubmitRequest,
    },
    state::{AppState, GamePhase},
};

/// stage_01 の初期メダル数（不規則な俵積みを個別の位置として定義）
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
    cues: Vec<game::port::AbilityCue>,
}

fn record(
    mut events: MessageReader<DomainEventMsg>,
    mut outcomes: MessageReader<RequestOutcome>,
    mut cues: MessageReader<game::port::AbilityCue>,
    mut recorded: ResMut<Recorded>,
) {
    recorded
        .events
        .extend(events.read().map(|m| m.0.clone()));
    recorded.outcomes.extend(outcomes.read().cloned());
    recorded.cues.extend(cues.read().cloned());
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
    app_with_setup(dir, |_| {})
}

fn app_with_setup(dir: tempfile::TempDir, setup: impl FnOnce(&mut App)) -> App {
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
    setup(&mut app);
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
    run_until(&mut app, 3000, |a| app_state(a) == AppState::Title);
    submit(&mut app, Request::Navigate(game::port::OutgameScreen::Preparation));
    run_until(&mut app, 10, |a| app_state(a) == AppState::CharacterSelect);
    *app.world_mut().resource_mut::<Recorded>() = Recorded::default();
    app
}

fn started() -> App {
    let mut app = loaded();
    submit(
        &mut app,
        Request::Start {
            character: PusherCharacterId::new("mop_rabbit"),
            stage: domain::id::StageId::new("stage_01"),
            difficulty: domain::difficulty::Difficulty::EASY,
        },
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
    assert_eq!(ids, ["ao_umiushi", "iron_bear", "mop_rabbit"]);
}

#[test]
fn boot_opens_title_with_exactly_three_menu_actions() {
    let mut app = app_with_setup(tempfile::tempdir().expect("dir"), |app| {
        app.init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(game::presentation::outgame::plugin);
    });
    run_until(&mut app, 3000, |a| app_state(a) == AppState::Title);
    let actions: Vec<_> = app
        .world_mut()
        .query::<&game::presentation::outgame::MenuAction>()
        .iter(app.world())
        .map(|a| a.0)
        .collect();
    assert_eq!(actions.len(), 3);
    for screen in [
        game::port::OutgameScreen::Preparation,
        game::port::OutgameScreen::Options,
        game::port::OutgameScreen::Credits,
    ] {
        assert!(actions.contains(&screen));
    }
    assert_eq!(count::<Medal>(&mut app), 0);
}

#[test]
fn options_language_is_saved_and_restored_on_restart() {
    use domain::progress::Language;
    use game::port::{OptionsModel, OutgameScreen};
    let mut app = app_with_setup(tempfile::tempdir().expect("dir"), |app| {
        app.init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(bevy_intl::I18nPlugin::with_config(bevy_intl::I18nConfig {
                default_lang: "ja".into(),
                fallback_lang: "en".into(),
                ..default()
            }))
            .add_plugins(game::presentation::outgame::plugin);
    });
    run_until(&mut app, 3000, |a| app_state(a) == AppState::Title);
    submit(&mut app, Request::Navigate(OutgameScreen::Options));
    run_until(&mut app, 10, |a| app_state(a) == AppState::Options);
    submit(&mut app, Request::SetLanguage(Language::English));
    run_until(&mut app, 200, |a| {
        save_settled(a) && saved(a).language() == Language::English
    });
    run_until(&mut app, 10, |a| {
        a.world_mut()
            .query::<&Text>()
            .iter(a.world())
            .any(|text| text.0 == "Options")
    });
    let dir = tempfile::tempdir().expect("dir");
    std::fs::copy(
        app.world().resource::<FileProgressRepository>().path(),
        dir.path().join("progress.sav"),
    )
    .expect("copy save");
    let mut restarted = app_with_save_dir(dir);
    run_until(&mut restarted, 3000, |a| app_state(a) == AppState::Title);
    assert_eq!(
        restarted.world().resource::<OptionsModel>().language,
        Language::English
    );
    submit(&mut app, Request::Navigate(OutgameScreen::Title));
    run_until(&mut app, 10, |a| app_state(a) == AppState::Title);
    submit(&mut app, Request::Navigate(OutgameScreen::Credits));
    run_until(&mut app, 10, |a| app_state(a) == AppState::Credits);
    submit(&mut app, Request::Navigate(OutgameScreen::Title));
    run_until(&mut app, 10, |a| app_state(a) == AppState::Title);
    assert_eq!(count::<Medal>(&mut app), 0);
}

#[test]
fn render_quality_is_rejected_outside_options_and_survives_restart() {
    use domain::progress::RenderQuality;
    use game::port::{OptionsModel, OutgameScreen};
    let dir = tempfile::tempdir().expect("dir");
    let mut app = app_with_save_dir(tempfile::tempdir().expect("dir"));
    run_until(&mut app, 3000, |a| app_state(a) == AppState::Title);
    submit(&mut app, Request::SetRenderQuality(RenderQuality::High));
    app.update();
    assert_eq!(
        app.world().resource::<OptionsModel>().render_quality,
        RenderQuality::Standard
    );
    assert!(
        app.world()
            .resource::<Recorded>()
            .outcomes
            .iter()
            .any(|o| o.request == Request::SetRenderQuality(RenderQuality::High)
                && o.status == RequestStatus::Rejected(RejectReason::WrongState))
    );
    submit(&mut app, Request::Navigate(OutgameScreen::Options));
    run_until(&mut app, 10, |a| app_state(a) == AppState::Options);
    submit(&mut app, Request::SetRenderQuality(RenderQuality::High));
    run_until(&mut app, 200, |a| {
        save_settled(a) && saved(a).render_quality() == RenderQuality::High
    });
    std::fs::copy(
        app.world().resource::<FileProgressRepository>().path(),
        dir.path().join("progress.sav"),
    )
    .expect("copy save");
    let mut restarted = app_with_save_dir(dir);
    run_until(&mut restarted, 3000, |a| app_state(a) == AppState::Title);
    assert_eq!(
        restarted
            .world()
            .resource::<OptionsModel>()
            .render_quality,
        RenderQuality::High
    );
}

#[test]
fn locked_difficulty_and_unknown_stage_are_rejected_without_starting() {
    use domain::{difficulty::Difficulty, id::StageId};
    let mut app = loaded();
    for (stage, difficulty) in [
        (StageId::new("stage_01"), Difficulty::new(2).expect("difficulty")),
        (StageId::new("missing"), Difficulty::EASY),
    ] {
        let outcomes = recorded(&app).outcomes.len();
        submit(
            &mut app,
            Request::Start {
                character: PusherCharacterId::new("mop_rabbit"),
                stage,
                difficulty,
            },
        );
        run_until(&mut app, 10, |a| recorded(a).outcomes.len() > outcomes);
        assert!(matches!(
            recorded(&app).outcomes.last().expect("outcome").status,
            RequestStatus::Rejected(RejectReason::Invalid(_))
        ));
        assert_eq!(app_state(&app), AppState::CharacterSelect);
        assert!(app.world().get_resource::<ActiveGame>().is_none());
    }
}

#[test]
fn start_request_completes_and_builds_the_stage() {
    let mut app = started();
    app.update();
    assert_eq!(count::<Medal>(&mut app), INITIAL_MEDALS);
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
    let start = || Request::Start {
        character: PusherCharacterId::new("mop_rabbit"),
        stage: domain::id::StageId::new("stage_01"),
        difficulty: domain::difficulty::Difficulty::EASY,
    };
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
        Request::Start {
            character: PusherCharacterId::new("nobody"),
            stage: domain::id::StageId::new("stage_01"),
            difficulty: domain::difficulty::Difficulty::EASY,
        },
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
            .query::<(&Medal, &Mass)>()
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
        .push(PhysicsFact::MedalsFell(25.0));
    run_until(&mut app, 5, |a| phase(a) == Some(GamePhase::Evolving));
    let medal = domain::id::MedalCode::normal();
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
        .push(PhysicsFact::MedalsFell(25.0));
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
        Request::Evolve(domain::evolution::Evolution::Acquire(domain::id::MedalCode::new(
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
        .query_filtered::<Entity, With<Medal>>()
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
        .query_filtered::<Entity, With<Medal>>()
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
    use domain::{evolution::Evolution, id::MedalCode};
    let mut app = started();
    let m = |id: &str| MedalCode::new(id);
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
            .query::<&Medal>()
            .iter(app.world())
            .filter(|kind| kind.0.medal() == &m("fire"))
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
        &MedalCode::normal()
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
            .unlocked_difficulty(&domain::id::StageId::new("stage_01"))
            .level(),
        2
    );
    submit(&mut app, Request::Menu);
    run_until(&mut app, 10, |a| app_state(a) == AppState::CharacterSelect);
    submit(
        &mut app,
        Request::Start {
            character: PusherCharacterId::new("mop_rabbit"),
            stage: domain::id::StageId::new("stage_01"),
            difficulty: domain::difficulty::Difficulty::new(2).expect("difficulty"),
        },
    );
    run_until(&mut app, 10, |a| phase(a) == Some(GamePhase::Running));
    assert_eq!(
        app.world()
            .resource::<ActiveGame>()
            .session
            .difficulty()
            .level(),
        2
    );
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
    let medals = count::<Medal>(&mut app);
    set_fire(&mut app, true);
    for _ in 0..=COOLDOWN_TICKS * 2 {
        app.update();
    }
    assert_eq!(launches(&app), 3, "ticks t, t+15, t+30");
    assert_eq!(count::<Medal>(&mut app), medals + 3);
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
    run_until(&mut app, 3000, |a| app_state(a) == AppState::Title);
    submit(&mut app, Request::Navigate(game::port::OutgameScreen::Preparation));
    run_until(&mut app, 10, |a| app_state(a) == AppState::CharacterSelect);
    *app.world_mut().resource_mut::<Recorded>() = Recorded::default();
    submit(
        &mut app,
        Request::Start {
            character: PusherCharacterId::new("iron_bear"),
            stage: domain::id::StageId::new("stage_01"),
            difficulty: domain::difficulty::Difficulty::EASY,
        },
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
                    domain::id::MedalCode::normal(),
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

// ── メダルの能力（効果の部品ごと、基本設計 3.14〜3.16） ──
// 定義データ（game.specs.ron）のサンプルの能力を使う。メダルは本番と同じ `spawn_medal`
// で盤面に出す。

mod abilities {
    use bevy::ecs::system::RunSystemOnce;
    use domain::{
        ability::Happening, evolution::MedalLevel, id::MedalCode, motion::Motion, unit::MedalUnit,
    };
    use game::{
        infrastructure::{
            abilities::{self, AbilityBudget, AbilityEvents, SessionAbort, ZoneField},
            data::Specs,
        },
        port::{Appearance, MedalMotionState},
    };

    use super::*;

    /// 能力の回数・報酬は、演出用の初期配置の崩れ方に依存しない固定盤面で検証する。
    /// 実ステージの不規則な山は、開始テストとランダムプレイのsoakで使う。
    fn started() -> App {
        use domain::stage::{InitialMedalPlacement, Stage, StageParts};
        use game::infrastructure::data::StageCatalog;
        let mut app = loaded();
        let id = domain::id::StageId::new("stage_01");
        {
            let mut catalog = app.world_mut().resource_mut::<StageCatalog>();
            let stage = catalog.0.get(&id).expect("stage");
            let fixture = Stage::new(StageParts {
                id: id.clone(),
                geometry: stage.geometry().clone(),
                catapult: stage.catapult().clone(),
                bridges: stage.bridges().to_vec(),
                abysses: stage.abysses().to_vec(),
                rails: stage.rails().to_vec(),
                goals: stage.goals().to_vec(),
                wave_plan: stage.wave_plan().clone(),
                initial_medals: vec![InitialMedalPlacement::grid(
                    Vec3::new(0.0, 0.05, -4.2),
                    12,
                    7,
                    3,
                    Vec3::new(0.6, 0.1, 0.6),
                )],
            })
            .expect("valid ability fixture");
            catalog.0.insert(id.clone(), fixture);
        }
        submit(
            &mut app,
            Request::Start {
                character: PusherCharacterId::new("mop_rabbit"),
                stage: id,
                difficulty: domain::difficulty::Difficulty::EASY,
            },
        );
        run_until(&mut app, 10, |a| phase(a) == Some(GamePhase::Running));
        app
    }

    fn spawn(app: &mut App, code: &str, level: u8, at: Vec3) -> Entity {
        let code = MedalCode::new(code);
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands,
                      specs: Res<Specs>,
                      clock: Res<GameClock>,
                      mut events: ResMut<AbilityEvents>| {
                    let unit = MedalUnit::new(code.clone(), MedalLevel::new(level).expect("level"));
                    abilities::spawn_medal(
                        &mut commands,
                        &specs,
                        unit,
                        at,
                        Vec3::ZERO,
                        &[Happening::Entered],
                        clock.now(),
                        &mut events,
                    )
                },
            )
            .expect("system ran")
            .expect("spawned")
    }

    /// 初期配置の山の上（落とすと山のメダルにぶつかって着地する）。
    fn above_the_pile(app: &mut App) -> Vec3 {
        let p = app
            .world_mut()
            .query_filtered::<&Position, With<Medal>>()
            .iter(app.world())
            .next()
            .expect("initial medals")
            .0;
        p + Vec3::Y * 0.6
    }

    fn motion(app: &App, medal: Entity) -> Option<Motion> {
        app.world()
            .get::<MedalMotionState>(medal)
            .map(|m| m.0.state())
    }

    /// 山: 盤面に出たとき（常時の能力）に自分へ「不動」の印 → 定義の質量の8倍。
    #[test]
    fn an_always_ability_marks_the_medal_as_it_enters() {
        let mut app = started();
        let mountain = spawn(&mut app, "mountain", 1, Vec3::new(0.0, 2.0, 0.0));
        let base = app
            .world()
            .resource::<Specs>()
            .medal(&MedalCode::new("mountain"), MedalLevel::MIN)
            .expect("mountain")
            .mass
            .get();
        app.update();
        assert_eq!(app.world().get::<Mass>(mountain).map(|m| m.0), Some(base * 8.0));
    }

    /// 幻: 落ちて転がり、止まったら（静止の出来事）ノーマルを1枚生む。挙動ステータスが飛行 →
    /// 静止と進む。
    #[test]
    fn a_dropped_medal_lands_rests_and_fires_its_rest_ability() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let before = count::<Medal>(&mut app);
        let illusion = spawn(&mut app, "illusion", 1, at);
        assert_eq!(motion(&app, illusion), Some(Motion::Flying));
        run_until(&mut app, 900, |a| motion(a, illusion) == Some(Motion::Resting));
        app.update();
        assert_eq!(count::<Medal>(&mut app), before + 2, "itself and one normal medal");
    }

    /// 水: メダルにぶつかったら、周りを濡らす → 摩擦が下がる。
    #[test]
    fn a_collision_marks_nearby_medals_and_lowers_their_friction() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let water = spawn(&mut app, "water", 1, at);
        run_until(&mut app, 120, |a| {
            a.world_mut()
                .query::<(Entity, &Friction)>()
                .iter(a.world())
                .any(|(e, f)| e != water && f.dynamic_coefficient == 0.05)
        });
    }

    /// 炎: 着地したら敵に効く「アチチ」の場を残し、4秒で消える。
    #[test]
    fn a_landing_leaves_a_zone_that_expires() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        spawn(&mut app, "fire", 1, at);
        run_until(&mut app, 120, |a| count::<ZoneField>(a) == 1);
        let key = app
            .world_mut()
            .query_filtered::<&Appearance, With<ZoneField>>()
            .single(app.world())
            .expect("one zone")
            .key
            .clone();
        assert_eq!(key, "zone/hot");
        run_until(&mut app, 300, |a| count::<ZoneField>(a) == 0);
    }

    /// 炎 Lv.4（覚醒）: 止まったら周りを外へ弾く。
    #[test]
    fn an_awakened_ability_pushes_nearby_medals() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let fire = spawn(&mut app, "fire", 4, at);
        run_until(&mut app, 900, |a| motion(a, fire) == Some(Motion::Resting));
        let fastest = app
            .world_mut()
            .query_filtered::<(Entity, &LinearVelocity), With<Medal>>()
            .iter(app.world())
            .filter(|(e, _)| *e != fire)
            .map(|(_, v)| v.length())
            .fold(0.0, f32::max);
        assert!(fastest > 1.0, "neighbours were blown away: {fastest}");
    }

    fn gauge(app: &App) -> f32 {
        app.world()
            .resource::<ActiveGame>()
            .session
            .gauge()
            .ratio()
    }

    const BELOW_KILL_PLANE: Vec3 = Vec3::new(0.0, -40.0, 0.0);

    /// ゾンビ: 1回目の落下はカタパルトへ復活して報酬なし。2回目の落下で退場し、
    /// 報酬は1回だけ（4点）。
    #[test]
    fn a_revived_medal_is_rewarded_only_on_its_last_fall() {
        let mut app = started();
        let zombie = spawn(&mut app, "zombie", 1, BELOW_KILL_PLANE);
        let catapult = app
            .world()
            .resource::<ActiveGame>()
            .stage
            .catapult()
            .position();
        run_until(&mut app, 5, |a| {
            a.world()
                .get::<Position>(zombie)
                .is_some_and(|p| p.0.distance(catapult) < 0.5)
        });
        assert_eq!(gauge(&app), 0.0, "no reward for a revived fall");
        teleport(&mut app, zombie, BELOW_KILL_PLANE);
        run_until(&mut app, 5, |a| a.world().get_entity(zombie).is_err());
        app.update();
        assert_eq!(gauge(&app), 0.04, "one reward for the last fall");
    }

    /// 幻が生んだノーマルは報酬資格がない（能力生成は既定で資格なし）。落ちてもゲージは増えない。
    #[test]
    fn spawned_medals_without_a_reward_earn_nothing_when_they_fall() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let illusion = spawn(&mut app, "illusion", 1, at);
        let before: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<Medal>>()
            .iter(app.world())
            .collect();
        run_until(&mut app, 900, |a| motion(a, illusion) == Some(Motion::Resting));
        app.update();
        let child = app
            .world_mut()
            .query_filtered::<Entity, With<Medal>>()
            .iter(app.world())
            .find(|e| !before.contains(e))
            .expect("the spawned child");
        teleport(&mut app, child, BELOW_KILL_PLANE);
        run_until(&mut app, 5, |a| a.world().get_entity(child).is_err());
        app.update();
        assert_eq!(gauge(&app), 0.0);
    }

    fn mass(app: &App, medal: Entity) -> f32 {
        app.world().get::<Mass>(medal).expect("mass").0
    }

    /// 血: 敵が奈落へ落ちて撃破が確定したら、他が落ちた能力で育つ（質量1.5倍）。
    #[test]
    fn an_enemy_falling_notifies_other_fell() {
        let mut app = started();
        let blood = spawn(&mut app, "blood", 1, Vec3::new(0.0, 0.5, 0.0));
        app.update();
        let base = mass(&app, blood);
        let enemy = first_enemy(&mut app);
        teleport(&mut app, enemy, ABYSS);
        run_until(&mut app, 10, |a| a.world().get_entity(enemy).is_err());
        app.update();
        assert_eq!(mass(&app, blood), base * 1.5);
    }

    /// 血: メダルが落ちたら育つ。復活したメダル（ゾンビの1回目）からは通知しない。
    #[test]
    fn only_medals_that_really_leave_notify_other_fell() {
        let mut app = started();
        let blood = spawn(&mut app, "blood", 1, Vec3::new(0.0, 0.5, 0.0));
        app.update();
        let base = mass(&app, blood);
        let zombie = spawn(&mut app, "zombie", 1, BELOW_KILL_PLANE);
        run_until(&mut app, 5, |a| {
            a.world()
                .get::<Position>(zombie)
                .is_some_and(|p| p.y > 0.0)
        });
        app.update();
        assert_eq!(mass(&app, blood), base, "a revived medal did not leave");
        teleport(&mut app, zombie, BELOW_KILL_PLANE);
        run_until(&mut app, 5, |a| a.world().get_entity(zombie).is_err());
        app.update();
        assert_eq!(mass(&app, blood), base * 1.5);
    }

    fn limit(app: &mut App, edit: impl FnOnce(&mut application::BudgetLimits)) {
        let live = app.world().resource::<AbilityBudget>().live_medals();
        let mut limits = application::BudgetLimits::default();
        edit(&mut limits);
        let budget = application::Budget::new(limits, live).expect("the board fits");
        app.world_mut().insert_resource(AbilityBudget(budget));
    }

    /// 幻: 生成の枠が無ければ、静止しても能力全体を棄却する（子は生まれない）。
    #[test]
    fn an_ability_without_spawn_budget_is_rejected_whole() {
        let mut app = started();
        limit(&mut app, |l| l.spawns = 0);
        let at = above_the_pile(&mut app);
        let illusion = spawn(&mut app, "illusion", 1, at);
        let before = count::<Medal>(&mut app);
        run_until(&mut app, 900, |a| motion(a, illusion) == Some(Motion::Resting));
        app.update();
        assert_eq!(count::<Medal>(&mut app), before, "no child without a spawn budget");
    }

    /// 盤面が満杯なら射出しない（射出の出来事も出ず、クールダウンも始まらない）。
    #[test]
    fn a_full_board_refuses_to_launch() {
        let mut app = started();
        limit(&mut app, |_| {});
        let live = app.world().resource::<AbilityBudget>().live_medals();
        limit(&mut app, |l| l.live_medals = live);
        set_fire(&mut app, true);
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(launches(&app), 0);
    }

    /// 承認後の予期しない失敗（中断の記録）: 勝敗を確定せず、結果を作らずに選択画面へ戻る。
    #[test]
    fn an_aborted_session_returns_to_select_without_a_result() {
        let mut app = started();
        app.world_mut().resource_mut::<SessionAbort>().0 = Some("injected".into());
        run_until(&mut app, 5, |a| app_state(a) == AppState::CharacterSelect);
        assert!(app.world().get_resource::<GameResult>().is_none());
        assert!(!lost(&app));
    }

    fn cues(app: &App, medal: &str) -> usize {
        recorded(app)
            .cues
            .iter()
            .filter(|c| c.medal.as_str() == medal)
            .count()
    }

    /// 爆弾: 3秒後に周りを押し出し、自分は同じ確定で消費される（報酬なし）。演出の通知は1回。
    #[test]
    fn a_bomb_blasts_and_consumes_itself_without_reward() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let bomb = spawn(&mut app, "bomb", 1, at);
        run_until(&mut app, 240, |a| a.world().get_entity(bomb).is_err());
        let fastest = app
            .world_mut()
            .query_filtered::<&LinearVelocity, With<Medal>>()
            .iter(app.world())
            .map(|v| v.length())
            .fold(0.0, f32::max);
        assert!(fastest > 1.0, "the blast pushed the pile: {fastest}");
        assert_eq!(cues(&app, "bomb"), 1);
        assert_eq!(gauge(&app), 0.0, "a consumed medal is not rewarded");
    }

    /// 水分身: 着地で分身を1枚。分身は世代1なので、着地しても増えない。
    #[test]
    fn a_water_clone_spawns_only_one_generation() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let before = count::<Medal>(&mut app);
        spawn(&mut app, "water_clone", 1, at);
        run_until(&mut app, 300, |a| count::<Medal>(a) == before + 2);
        for _ in 0..300 {
            app.update();
        }
        assert_eq!(count::<Medal>(&mut app), before + 2, "the clone cannot clone again");
        assert_eq!(cues(&app, "water_clone"), 1);
    }

    /// 雷: 2秒ごとに放電する（止まっていても、範囲に相手がいれば）。
    #[test]
    fn lightning_discharges_periodically() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        spawn(&mut app, "lightning", 1, at);
        run_until(&mut app, 260, |a| cues(a, "lightning") >= 2);
    }

    /// 6種（水・周期の雷・時限の爆弾・水分身・ゾンビ・デジャヴ）を同じ盤面で同時に動かす。
    /// 実行時エラーなし（Fallbackはpanic）、複数の種類が発動し、生存数は上限内。
    #[test]
    fn the_six_prototypes_run_together_on_one_board() {
        let mut app = started();
        let at = above_the_pile(&mut app);
        let six = ["water", "lightning", "bomb", "water_clone", "zombie", "deja_vu"];
        for (i, code) in six.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let offset = Vec3::new((i as f32 - 2.5) * 0.4, 0.3 * i as f32, 0.0);
            spawn(&mut app, code, 1, at + offset);
        }
        for _ in 0..360 {
            app.update();
        }
        let fired: std::collections::HashSet<&str> = recorded(&app)
            .cues
            .iter()
            .map(|c| c.medal.as_str())
            .collect();
        assert!(fired.len() >= 3, "several kinds fired: {fired:?}");
        assert!(fired.contains("bomb") && fired.contains("lightning"), "{fired:?}");
        let live = app.world().resource::<AbilityBudget>().live_medals();
        assert!(live <= application::BudgetLimits::default().live_medals);
    }
    /// 星: 落ちたら（自分が落ちた能力）ゲージ+5。落下そのものの価値1.0 × Easy 4点と合わせて9点。
    #[test]
    fn a_fall_ability_adds_points_with_the_fall() {
        let mut app = started();
        spawn(&mut app, "star", 1, Vec3::new(0.0, -40.0, 0.0));
        let ratio = |a: &App| {
            a.world()
                .resource::<ActiveGame>()
                .session
                .gauge()
                .ratio()
        };
        run_until(&mut app, 5, |a| ratio(a) > 0.0);
        assert_eq!(ratio(&app), 0.09);
    }
}
