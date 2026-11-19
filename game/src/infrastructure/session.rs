//! セッションの開始・終了と、固定ステップのゲーム進行（基本設計 6章）。
//! ユースケース（application）を呼ぶのはこのファイルの System だけ。

use anyhow::Context;
use application::{
    PhysicsFact, StartGameCommand, advance_waves, launch_medal, level_up, resolve_physics,
    start_game,
};
use avian3d::prelude::{Forces, Mass};
// Bevy 0.20 の prelude は `ContextExt`（`.context()`）を含み `anyhow::Context` と衝突するため、
// anyhow で文脈を付けるこのファイルでは prelude を glob で読み込まない。
use bevy::prelude::{
    App, Commands, Deref, DerefMut, DetectChangesMut, Entity, FixedUpdate, In, IntoScheduleConfigs,
    IntoSystem, MessageReader, MessageWriter, NextState, OnEnter, OnExit, Query, Res, ResMut,
    Resource, SystemSet, Time, Update, Without, debug, error, in_state, resource_exists,
};
use domain::{
    event::DomainEvent,
    id::{GameSessionId, PusherCharacterId, StageId},
    launch::CatapultState,
    navigation::Route,
    pusher::PusherCharacter,
    session::GameSession,
    spec::MedalType,
    stage::Stage,
};

use super::{
    data::{CharacterCatalog, Specs, StageCatalog},
    physics::{self, KILL_DEPTH, KillPlane, PendingFacts},
    world::{self, Enemy, MedalKind, Pusher, PusherMotion},
};
use crate::{
    port::{
        AimModel, AimRequested, Appearance, DomainEventMsg, GameResult, HudModel, LaunchRequested,
        LevelUpRequested, MenuRequested, StartRequested,
    },
    state::AppState,
};

/// 固定ステップ内の処理順（基本設計 6.2）。`.chain()` で明示的に直列化する。
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameplaySet {
    /// 前ステップの物理結果 → 撃破・敗北の判定
    Resolve,
    /// プレイヤーの要求 → 射出・強化
    Command,
    /// 敵の出現・歩行力・Pusherの制御力
    Intent,
}

/// `GameSession` のフィールドは非公開のため、Resource経由でも公開メソッドでしか変更できない。
#[derive(Resource, Deref, DerefMut)]
pub struct ActiveSession(pub GameSession);

#[derive(Resource, Deref)]
pub struct ActiveStage(pub Stage);

#[derive(Resource, Deref)]
pub struct ActiveCharacter(pub PusherCharacter);

#[derive(Resource, Deref, DerefMut, Default)]
pub struct CatapultControl(pub CatapultState);

/// 次に開始するインゲームの選択。
#[derive(Resource, Debug, Clone)]
pub struct Selection {
    pub stage: StageId,
    pub character: PusherCharacterId,
}

#[derive(Resource, Default)]
struct SessionCounter(u64);

pub(super) fn plugin(app: &mut App) {
    use GameplaySet::*;
    app.init_resource::<PendingFacts>()
        .init_resource::<SessionCounter>()
        .configure_sets(
            FixedUpdate,
            (Resolve, Command, Intent)
                .chain()
                .run_if(in_state(AppState::InGame))
                .run_if(resource_exists::<ActiveSession>),
        )
        .add_systems(Update, select_character.run_if(in_state(AppState::CharacterSelect)))
        .add_systems(Update, back_to_menu.run_if(in_state(AppState::Result)))
        .add_systems(OnEnter(AppState::InGame), start_session.pipe(back_to_select_on_error))
        .add_systems(OnExit(AppState::InGame), end_session)
        .add_systems(
            FixedUpdate,
            (
                (physics::collect_facts, resolve).chain().in_set(Resolve),
                (aim, launch, apply_level_up).chain().in_set(Command),
                (spawn_waves, physics::walk_enemies, drive_pushers).in_set(Intent),
            ),
        )
        .add_systems(Update, publish_models.run_if(resource_exists::<ActiveSession>));
}

// ── 状態遷移 ──────────────────────────────────────────────

fn select_character(
    mut requests: MessageReader<StartRequested>,
    stages: Res<StageCatalog>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(request) = requests.read().last() else { return };
    let Some(stage) = stages.default_stage() else { return };
    commands.insert_resource(Selection { stage: stage.clone(), character: request.character.clone() });
    next.set(AppState::InGame);
}

fn back_to_menu(mut requests: MessageReader<MenuRequested>, mut next: ResMut<NextState<AppState>>) {
    if requests.read().count() > 0 {
        next.set(AppState::CharacterSelect);
    }
}

#[allow(clippy::too_many_arguments)]
fn start_session(
    mut commands: Commands,
    selection: Res<Selection>,
    stages: Res<StageCatalog>,
    characters: Res<CharacterCatalog>,
    specs: Res<Specs>,
    mut counter: ResMut<SessionCounter>,
    mut facts: ResMut<PendingFacts>,
    mut events: MessageWriter<DomainEventMsg>,
) -> anyhow::Result<()> {
    counter.0 += 1;
    let command = StartGameCommand {
        session_id: GameSessionId(counter.0),
        stage_id: selection.stage.clone(),
        character_id: selection.character.clone(),
    };
    let out = start_game(&*stages, &*characters, command)
        .with_context(|| format!("failed to start game with {selection:?}"))?;
    let medal = out
        .session
        .medal_spec(MedalType::Normal, &specs)
        .context("normal medal spec is missing")?;

    world::spawn_stage(&mut commands, &out.stage);
    world::spawn_initial_medals(&mut commands, &out.stage, &medal);
    world::spawn_pusher(&mut commands, &out.stage, &out.character.id, &out.pusher);

    facts.0.clear();
    commands.insert_resource(KillPlane(out.stage.geometry().floor_y - KILL_DEPTH));
    commands.insert_resource(ActiveSession(out.session));
    commands.insert_resource(ActiveStage(out.stage));
    commands.insert_resource(ActiveCharacter(out.character));
    commands.insert_resource(CatapultControl::default());
    commands.remove_resource::<GameResult>();
    events.write(DomainEventMsg(out.event));
    Ok(())
}

