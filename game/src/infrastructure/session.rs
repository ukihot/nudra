//! セッションのライフサイクルと、固定ステップのゲーム進行（基本設計 6章）。
//! ユースケース（application）を呼ぶのはこのファイルの System だけ。
//!
//! - 指図（開始・強化・メニュー）は台帳から受け付け、必ず `RequestOutcome` で終端を通知する
//! - 意図（旋回・発射）は `ControlIntent` を毎tick読む
//! - インゲームは `Starting → Running → Ending`。ゲーム進行は `Running` の間だけ動く
//! - ゲーム内の時刻は `GameClock`（Tick）だけ。`Running` の固定ステップごとに1つ進む

use std::{num::NonZeroU32, time::Duration};

use anyhow::Context;
use application::{
    PhysicsFact, StartGameCommand, advance_waves, launch_medal, level_up, record_result,
    resolve_physics, start_game,
};
use avian3d::prelude::{Forces, Mass};
// Bevy 0.20 の prelude は `ContextExt`（`.context()`）を含み `anyhow::Context`
// と衝突するため、 anyhow で文脈を付けるこのファイルでは prelude を glob で読み込まない。
use bevy::prelude::{
    App, Commands, DetectChangesMut, Entity, FixedUpdate, In, IntoScheduleConfigs, IntoSystem,
    Last, MessageReader, MessageWriter, NextState, OnEnter, OnExit, Query, Res, ResMut, Resource,
    State, SystemSet, Time, Update, Without, debug, error, in_state, info, resource_exists,
};
use domain::{
    event::DomainEvent,
    id::{GameSessionId, PusherCharacterId, StageId},
    launch::CatapultState,
    navigation::Route,
    pusher::PusherCharacter,
    session::{GameSession, GameStatus},
    spec::MedalType,
    stage::Stage,
    time::Tick,
};

use super::{
    data::{CharacterCatalog, Specs, StageCatalog},
    physics::{self, KILL_DEPTH, KillPlane, PendingFacts},
    progress::{Progress, SaveJob},
    requests::{RequestKind, RequestLedger, outcome},
    world::{self, Enemy, MedalKind, Pusher, PusherMotion},
};
use crate::{
    port::{
        AimModel, Appearance, ControlIntent, DomainEventMsg, GameResult, HudModel, RejectReason,
        Request, RequestOutcome, RequestStatus, SubmitRequest,
    },
    state::{AppState, GamePhase},
};

/// 固定ステップの周波数。`Time<Fixed>` と、定義データのミリ秒→Tick変換の両方に使う。
pub const TICK_HZ: u32 = 60;
const TICKS_PER_SECOND: NonZeroU32 = NonZeroU32::new(TICK_HZ).expect("TICK_HZ is non-zero");

/// ゲーム世界に1つだけの時計（基本設計 6.5）。セッション開始が `Tick(0)`。
/// `Running` の固定ステップでしか進まないため、一時停止中は進まない。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameClock {
    now: Tick,
    /// このセッションで使った周波数（周波数を将来変えても、記録の意味が変わらないように持つ）
    ticks_per_second: NonZeroU32,
}

impl GameClock {
    pub fn start(ticks_per_second: NonZeroU32) -> Self {
        Self { now: Tick::ZERO, ticks_per_second }
    }

    pub fn now(self) -> Tick {
        self.now
    }

    pub fn advance(&mut self) {
        self.now = self.now.next();
    }

