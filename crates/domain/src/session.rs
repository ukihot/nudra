//! GameSession集約（基本設計 3.4）。

mod level;

use std::collections::HashSet;

pub use level::{LevelProgress, Upgrade, UpgradeError};

use crate::{
    event::DomainEvent,
    id::{BridgeId, EnemyId, EnemyTypeId, GameSessionId, GoalId, PusherCharacterId, StageId},
    pusher::{PusherCharacter, PusherStats},
    spec::{MedalSpec, MedalType, SpecCatalog},
    stage::Stage,
    time::Tick,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameStatus {
    Playing,
    /// 【未確定】勝利条件の判定は未実装。
    Won,
    Lost {
        enemy: EnemyId,
        goal: GoalId,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GameSessionError {
    #[error("session is not in progress ({0:?})")]
    NotPlaying(GameStatus),
    #[error(transparent)]
    Upgrade(#[from] UpgradeError),
}

/// 敵の出現指示。EnemyIdは発行済み。
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnOrder {
    pub enemy: EnemyId,
    pub kind: EnemyTypeId,
    pub bridge: BridgeId,
    pub goal: GoalId,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaveProgress {
    wave: usize,
    /// 現Waveを開始したTick。出現は「開始Tick + 出現オフセット ≦ 現在のTick」で決まる
    started_at: Tick,
    next: usize,
}

impl WaveProgress {
    /// 0始まりのWave番号。
    pub fn wave(&self) -> usize {
        self.wave
    }
}

#[derive(Debug, Clone)]
pub struct GameSession {
    id: GameSessionId,
    stage_id: StageId,
    character_id: PusherCharacterId,
    status: GameStatus,
    waves: WaveProgress,
    level: LevelProgress,
    /// 退場の二重判定を防ぐためのID集合。位置などの物理状態は持たない。
    alive_enemies: HashSet<EnemyId>,
    next_enemy_id: u64,
    /// 撃破数（セーブの記録に使う）
    defeated: u32,
}

impl GameSession {
    pub fn start(
        id: GameSessionId,
        stage: &Stage,
        character: &PusherCharacter,
    ) -> (Self, DomainEvent) {
        let session = Self {
            id,
            stage_id: stage.id().clone(),
            character_id: character.id.clone(),
            status: GameStatus::Playing,
            waves: WaveProgress::default(),
            level: LevelProgress::initial(),
            alive_enemies: HashSet::new(),
            next_enemy_id: 1,
            defeated: 0,
        };
        let event = DomainEvent::GameStarted {
            session: id,
            stage: session.stage_id.clone(),
            character: session.character_id.clone(),
        };
        (session, event)
    }

    pub fn id(&self) -> GameSessionId {
        self.id
    }
    pub fn stage_id(&self) -> &StageId {
        &self.stage_id
    }
    pub fn character_id(&self) -> &PusherCharacterId {
        &self.character_id
    }
    pub fn status(&self) -> GameStatus {
        self.status
    }
    pub fn waves(&self) -> &WaveProgress {
        &self.waves
    }
    pub fn level(&self) -> &LevelProgress {
        &self.level
    }
    pub fn alive_enemy_count(&self) -> usize {
        self.alive_enemies.len()
    }
    pub fn defeated(&self) -> u32 {
        self.defeated
    }

    pub fn ensure_playing(&self) -> Result<(), GameSessionError> {
        match self.status {
            GameStatus::Playing => Ok(()),
            other => Err(GameSessionError::NotPlaying(other)),
        }
    }

    /// 現在のTickまでに出現すべき敵を出す。Playing以外では何もしない。
    /// 現Waveの出現が終わり、かつ生存敵がいなくなったら、そのTickから次のWaveを始める。
    pub fn advance(&mut self, stage: &Stage, now: Tick) -> Vec<SpawnOrder> {
        if self.ensure_playing().is_err() {
            return Vec::new();
        }
        let waves = stage.wave_plan().waves();
        let Some(wave) = waves.get(self.waves.wave) else {
            return Vec::new();
        };
        let started = self.waves.started_at;
        let pending = &wave.spawns()[self.waves.next..];
        let due = pending
            .iter()
            .take_while(|s| started + s.at <= now)
            .count();
        let orders = pending[..due]
            .iter()
            .map(|s| {
                let enemy = EnemyId(self.next_enemy_id);
                self.next_enemy_id += 1;
                self.alive_enemies.insert(enemy);
                SpawnOrder {
                    enemy,
                    kind: s.enemy.clone(),
                    bridge: s.bridge,
                    goal: s.goal,
                }
            })
            .collect();
        self.waves.next += due;

        if self.waves.next >= wave.spawns().len()
            && self.alive_enemies.is_empty()
            && self.waves.wave + 1 < waves.len()
        {
            self.waves = WaveProgress {
                wave: self.waves.wave + 1,
                started_at: now,
                next: 0,
            };
        }
        orders
    }

    /// 物理由来の結果。終了後・退場済みの敵は正常な遅れとして無視する（G3, G4）。
    pub fn enemy_fell(&mut self, enemy: EnemyId) -> Option<DomainEvent> {
        if self.ensure_playing().is_err() || !self.alive_enemies.remove(&enemy) {
            return None;
        }
        self.defeated = self.defeated.saturating_add(1);
        Some(DomainEvent::EnemyDefeated { enemy })
    }

    /// Goal到達。成立すると同じ呼び出しの中でLostへ遷移する（G2）。
    pub fn enemy_reached_goal(&mut self, enemy: EnemyId, goal: GoalId) -> Vec<DomainEvent> {
        if self.ensure_playing().is_err() || !self.alive_enemies.remove(&enemy) {
            return Vec::new();
        }
        self.status = GameStatus::Lost { enemy, goal };
        vec![
            DomainEvent::EnemyReachedGoal { enemy, goal },
            DomainEvent::GameLost { enemy, goal },
        ]
    }

    /// 成長補正済みのメダル定義。
    pub fn medal_spec(&self, medal: MedalType, specs: &SpecCatalog) -> Option<MedalSpec> {
        specs
            .medal(medal)
            .map(|base| self.level.medal_spec(medal, *base))
    }

    /// 成長補正済みのPusher能力値。
    pub fn pusher_stats(&self, character: &PusherCharacter) -> PusherStats {
        self.level.pusher_stats(character.base_stats)
    }

    /// 強化後の値を検証して確定する。失敗時は状態を変更しない。
    pub fn apply_level_up(
        &mut self,
        upgrade: Upgrade,
        specs: &SpecCatalog,
        character: &PusherCharacter,
    ) -> Result<DomainEvent, GameSessionError> {
        self.ensure_playing()?;
        let level = self
            .level
            .apply(upgrade, specs, character.base_stats)?;
        Ok(DomainEvent::LevelUpCompleted { level, upgrade })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        positive::{Positive, fixture::p},
        pusher::MovementPattern,
        spec::{SpecialKind, fixture::catalog},
        stage::{
            StageParts, Wave, WavePlan,
            fixture::{parts, spawn, stage},
        },
    };

    fn character() -> PusherCharacter {
        PusherCharacter {
            id: PusherCharacterId::new("rabbit"),
            name: "Rabbit".into(),
            base_stats: PusherStats {
                speed: p(2.0),
                mass: p(10.0),
                push_force: p(100.0),
                push_width: p(1.5),
            },
            movement: MovementPattern::PingPong { pause_at_ends: 0.3 },
        }
    }

    #[fixture]
    fn started(stage: Stage) -> (GameSession, Stage) {
        let (session, _) = GameSession::start(GameSessionId(1), &stage, &character());
        (session, stage)
    }

    #[rstest]
    fn start_emits_game_started_with_initial_growth(stage: Stage) {
        let (session, event) = GameSession::start(GameSessionId(7), &stage, &character());
        assert_eq!(
            event,
            DomainEvent::GameStarted {
                session: GameSessionId(7),
                stage: StageId::new("test"),
                character: PusherCharacterId::new("rabbit"),
            }
        );
        assert_eq!(session.status(), GameStatus::Playing);
        assert_eq!(*session.level(), LevelProgress::initial());
        assert_eq!(session.ensure_playing(), Ok(()));
    }

    #[rstest]
    fn tick_spawns_enemies_on_schedule(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let first = session.advance(&stage, Tick(30));
        assert_eq!(first.iter().map(|o| o.enemy).collect::<Vec<_>>(), [EnemyId(1)]);
        assert!(session.advance(&stage, Tick(59)).is_empty());
        assert_eq!(session.advance(&stage, Tick(60))[0].enemy, EnemyId(2));
        assert_eq!(session.alive_enemy_count(), 2);
    }

    #[rstest]
    fn next_wave_starts_after_all_enemies_leave(parts: StageParts) {
        let mut parts = parts;
        parts.wave_plan =
            WavePlan::new(vec![Wave::new(vec![spawn(0, 1, 1)]), Wave::new(vec![spawn(0, 1, 1)])]);
        let stage = Stage::new(parts).expect("valid");
        let (mut session, _) = GameSession::start(GameSessionId(1), &stage, &character());

        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        assert_eq!(session.waves().wave(), 0, "enemy still alive");
        session.enemy_fell(enemy);
        assert!(session.advance(&stage, Tick(0)).is_empty());
        assert_eq!(session.waves().wave(), 1);
        assert_eq!(session.advance(&stage, Tick(0))[0].enemy, EnemyId(2));
    }

    /// 次のWaveの出現時刻は、そのWaveを始めたTickからの相対で決まる。
    #[rstest]
    fn next_wave_timing_is_relative_to_its_start(parts: StageParts) {
        let mut parts = parts;
        parts.wave_plan = WavePlan::new(vec![
            Wave::new(vec![spawn(0, 1, 1)]),
            Wave::new(vec![spawn(30, 1, 1)]),
        ]);
        let stage = Stage::new(parts).expect("valid");
        let (mut session, _) = GameSession::start(GameSessionId(1), &stage, &character());

        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_fell(enemy);
        assert!(
            session.advance(&stage, Tick(100)).is_empty(),
            "wave 2 starts at tick 100"
        );
        assert!(session.advance(&stage, Tick(129)).is_empty());
        assert_eq!(session.advance(&stage, Tick(130)).len(), 1);
    }

    #[rstest]
    fn enemy_reaching_goal_loses_the_game(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        assert_eq!(
            session.enemy_reached_goal(enemy, GoalId(1)),
            [
                DomainEvent::EnemyReachedGoal { enemy, goal: GoalId(1) },
                DomainEvent::GameLost { enemy, goal: GoalId(1) }
            ]
        );
        assert_eq!(session.status(), GameStatus::Lost { enemy, goal: GoalId(1) });
    }

    #[rstest]
    fn results_and_requests_after_loss_are_ignored_or_rejected(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        session.advance(&stage, Tick(60));
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        let lost = session.status();

        assert!(
            session
                .enemy_reached_goal(EnemyId(2), GoalId(1))
                .is_empty()
        );
        assert_eq!(session.enemy_fell(EnemyId(2)), None);
        assert!(session.advance(&stage, Tick(600)).is_empty());
        assert_eq!(session.ensure_playing(), Err(GameSessionError::NotPlaying(lost)));
        assert_eq!(
            session.apply_level_up(
                Upgrade::PusherSpeed { factor: p(2.0) },
                &catalog(),
                &character()
            ),
            Err(GameSessionError::NotPlaying(lost))
        );
    }

    #[rstest]
    fn enemy_leaves_only_once(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        assert_eq!(session.enemy_fell(enemy), Some(DomainEvent::EnemyDefeated { enemy }));
        assert_eq!(session.enemy_fell(enemy), None);
        assert!(session.enemy_reached_goal(enemy, GoalId(1)).is_empty());
        assert_eq!(session.status(), GameStatus::Playing);
    }

    #[rstest]
    fn unknown_enemy_is_ignored(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        assert_eq!(session.enemy_fell(EnemyId(99)), None);
        assert!(
            session
                .enemy_reached_goal(EnemyId(99), GoalId(1))
                .is_empty()
        );
    }

    /// (Normalメダル質量, Pusher速度, Pusher推力)
    fn effective(session: &GameSession, catalog: &SpecCatalog) -> (f32, f32, f32) {
        let mass = session
            .medal_spec(MedalType::Normal, catalog)
            .expect("normal")
            .mass
            .get();
        let stats = session.pusher_stats(&character());
        (mass, stats.speed.get(), stats.push_force.get())
    }

    #[rstest]
    #[case::medal_mass(Upgrade::MedalMass { medal: MedalType::Normal, factor: p(2.0) }, (2.0, 2.0, 100.0))]
    #[case::pusher_speed(Upgrade::PusherSpeed { factor: p(1.5) }, (1.0, 3.0, 100.0))]
    #[case::pusher_force(Upgrade::PusherForce { factor: p(3.0) }, (1.0, 2.0, 300.0))]
    #[case::shrink(Upgrade::MedalMass { medal: MedalType::Normal, factor: p(0.5) }, (0.5, 2.0, 100.0))]
    fn level_up_changes_effective_stats(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
        #[case] upgrade: Upgrade,
        #[case] expected: (f32, f32, f32),
    ) {
        let (mut session, _) = started;
        assert_eq!(
            session.apply_level_up(upgrade, &catalog, &character()),
            Ok(DomainEvent::LevelUpCompleted { level: 2, upgrade })
        );
        assert_eq!(effective(&session, &catalog), expected);
    }

    #[rstest]
    fn upgrades_compound(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        let twice = Upgrade::MedalMass { medal: MedalType::Normal, factor: p(2.0) };
        let speed = Upgrade::PusherSpeed { factor: p(2.0) };
        for upgrade in [twice, speed, twice, speed] {
            session
                .apply_level_up(upgrade, &catalog, &character())
                .expect("valid upgrade");
        }
        assert_eq!(effective(&session, &catalog), (4.0, 8.0, 100.0));
        assert_eq!(session.level().level(), 5);
    }

    /// 基礎値はメダル質量1.0・速度2.0・推力100.0。メダルは事前に×2して1.0超にしておく
    /// （1.0 × f32::MAX は有限値のため）。
    #[rstest]
    #[case::medal_mass(Upgrade::MedalMass { medal: MedalType::Normal, factor: p(f32::MAX) })]
    #[case::pusher_speed(Upgrade::PusherSpeed { factor: p(f32::MAX) })]
    #[case::pusher_force(Upgrade::PusherForce { factor: p(f32::MAX) })]
    fn upgrade_overflowing_is_rejected_without_state_change(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
        #[case] upgrade: Upgrade,
    ) {
        let (mut session, _) = started;
        let boost = Upgrade::MedalMass { medal: MedalType::Normal, factor: p(2.0) };
        session
            .apply_level_up(boost, &catalog, &character())
            .expect("valid upgrade");
        let before = (session.level().clone(), effective(&session, &catalog));
        assert_eq!(
            session.apply_level_up(upgrade, &catalog, &character()),
            Err(GameSessionError::Upgrade(UpgradeError::Overflow(upgrade)))
        );
        assert_eq!((session.level().clone(), effective(&session, &catalog)), before);
    }

    #[rstest]
    fn upgrade_of_medal_without_spec_is_rejected(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        let fire = MedalType::Special(SpecialKind::Fire);
        assert_eq!(
            session.apply_level_up(
                Upgrade::MedalMass { medal: fire, factor: p(2.0) },
                &catalog,
                &character()
            ),
            Err(GameSessionError::Upgrade(UpgradeError::MissingMedalSpec(fire)))
        );
        assert_eq!(session.level().level(), 1);
    }

    #[rstest]
    fn medal_without_spec_is_none(started: (GameSession, Stage), catalog: SpecCatalog) {
        assert_eq!(
            started
                .0
                .medal_spec(MedalType::Special(SpecialKind::Fire), &catalog),
            None
        );
    }

    fn upgrade() -> impl Strategy<Value = Upgrade> {
        let factor = any::<f32>().prop_filter_map("positive", |v| Positive::new(v).ok());
        prop_oneof![
            factor
                .clone()
                .prop_map(|factor| Upgrade::MedalMass { medal: MedalType::Normal, factor }),
            factor
                .clone()
                .prop_map(|factor| Upgrade::PusherSpeed { factor }),
            factor.prop_map(|factor| Upgrade::PusherForce { factor }),
        ]
    }

    #[derive(Debug, Clone)]
    enum Op {
        Fell(u64),
        Goal(u64),
    }

    #[rstest]
    fn defeated_counts_each_enemy_once(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        assert_eq!(session.defeated(), 0);
        session.advance(&stage, Tick(60));
        session.enemy_fell(EnemyId(1));
        session.enemy_fell(EnemyId(1));
        session.enemy_fell(EnemyId(99));
        assert_eq!(session.defeated(), 1);
        session.enemy_fell(EnemyId(2));
        assert_eq!(session.defeated(), 2);
    }

    proptest! {
        /// どんな順序で物理結果が届いても、各敵の退場イベントは高々1回（G4）。
        #[test]
        fn each_enemy_leaves_at_most_once(ops in prop::collection::vec(
            prop_oneof![(1u64..4).prop_map(Op::Fell), (1u64..4).prop_map(Op::Goal)], 0..30)
        ) {
            let (mut session, stage) = started(stage(parts()));
            session.advance(&stage, Tick(300));
            let mut left = std::collections::HashMap::<EnemyId, usize>::new();
            for op in ops {
                let events = match op {
                    Op::Fell(id) => session.enemy_fell(EnemyId(id)).into_iter().collect(),
                    Op::Goal(id) => session.enemy_reached_goal(EnemyId(id), GoalId(1)),
                };
                for e in events {
                    if let DomainEvent::EnemyDefeated { enemy } | DomainEvent::EnemyReachedGoal { enemy, .. } = e {
                        *left.entry(enemy).or_default() += 1;
                    }
                }
            }
            prop_assert!(left.values().all(|&n| n == 1));
        }

        /// どんな倍率を重ねても、成功した強化はレベルを1上げ、失敗した強化は状態を変えない。
        #[test]
        fn upgrades_are_all_or_nothing(upgrades in prop::collection::vec(upgrade(), 0..20)) {
            let catalog = catalog();
            let (mut session, _) = started(stage(parts()));
            for upgrade in upgrades {
                let before = session.level().clone();
                match session.apply_level_up(upgrade, &catalog, &character()) {
                    Ok(_) => prop_assert_eq!(session.level().level(), before.level() + 1),
                    Err(_) => prop_assert_eq!(session.level(), &before),
                }
            }
        }
    }
}