/// 開始に失敗したらキャラクター選択へ戻す（基本設計 8.5）。
fn back_to_select_on_error(
    In(result): In<anyhow::Result<()>>,
    mut next: ResMut<NextState<AppState>>,
) {
    if let Err(e) = result {
        error!("{e:?}");
        next.set(AppState::CharacterSelect);
    }
}

fn end_session(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ActiveStage>();
    commands.remove_resource::<ActiveCharacter>();
    commands.remove_resource::<CatapultControl>();
    commands.remove_resource::<KillPlane>();
}

// ── Resolve ───────────────────────────────────────────────

fn resolve(
    mut session: ResMut<ActiveSession>,
    mut facts: ResMut<PendingFacts>,
    enemies: Query<(Entity, &Enemy)>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
    mut next: ResMut<NextState<AppState>>,
) {
    let _ = (&mut session, &mut facts, &enemies, &mut commands, &mut events, &mut next);
    let _: fn(&mut GameSession, &mut Vec<PhysicsFact>) -> Vec<DomainEvent> = resolve_physics;
    todo!()
}

// ── Command ───────────────────────────────────────────────

fn aim(
    mut requests: MessageReader<AimRequested>,
    stage: Res<ActiveStage>,
    mut catapult: ResMut<CatapultControl>,
) {
    for request in requests.read() {
        catapult.aim(request.delta, stage.catapult());
    }
}

#[allow(clippy::too_many_arguments)]
fn launch(
    mut requests: MessageReader<LaunchRequested>,
    session: Res<ActiveSession>,
    stage: Res<ActiveStage>,
    mut catapult: ResMut<CatapultControl>,
    specs: Res<Specs>,
    time: Res<Time>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let _ = (&mut requests, &session, &stage, &mut catapult, &specs, &time, &mut commands, &mut events);
    let _ = launch_medal;
    todo!()
}

/// 【仮】強化結果をECSへ1回だけ反映する（基本設計 4.4）。
fn apply_level_up(
    mut requests: MessageReader<LevelUpRequested>,
    mut session: ResMut<ActiveSession>,
    character: Res<ActiveCharacter>,
    specs: Res<Specs>,
    mut medals: Query<(&MedalKind, &mut Mass)>,
    mut pushers: Query<(&mut Pusher, &mut Mass), Without<MedalKind>>,
    mut events: MessageWriter<DomainEventMsg>,
) {
    for LevelUpRequested(upgrade) in requests.read() {
        match level_up(&mut session, &specs, &character, *upgrade) {
            Ok(out) => {
                if let Some((medal, spec)) = out.medal {
                    for (_, mut mass) in medals.iter_mut().filter(|(kind, _)| kind.0 == medal) {
                        mass.0 = spec.mass.get();
                    }
                }
                if let Some(stats) = out.pusher {
                    for (mut pusher, mut mass) in &mut pushers {
                        pusher.stats = stats;
                        mass.0 = stats.mass.get();
                    }
                }
                events.write(DomainEventMsg(out.event));
            },
            Err(rejected) => debug!(%rejected, "level up rejected"),
        }
    }
}

// ── Intent ────────────────────────────────────────────────

fn spawn_waves(
    mut session: ResMut<ActiveSession>,
    stage: Res<ActiveStage>,
    specs: Res<Specs>,
    time: Res<Time>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let out = advance_waves(&mut session, &stage, time.delta());
    for order in out.spawns {
        // 参照整合性はLoadingで検証済み（Stage::new, SpecCatalog::check_stage）
        let (Some(spec), Some(route)) =
            (specs.enemy(&order.kind), Route::new(&stage, order.bridge, order.goal))
        else {
            error!(?order, "spawn order refers to unknown data");
            continue;
        };
        world::spawn_enemy(&mut commands, order.enemy, Appearance::enemy_key(&order.kind), spec, route);
    }
    events.write_batch(out.events.into_iter().map(DomainEventMsg));
}

fn drive_pushers(
    mut pushers: Query<(&Pusher, &mut PusherMotion, Forces)>,
    stage: Res<ActiveStage>,
    character: Res<ActiveCharacter>,
    time: Res<Time>,
) {
    for (pusher, mut motion, mut forces) in &mut pushers {
        let Some(rail) = stage.rail(motion.rail) else { continue };
        physics::drive_pusher(
            pusher,
            &mut motion,
            rail,
            &character.movement,
            time.delta_secs(),
            &mut forces,
        );
    }
}

// ── 読み取りモデル ────────────────────────────────────────

fn publish_models(
    session: Res<ActiveSession>,
    stage: Res<ActiveStage>,
    catapult: Res<CatapultControl>,
    mut hud: ResMut<HudModel>,
    mut aim: ResMut<AimModel>,
) {
    hud.set_if_neq(HudModel {
        wave: session.waves().wave() + 1,
        alive_enemies: session.alive_enemy_count(),
        level: session.level().level(),
    });
    let c = stage.catapult();
    aim.set_if_neq(AimModel { origin: c.position(), direction: c.direction(catapult.yaw()) });
}