    /// セッション開始からのゲーム内経過時間（一時停止を含まない）。1ns未満は切り捨て。
    pub fn elapsed(self) -> Duration {
        let nanos =
            u128::from(self.now.0) * 1_000_000_000 / u128::from(self.ticks_per_second.get());
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// 固定ステップ内の処理順（基本設計 6.2）。`.chain()` で明示的に直列化する。
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameplaySet {
    /// 前ステップの物理結果 → 撃破・敗北の判定
    Resolve,
    /// 指図（強化）の受付と、意図（旋回・発射）の反映
    Command,
    /// 敵の出現・歩行力・Pusherの制御力
    Intent,
    /// 時計を1つ進める（このtickの処理はすべて同じTickを見る）
    Advance,
}

/// セッションに必要な値を1つにまとめる。「一部だけ存在する」時間帯を作らない（基本設計 6.4）。
/// `GameSession` のフィールドは非公開のため、ここからでも公開メソッドでしか変更できない。
#[derive(Resource)]
pub struct ActiveGame {
    pub session: GameSession,
    pub stage: Stage,
    pub character: PusherCharacter,
    pub catapult: CatapultState,
}

/// 次に開始するインゲームの選択（開始の指図を受け付けたときに決まる）。
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
        .init_resource::<RequestLedger>()
        .configure_sets(
            FixedUpdate,
            (Resolve, Command, Intent, Advance).chain().run_if(in_state(GamePhase::Running)),
        )
        // 指図: 記帳 → 状態ごとの受付 → フレームの終わりに状態違い・期限切れを判定
        .add_systems(Update, register_requests)
        .add_systems(
            Update,
            accept_start.after(register_requests).run_if(in_state(AppState::CharacterSelect)),
        )
        .add_systems(Update, accept_menu.after(register_requests).run_if(in_state(AppState::Result)))
        .add_systems(Last, close_requests)
        // セッションのライフサイクル
        .add_systems(OnEnter(GamePhase::Starting), start_session.pipe(back_to_select_on_error))
        .add_systems(OnEnter(GamePhase::Ending), finish_session)
        .add_systems(OnExit(AppState::InGame), end_session)
        // ゲーム進行
        .add_systems(
            FixedUpdate,
            (
                (physics::collect_facts, resolve).chain().in_set(Resolve),
                (accept_level_up, apply_intent).chain().in_set(Command),
                (spawn_waves, physics::walk_enemies, drive_pushers).in_set(Intent),
                advance_clock.in_set(Advance),
            ),
        )
        .add_systems(Update, publish_models.run_if(resource_exists::<ActiveGame>));
}

// ── 指図 ──────────────────────────────────────────────────

fn register_requests(
    mut submitted: MessageReader<SubmitRequest>,
    mut ledger: ResMut<RequestLedger>,
) {
    for SubmitRequest(request) in submitted.read() {
        let id = ledger.submit(request.clone());
        debug!(?id, ?request, "request submitted");
    }
}

/// 終端の通知とログを1か所にまとめる。
fn report(
    outcomes: impl IntoIterator<Item = RequestOutcome>,
    writer: &mut MessageWriter<RequestOutcome>,
) {
    for o in outcomes {
        match &o.status {
            RequestStatus::Completed => {
                debug!(id = ?o.id, request = ?o.request, "request completed")
            },
            status => debug!(id = ?o.id, request = ?o.request, ?status, "request not completed"),
        }
        writer.write(o);
    }
}

/// フレームの終わり: 今の状態で受け付けない指図は却下、期限を過ぎたものは期限切れにする。
fn close_requests(
    mut ledger: ResMut<RequestLedger>,
    app_state: Res<State<AppState>>,
    phase: Option<Res<State<GamePhase>>>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    let app_state = *app_state.get();
    let phase = phase.map(|p| *p.get());
    let closed = ledger.end_frame(|kind| match kind {
        RequestKind::Start => app_state == AppState::CharacterSelect,
        RequestKind::Menu => app_state == AppState::Result,
        // Starting 中に届いた強化は Running まで待たせる（期限内）
        RequestKind::LevelUp => matches!(phase, Some(GamePhase::Starting | GamePhase::Running)),
    });
    report(closed, &mut outcomes);
}

#[allow(clippy::too_many_arguments)]
fn accept_start(
    mut ledger: ResMut<RequestLedger>,
    stages: Res<StageCatalog>,
    characters: Res<CharacterCatalog>,
    mut progress: ResMut<Progress>,
    mut save: ResMut<SaveJob>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    let (accepted, duplicates) = ledger.take_first(RequestKind::Start);
    report(duplicates, &mut outcomes);
    let Some((id, request)) = accepted else {
        return;
    };
    let Request::Start { character } = &request else {
        return;
    };

    // 受付で実行可否を決める（実行段階に失敗し得る処理を残さない）
    let status = match (stages.default_stage(), characters.0.contains_key(character)) {
        (Some(stage), true) => {
            commands
                .insert_resource(Selection { stage: stage.clone(), character: character.clone() });
            progress.select_character(character.clone());
            save.request(&progress);
            next.set(AppState::InGame);
            RequestStatus::Completed
        },
        (None, _) => RequestStatus::Rejected(RejectReason::Invalid("no stage is loaded".into())),
        (_, false) => {
            RequestStatus::Rejected(RejectReason::Invalid(format!("unknown character {character}")))
        },
    };
    report([outcome(id, request, status)], &mut outcomes);
}

fn accept_menu(
    mut ledger: ResMut<RequestLedger>,
    mut next: ResMut<NextState<AppState>>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    let (accepted, duplicates) = ledger.take_first(RequestKind::Menu);
    report(duplicates, &mut outcomes);
    if let Some((id, request)) = accepted {
        next.set(AppState::CharacterSelect);
        report([outcome(id, request, RequestStatus::Completed)], &mut outcomes);
    }
}

// ── セッションのライフサイクル ─────────────────────────────

#[allow(clippy::too_many_arguments)]
fn start_session(
    mut commands: Commands,
    selection: Res<Selection>,
    stages: Res<StageCatalog>,
    characters: Res<CharacterCatalog>,
    specs: Res<Specs>,
    mut counter: ResMut<SessionCounter>,
    mut facts: ResMut<PendingFacts>,
    mut phase: ResMut<NextState<GamePhase>>,
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
    commands.insert_resource(GameClock::start(TICKS_PER_SECOND));
    commands.insert_resource(ActiveGame {
        session: out.session,
        stage: out.stage,
        character: out.character,
        catapult: CatapultState::default(),
    });
    commands.remove_resource::<GameResult>();
    events.write(DomainEventMsg(out.event));
    phase.set(GamePhase::Running);
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

/// 結果を記録して保存を依頼し、リザルトへ移る。
fn finish_session(
    game: Res<ActiveGame>,
    clock: Res<GameClock>,
    mut progress: ResMut<Progress>,
    mut save: ResMut<SaveJob>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppState>>,
) {
    let session = &game.session;
    if let GameStatus::Lost { enemy, goal } = session.status() {
        let record = record_result(&mut progress, session);
        if record.is_ok() {
            save.request(&progress);
        }
        let (best_defeated, new_record) =
            record.map_or((session.defeated(), false), |r| (r.best_defeated, r.new_record));
        let play_time = clock.elapsed();
        info!(
            defeated = session.defeated(),
            best_defeated,
            new_record,
            ?play_time,
            "game over"
        );
        commands.insert_resource(GameResult::Lost {
            enemy,
            goal,
            defeated: session.defeated(),
            best_defeated,
            new_record,
            play_time,
        });
    }
    next.set(AppState::Result);
}

fn end_session(mut commands: Commands, mut intent: ResMut<ControlIntent>) {
    commands.remove_resource::<ActiveGame>();
    commands.remove_resource::<GameClock>();
    commands.remove_resource::<KillPlane>();
    // 押しっぱなしの意図を次のセッションへ持ち越さない
    *intent = ControlIntent::default();
}

// ── Resolve ───────────────────────────────────────────────

fn resolve(
    mut game: ResMut<ActiveGame>,
    mut facts: ResMut<PendingFacts>,
    enemies: Query<(Entity, &Enemy)>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
    mut phase: ResMut<NextState<GamePhase>>,
) {
    let _ = (
        &mut game,
        &mut facts,
        &enemies,
        &mut commands,
        &mut events,
        &mut phase,
    );
    let _: fn(&mut GameSession, &mut Vec<PhysicsFact>) -> Vec<DomainEvent> = resolve_physics;
    todo!()
}

// ── Command ───────────────────────────────────────────────

/// 【仮】強化の指図を受け付け、結果をECSへ1回だけ反映する（基本設計 4.4）。
#[allow(clippy::too_many_arguments)]
fn accept_level_up(
    mut ledger: ResMut<RequestLedger>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut medals: Query<(&MedalKind, &mut Mass)>,
    mut pushers: Query<(&mut Pusher, &mut Mass), Without<MedalKind>>,
    mut events: MessageWriter<DomainEventMsg>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    for (id, request) in ledger.take_all(RequestKind::LevelUp) {
        let Request::LevelUp(upgrade) = request else {
            continue;
        };
        let ActiveGame { session, character, .. } = &mut *game;
        let status = match level_up(session, &specs, character, upgrade) {
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
                RequestStatus::Completed
            },
            Err(e) => RequestStatus::Rejected(RejectReason::Invalid(e.to_string())),
        };
        report([outcome(id, request, status)], &mut outcomes);
    }
}

/// 意図の反映: 旋回量を適用し、発射の意図があれば発射条件（クールダウン）を評価する。
fn apply_intent(
    mut intent: ResMut<ControlIntent>,
    mut game: ResMut<ActiveGame>,
    clock: Res<GameClock>,
    specs: Res<Specs>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let _ = (&mut intent, &mut game, &clock, &specs, &mut commands, &mut events);
    let _ = launch_medal;
    todo!()
}

// ── Intent ────────────────────────────────────────────────

fn spawn_waves(
    mut game: ResMut<ActiveGame>,
    clock: Res<GameClock>,
    specs: Res<Specs>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
) {
    let ActiveGame { session, stage, .. } = &mut *game;
    let out = advance_waves(session, stage, clock.now());
    for order in out.spawns {
        // 参照整合性はLoadingで検証済み（Stage::new, SpecCatalog::check_stage）
        let (Some(spec), Some(route)) =
            (specs.enemy(&order.kind), Route::new(stage, order.bridge, order.goal))
        else {
            error!(?order, "spawn order refers to unknown data");
            continue;
        };
        world::spawn_enemy(
            &mut commands,
            order.enemy,
            Appearance::enemy_key(&order.kind),
            spec,
            route,
        );
    }
    events.write_batch(out.events.into_iter().map(DomainEventMsg));
}

fn drive_pushers(
    mut pushers: Query<(&Pusher, &mut PusherMotion, Forces)>,
    game: Res<ActiveGame>,
    time: Res<Time>,
) {
    for (pusher, mut motion, mut forces) in &mut pushers {
        let Some(rail) = game.stage.rail(motion.rail) else {
            continue;
        };
        physics::drive_pusher(
            pusher,
            &mut motion,
            rail,
            &game.character.movement,
            time.delta_secs(),
            &mut forces,
        );
    }
}

fn advance_clock(mut clock: ResMut<GameClock>) {
    clock.advance();
}

// ── 読み取りモデル ────────────────────────────────────────

fn publish_models(game: Res<ActiveGame>, mut hud: ResMut<HudModel>, mut aim: ResMut<AimModel>) {
    let session = &game.session;
    hud.set_if_neq(HudModel {
        wave: session.waves().wave() + 1,
        alive_enemies: session.alive_enemy_count(),
        level: session.level().level(),
    });
    let c = game.stage.catapult();
    aim.set_if_neq(AimModel {
        origin: c.position(),
        direction: c.direction(game.catapult.yaw()),
    });
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn clock(hz: u32, ticks: u64) -> GameClock {
        let mut clock = GameClock::start(NonZeroU32::new(hz).expect("non-zero"));
        (0..ticks).for_each(|_| clock.advance());
        clock
    }

    #[rstest]
    #[case::start(60, 0, Duration::ZERO)]
    #[case::one_and_a_half_seconds(60, 90, Duration::from_millis(1500))]
    #[case::fraction(60, 1, Duration::from_nanos(16_666_666))]
    #[case::other_rate(50, 90, Duration::from_millis(1800))]
    fn elapsed_uses_the_session_tick_rate(
        #[case] hz: u32,
        #[case] ticks: u64,
        #[case] expected: Duration,
    ) {
        let clock = clock(hz, ticks);
        assert_eq!(clock.now(), Tick(ticks));
        assert_eq!(clock.elapsed(), expected);
    }
}
