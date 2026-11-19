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
    StartGameCommand, advance_waves, evolve, launch_medal, record_result, resolve_physics,
    start_game,
};
use avian3d::prelude::{Collider, Forces, Friction, Mass, Physics, PhysicsTime, Restitution};
// Bevy 0.20 の prelude は `ContextExt`（`.context()`）を含み `anyhow::Context`
// と衝突するため、 anyhow で文脈を付けるこのファイルでは prelude を glob で読み込まない。
use bevy::prelude::{
    App, Commands, DetectChangesMut, Entity, FixedUpdate, In, IntoScheduleConfigs, IntoSystem,
    Last, MessageReader, MessageWriter, NextState, OnEnter, OnExit, Query, Res, ResMut, Resource,
    State, SystemSet, Time, Update, debug, error, in_state, info, resource_exists,
};
use domain::{
    difficulty::Difficulty,
    event::DomainEvent,
    evolution::{Evolution, MedalLevel},
    id::{GameSessionId, MedalId, PusherCharacterId, StageId},
    launch::CatapultState,
    navigation::Route,
    pusher::PusherCharacter,
    session::{GameSession, GameStatus},
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
        AimModel, Appearance, ControlIntent, DomainEventMsg, EvolutionChoice, EvolutionModel,
        GameResult, HudModel, MedalModel, RejectReason, Request, RequestOutcome, RequestStatus,
        Silhouette, SubmitRequest,
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
        .add_systems(Update, accept_evolution.after(register_requests).run_if(in_state(GamePhase::Evolving)))
        .add_systems(Update, accept_medal.after(register_requests).run_if(in_state(GamePhase::Running)))
        .add_systems(Last, close_requests)
        // セッションのライフサイクル
        .add_systems(OnEnter(GamePhase::Starting), start_session.pipe(back_to_select_on_error))
        .add_systems(OnEnter(GamePhase::Ending), finish_session)
        .add_systems(OnEnter(GamePhase::Running), resume_physics)
        .add_systems(OnExit(AppState::InGame), end_session)
        // ゲーム進行
        .add_systems(
            FixedUpdate,
            (
                (physics::collect_facts, resolve).chain().in_set(Resolve),
                apply_intent.in_set(Command).run_if(can_advance),
                (spawn_waves, physics::walk_enemies, drive_pushers).in_set(Intent).run_if(can_advance),
                advance_clock.in_set(Advance).run_if(can_advance),
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
        RequestKind::Evolve => phase == Some(GamePhase::Evolving),
        RequestKind::SelectMedal => phase == Some(GamePhase::Running),
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
        difficulty: Difficulty::EASY,
        seed: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            ^ counter.0,
    };
    let out = start_game(&*stages, &*characters, &specs, command)
        .with_context(|| format!("failed to start game with {selection:?}"))?;
    let medal = out
        .session
        .medal_spec(&MedalId::normal(), &specs)
        .context("normal medal spec is missing")?;

    world::spawn_stage(&mut commands, &out.stage, out.session.difficulty());
    world::spawn_initial_medals(&mut commands, &out.stage, &medal);
    world::spawn_pusher(&mut commands, &out.stage, &out.character.id, &out.pusher);

    facts.0.clear();
    commands.insert_resource(KillPlane(out.stage.geometry().floor_y - KILL_DEPTH));
    commands.insert_resource(GameClock::start(TICKS_PER_SECOND));
    let catapult = CatapultState::new(out.stage.catapult());
    commands.insert_resource(ActiveGame {
        session: out.session,
        stage: out.stage,
        character: out.character,
        catapult,
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
    if session.status() != GameStatus::Playing {
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
        let result = match session.status() {
            GameStatus::Lost { enemy, goal } => GameResult::Lost {
                enemy,
                goal,
                defeated: session.defeated(),
                best_defeated,
                new_record,
                play_time,
            },
            GameStatus::Won => GameResult::Won {
                defeated: session.defeated(),
                best_defeated,
                new_record,
                play_time,
            },
            GameStatus::Playing => return,
        };
        commands.insert_resource(result);
    }
    next.set(AppState::Result);
}

fn end_session(mut commands: Commands, mut intent: ResMut<ControlIntent>) {
    commands.remove_resource::<ActiveGame>();
    commands.remove_resource::<GameClock>();
    commands.remove_resource::<KillPlane>();
    commands.insert_resource(MedalModel::default());
    commands.insert_resource(EvolutionModel::default());
    // 押しっぱなしの意図を次のセッションへ持ち越さない
    *intent = ControlIntent::default();
}

// ── Resolve ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn resolve(
    mut game: ResMut<ActiveGame>,
    mut facts: ResMut<PendingFacts>,
    enemies: Query<(Entity, &Enemy)>,
    mut commands: Commands,
    mut events: MessageWriter<DomainEventMsg>,
    mut phase: ResMut<NextState<GamePhase>>,
    specs: Res<Specs>,
    mut physics_time: ResMut<Time<Physics>>,
) {
    if facts.0.is_empty() {
        return;
    }
    let resolved = resolve_physics(&mut game.session, &mut facts.0, &specs);
    let defeated: Vec<_> = resolved
        .iter()
        .filter_map(|e| match e {
            DomainEvent::EnemyDefeated { enemy } => Some(*enemy),
            _ => None,
        })
        .collect();
    physics::despawn_enemies(&defeated, &enemies, &mut commands);
    if game.session.status() != GameStatus::Playing {
        physics_time.pause();
        phase.set(GamePhase::Ending);
    } else if game.session.gauge().is_full() {
        physics_time.pause();
        phase.set(GamePhase::Evolving);
    }
    events.write_batch(resolved.into_iter().map(DomainEventMsg));
}

// ── Command ───────────────────────────────────────────────

/// 進化の指図を受け付け、物理特性と仮形状を1回だけ更新する。
#[allow(clippy::too_many_arguments)]
fn accept_evolution(
    mut ledger: ResMut<RequestLedger>,
    mut game: ResMut<ActiveGame>,
    specs: Res<Specs>,
    mut medals: Query<(
        &MedalKind,
        &mut Mass,
        &mut Collider,
        &mut Friction,
        &mut Restitution,
        &mut Appearance,
    )>,
    mut events: MessageWriter<DomainEventMsg>,
    mut outcomes: MessageWriter<RequestOutcome>,
    mut phase: ResMut<NextState<GamePhase>>,
) {
    let (accepted, duplicates) = ledger.take_first(RequestKind::Evolve);
    report(duplicates, &mut outcomes);
    if let Some((id, request)) = accepted {
        let Request::Evolve(evolution) = &request else {
            return;
        };
        let status = match evolve(&mut game.session, &specs, evolution.clone()) {
            Ok(out) => {
                if let Some((medal, spec)) = out.medal {
                    for (
                        _,
                        mut mass,
                        mut collider,
                        mut friction,
                        mut restitution,
                        mut appearance,
                    ) in medals.iter_mut().filter(|(kind, ..)| kind.0 == medal)
                    {
                        mass.0 = spec.mass.get();
                        *collider = Collider::cylinder(spec.radius.get(), spec.thickness.get());
                        *friction = Friction::new(spec.friction);
                        *restitution = Restitution::new(spec.restitution);
                        appearance.silhouette = Silhouette::Cylinder {
                            radius: spec.radius.get(),
                            height: spec.thickness.get(),
                        };
                    }
                }
                if game
                    .session
                    .medals()
                    .level(game.catapult.selected())
                    .is_none()
                {
                    game.catapult.select(MedalId::normal());
                }
                phase.set(GamePhase::Running);
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
    let ActiveGame { session, stage, catapult, .. } = &mut *game;
    let aim = std::mem::take(&mut intent.aim);
    if aim != 0.0 {
        catapult.aim(aim, stage.catapult());
    }
    if !(intent.fire || intent.auto_fire) {
        return;
    }
    // 発射条件（クールダウン）が成立していなければ何もしない。却下ではない
    match launch_medal(session, stage, catapult, &specs, clock.now()) {
        Ok(Some(out)) => {
            world::spawn_launched_medal(&mut commands, &out.plan);
            events.write(DomainEventMsg(out.event));
        },
        Ok(None) => {},
        Err(not_launchable) => debug!(%not_launchable, "fire intent ignored"),
    }
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

fn publish_models(
    game: Res<ActiveGame>,
    specs: Res<Specs>,
    mut hud: ResMut<HudModel>,
    mut aim: ResMut<AimModel>,
    mut evolution: ResMut<EvolutionModel>,
    mut medals: ResMut<MedalModel>,
) {
    let session = &game.session;
    hud.set_if_neq(HudModel {
        wave: session.waves().wave() + 1,
        alive_enemies: session.alive_enemy_count(),
        level: session
            .medals()
            .level(game.catapult.selected())
            .map_or(1, |l| u32::from(l.get())),
        evolution_percent: (session.gauge().ratio() * 100.0) as u32,
    });
    let c = game.stage.catapult();
    aim.set_if_neq(AimModel {
        origin: c.position(),
        direction: c.direction(game.catapult.yaw()),
    });
    medals.set_if_neq(MedalModel {
        owned: session
            .medals()
            .owned()
            .map(|(m, _)| m.clone())
            .collect(),
        selected: Some(game.catapult.selected().clone()),
    });
    let mut choices = Vec::new();
    if session.gauge().is_full() {
        for (medal, level) in session.medals().owned() {
            if level < MedalLevel::MAX {
                choices.push(EvolutionChoice {
                    evolution: Evolution::LevelUp(medal.clone()),
                    result: medal.clone(),
                    enabled: true,
                });
            }
        }
        for (a, b, result) in specs.fusions() {
            if session.medals().level(a).is_some() && session.medals().level(b).is_some() {
                choices.push(EvolutionChoice {
                    evolution: Evolution::Fuse(a.clone(), b.clone()),
                    result: result.clone(),
                    enabled: session.medals().level(a) == Some(MedalLevel::MAX)
                        && session.medals().level(b) == Some(MedalLevel::MAX)
                        && session.medals().level(result).is_none(),
                });
            }
        }
        if let Some(medal) = session.evolution_offer() {
            choices.push(EvolutionChoice {
                evolution: Evolution::Acquire(medal.clone()),
                result: medal.clone(),
                enabled: true,
            });
        }
    }
    evolution.set_if_neq(EvolutionModel(choices));
}

fn can_advance(game: Option<Res<ActiveGame>>) -> bool {
    game.is_some_and(|game| {
        game.session.status() == GameStatus::Playing && !game.session.gauge().is_full()
    })
}

fn resume_physics(mut time: ResMut<Time<Physics>>) {
    time.unpause();
}

fn accept_medal(
    mut ledger: ResMut<RequestLedger>,
    mut game: ResMut<ActiveGame>,
    mut outcomes: MessageWriter<RequestOutcome>,
) {
    for (id, request) in ledger.take_all(RequestKind::SelectMedal) {
        let Request::SelectMedal(medal) = &request else {
            continue;
        };
        let status = if game.session.status() != GameStatus::Playing
            || game.session.gauge().is_full()
        {
            RequestStatus::Rejected(RejectReason::WrongState)
        } else if game.session.medals().level(medal).is_some() {
            game.catapult.select(medal.clone());
            RequestStatus::Completed
        } else {
            RequestStatus::Rejected(RejectReason::Invalid(format!("medal is not owned: {medal}")))
        };
        report([outcome(id, request, status)], &mut outcomes);
    }
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
