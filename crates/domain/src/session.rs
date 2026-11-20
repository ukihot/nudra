//! GameSession集約（基本設計 3.4, 3.12）。

use std::collections::HashSet;

use crate::{
    ability::{AbilityRef, Effect, FiredAbility},
    difficulty::Difficulty,
    event::DomainEvent,
    evolution::{Evolution, EvolutionChoice, EvolutionError, EvolutionGauge, MedalCollection},
    id::{
        BridgeId, EnemyId, EnemyTypeId, GameSessionId, GoalId, MedalCode, PusherCharacterId,
        StageId,
    },
    pusher::PusherCharacter,
    random::SessionRng,
    spec::{EvolutionRules, MedalSpec, SpecCatalog},
    stage::{SpawnEntry, Stage, WavePlan},
    time::Tick,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameStatus {
    Playing,
    /// 最終Waveのボスを倒した
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
    #[error("waiting for the player to choose an evolution")]
    EvolutionPending,
    #[error(transparent)]
    Evolution(#[from] EvolutionError),
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
    /// 出した通常の敵の数
    next: usize,
    /// 出したボス（撃破でこのWaveをクリアする）
    boss: Option<EnemyId>,
    /// ボスを倒した。次に進めたTickから次のWaveを始める
    cleared: bool,
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
    difficulty: Difficulty,
    status: GameStatus,
    waves: WaveProgress,
    /// 所持しているメダル種とレベル（インゲームの成長。持ち越さない）
    medals: MedalCollection,
    gauge: EvolutionGauge,
    /// この難易度での、メダル1枚の落下あたりのゲージのポイント
    gauge_points: u32,
    /// New! の候補を選ぶ決定的な乱数（セッションのシードから）
    rng: SessionRng,
    /// 進化画面で提示中の New!（ゲージが満タンになった時点で固定する）
    offer: Option<MedalCode>,
    /// 盤面で直前に発動した能力（デジャヴの再生に使う。再生を含む能力は記録しない）
    last_ability: Option<AbilityRef>,
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
        difficulty: Difficulty,
        rules: &EvolutionRules,
        seed: u64,
    ) -> (Self, DomainEvent) {
        let session = Self {
            id,
            stage_id: stage.id().clone(),
            character_id: character.id.clone(),
            difficulty,
            status: GameStatus::Playing,
            waves: WaveProgress::default(),
            medals: MedalCollection::default(),
            gauge: EvolutionGauge::new(rules.threshold),
            gauge_points: rules.points_per_medal(difficulty),
            rng: SessionRng::seeded(seed),
            offer: None,
            last_ability: None,
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
    pub fn difficulty(&self) -> Difficulty {
        self.difficulty
    }
    pub fn status(&self) -> GameStatus {
        self.status
    }
    pub fn waves(&self) -> &WaveProgress {
        &self.waves
    }
    pub fn medals(&self) -> &MedalCollection {
        &self.medals
    }
    pub fn gauge(&self) -> &EvolutionGauge {
        &self.gauge
    }
    /// 進化画面の New! に提示中の、未所持の素材メダル（全部持っていれば `None`）。
    pub fn evolution_offer(&self) -> Option<&MedalCode> {
        self.offer.as_ref()
    }
    pub fn alive_enemy_count(&self) -> usize {
        self.alive_enemies.len()
    }
    pub fn defeated(&self) -> u32 {
        self.defeated
    }

    /// ゲームが進行できる状態か（Playing で、進化の選択待ちでない）。射出・メダル選択・出現の前提。
    pub fn ensure_running(&self) -> Result<(), GameSessionError> {
        self.ensure_playing()?;
        if self.gauge.is_full() {
            return Err(GameSessionError::EvolutionPending);
        }
        Ok(())
    }

    pub fn ensure_playing(&self) -> Result<(), GameSessionError> {
        match self.status {
            GameStatus::Playing => Ok(()),
            other => Err(GameSessionError::NotPlaying(other)),
        }
    }

    /// 現在のTickまでに出現すべき敵（通常の敵と、終盤のボス）を出す。進行できない間（終了後・
    /// 進化の選択中）は何もしない。 前のWaveのボスを倒していたら、
    /// このTickから次のWaveを始める（出現時刻はこのTickからの相対）。
    pub fn advance(&mut self, stage: &Stage, now: Tick) -> Vec<SpawnOrder> {
        if self.ensure_running().is_err() {
            return Vec::new();
        }
        let waves = stage.wave_plan().waves();
        if self.waves.cleared && self.waves.wave + 1 < waves.len() {
            self.waves = WaveProgress {
                wave: self.waves.wave + 1,
                started_at: now,
                ..Default::default()
            };
        }
        let Some(wave) = waves.get(self.waves.wave) else {
            return Vec::new();
        };
        let started = self.waves.started_at;
        let pending = &wave.spawns()[self.waves.next..];
        let due = pending
            .iter()
            .take_while(|s| started + s.at <= now)
            .count();
        let mut orders: Vec<_> = pending[..due]
            .iter()
            .map(|s| self.spawn(stage, s))
            .collect();
        self.waves.next += due;

        if self.waves.boss.is_none() && started + wave.boss().at <= now {
            let boss = self.spawn(stage, wave.boss());
            self.waves.boss = Some(boss.enemy);
            orders.push(boss);
        }
        orders
    }

    /// 出現エントリのGoalがこの難易度で無効なら、Easy のGoal（必ず1つある）へ向かわせる。
    fn spawn(&mut self, stage: &Stage, entry: &SpawnEntry) -> SpawnOrder {
        let active = stage
            .goal(entry.goal)
            .is_some_and(|g| g.available_from <= self.difficulty);
        let goal = if active {
            entry.goal
        } else {
            stage
                .active_goals(Difficulty::EASY)
                .next()
                .map_or(entry.goal, |g| g.id)
        };
        let enemy = EnemyId(self.next_enemy_id);
        self.next_enemy_id += 1;
        self.alive_enemies.insert(enemy);
        SpawnOrder {
            enemy,
            kind: entry.enemy.clone(),
            bridge: entry.bridge,
            goal,
        }
    }

    /// 物理由来の結果。終了後・退場済みの敵は正常な遅れとして無視する（G3, G4）。
    /// Waveのボスなら、そのWaveをクリアする（最終Waveなら勝利）。
    pub fn enemy_fell(&mut self, enemy: EnemyId) -> Vec<DomainEvent> {
        if self.ensure_playing().is_err() || !self.alive_enemies.remove(&enemy) {
            return Vec::new();
        }
        self.defeated = self.defeated.saturating_add(1);
        let mut events = vec![DomainEvent::EnemyDefeated { enemy }];
        if self.waves.boss == Some(enemy) {
            let wave = self.waves.wave + 1;
            if wave == WavePlan::WAVES {
                self.status = GameStatus::Won;
                events.push(DomainEvent::GameWon);
            } else {
                self.waves.cleared = true;
                events.push(DomainEvent::WaveCleared { wave });
            }
        }
        events
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

    /// メダルが奈落へ落ちた（tickごとの集計）。`value`
    /// は落ちたメダルの価値の合計（ノーマルは1枚1.0、 偽物は0、輝きは2倍。
    /// [`crate::unit::MedalUnit::modifiers`] の `value`）。価値 × 難易度のポイントを
    /// 四捨五入してゲージに足す。物理由来の値なので、非有限・負は0として扱う（エラーにしない）。
    pub fn medals_fell(&mut self, value: f32, specs: &SpecCatalog) -> Vec<DomainEvent> {
        let points = self.points_for(value);
        self.fill_gauge(points, specs)
    }

    /// 能力によるゲージの加算（[`crate::ability::Effect::Gauge`]）。落下と同じく満タンで知らせる。
    pub fn add_gauge(&mut self, points: u32, specs: &SpecCatalog) -> Vec<DomainEvent> {
        self.fill_gauge(points, specs)
    }

    fn points_for(&self, value: f32) -> u32 {
        if !(value.is_finite() && value > 0.0) {
            return 0;
        }
        let points = (f64::from(value) * f64::from(self.gauge_points)).round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if points >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            points as u32
        }
    }

    /// ゲージに足し、満タンになったら知らせる。満タンになった時点で New! の候補
    /// （未所持の素材から1つ）を決めて固定する。
    fn fill_gauge(&mut self, points: u32, specs: &SpecCatalog) -> Vec<DomainEvent> {
        if self.ensure_playing().is_err() || !self.can_evolve(specs) {
            return Vec::new();
        }
        if !self.gauge.fill(points) {
            return Vec::new();
        }
        let unowned = || {
            specs
                .elements()
                .filter(|m| self.medals.level(m).is_none())
        };
        self.offer = self
            .rng
            .pick(unowned().count())
            .and_then(|i| unowned().nth(i))
            .cloned();
        vec![DomainEvent::EvolutionReady]
    }

    /// 素材メダルから1つ（セッションの乱数。同じシードなら同じ順に選ぶ）。生成・変化の効果に使う。
    pub fn random_element(&mut self, specs: &SpecCatalog) -> Option<MedalCode> {
        let i = self.rng.pick(specs.elements().count())?;
        specs.elements().nth(i).cloned()
    }

    /// 進化先が尽きたらゲージを溜めず、選べない進化画面による停止を防ぐ。
    /// New! はゲージが満タンになった時点で決まるため、
    /// ここでは「未所持の素材が残っているか」で見る。
    fn can_evolve(&self, specs: &SpecCatalog) -> bool {
        self.evolution_choices(specs).any(|c| c.available)
            || specs
                .elements()
                .any(|medal| self.medals.level(medal).is_none())
    }

    /// 進化画面の選択肢（レベルアップ → 錬成 → New! の順）。可否は `evolve` と同じ検証で決める。
    /// Lv.4 のレベルアップは出さない。錬成は素材を両方所持していれば、条件不足でも表示する。
    pub fn evolution_choices<'a>(
        &'a self,
        specs: &'a SpecCatalog,
    ) -> impl Iterator<Item = EvolutionChoice> + 'a {
        let owned = |m: &MedalCode| self.medals.level(m).is_some();
        let level_ups = self
            .medals
            .owned()
            .map(|(m, _)| Evolution::LevelUp(m.clone()));
        let fusions = specs
            .fusions()
            .filter(move |(a, b, _)| owned(a) && owned(b))
            .map(|(a, b, _)| Evolution::Fuse(a.clone(), b.clone()));
        let offer = self.offer.iter().cloned().map(Evolution::Acquire);
        level_ups
            .chain(fusions)
            .chain(offer)
            .filter_map(move |evolution| {
                let checked =
                    self.medals
                        .check(&evolution, |a, b| specs.fusion(a, b), self.offer.as_ref());
                let (result, available) = match (&evolution, checked) {
                    (_, Ok(evolved)) => (evolved.medal, true),
                    (Evolution::Fuse(a, b), Err(_)) => (specs.fusion(a, b)?, false),
                    (_, Err(_)) => return None,
                };
                Some(EvolutionChoice { evolution, result, available })
            })
    }

    /// 進化（レベルアップ・錬成・New! の獲得）。ゲージが満タンでなければ受け付けない。
    /// 成功するとゲージを空にし、提示中の New! も取り下げる。失敗時は状態を変えない。
    pub fn evolve(
        &mut self,
        evolution: Evolution,
        specs: &SpecCatalog,
    ) -> Result<DomainEvent, GameSessionError> {
        self.ensure_playing()?;
        if !self.gauge.is_full() {
            return Err(EvolutionError::NotReady.into());
        }
        let evolved =
            self.medals
                .evolve(evolution, |a, b| specs.fusion(a, b), self.offer.as_ref())?;
        self.gauge.reset();
        self.offer = None;
        Ok(DomainEvent::Evolved { medal: evolved.medal, level: evolved.level })
    }

    /// 能力の発動。実行する能力を返し、盤面で直前に発動した能力として記録する。
    /// 再生（[`Effect::Replay`]）を含む能力は記録せず、直前の能力を `replayed` に入れて返す。
    /// 進行できない間（終了後・進化の選択中）と、定義に無い能力は `None`（何も記録しない）。
    pub fn fire_ability<'a>(
        &mut self,
        fired: &AbilityRef,
        specs: &'a SpecCatalog,
    ) -> Option<FiredAbility<'a>> {
        self.ensure_running().ok()?;
        let book = specs.abilities();
        let ability = book
            .abilities(&fired.medal, fired.level)
            .get(fired.index)?;
        if ability.effects.contains(&Effect::Replay) {
            let replayed = self
                .last_ability
                .as_ref()
                .and_then(|last| book.abilities(&last.medal, last.level).get(last.index));
            return Some(FiredAbility { ability, replayed });
        }
        self.last_ability = Some(fired.clone());
        Some(FiredAbility { ability, replayed: None })
    }

    /// 所持しているメダル種の、今のレベルでの物理特性。所持していなければ `None`。
    pub fn medal_spec(&self, medal: &MedalCode, specs: &SpecCatalog) -> Option<MedalSpec> {
        let level = self.medals.level(medal)?;
        specs.medal(medal, level).copied()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        evolution::MedalLevel,
        positive::fixture::p,
        pusher::{MovementPattern, PusherStats},
        spec::fixture::{catalog, fire, parts as spec_parts, sauna, water},
        stage::{
            Goal, StageParts, Wave,
            fixture::{parts, spawn, stage, waves_with_first},
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

    fn start(stage: &Stage, difficulty: Difficulty) -> GameSession {
        let rules = *catalog().evolution();
        GameSession::start(GameSessionId(1), stage, &character(), difficulty, &rules, 1).0
    }

    #[fixture]
    fn started(stage: Stage) -> (GameSession, Stage) {
        (start(&stage, Difficulty::EASY), stage)
    }

    #[rstest]
    fn start_emits_game_started_with_initial_growth(stage: Stage) {
        let rules = *catalog().evolution();
        let (session, event) = GameSession::start(
            GameSessionId(7),
            &stage,
            &character(),
            Difficulty::HARDEST,
            &rules,
            9,
        );
        assert_eq!(
            event,
            DomainEvent::GameStarted {
                session: GameSessionId(7),
                stage: StageId::new("test"),
                character: PusherCharacterId::new("rabbit"),
            }
        );
        assert_eq!(session.status(), GameStatus::Playing);
        assert_eq!(session.difficulty(), Difficulty::HARDEST);
        assert_eq!(*session.medals(), MedalCollection::default());
        assert!(!session.gauge().is_full());
        assert_eq!(session.ensure_playing(), Ok(()));
    }

    // ── Wave ──

    fn ids(orders: &[SpawnOrder]) -> Vec<u64> {
        orders.iter().map(|o| o.enemy.0).collect()
    }

    /// 第1Waveを最後まで出し、その中ボスのIDを返す（通常1・2、中ボス3）。
    fn spawn_first_wave(session: &mut GameSession, stage: &Stage) -> EnemyId {
        let spawned = session.advance(stage, Tick(120));
        spawned.last().expect("boss spawned").enemy
    }

    #[rstest]
    fn regulars_then_boss_spawn_on_schedule(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        assert_eq!(ids(&session.advance(&stage, Tick(30))), [1]);
        assert!(session.advance(&stage, Tick(59)).is_empty());
        assert_eq!(ids(&session.advance(&stage, Tick(60))), [2]);
        assert!(session.advance(&stage, Tick(119)).is_empty());
        assert_eq!(
            ids(&session.advance(&stage, Tick(120))),
            [3],
            "mid boss at the end of the wave"
        );
        assert!(
            session.advance(&stage, Tick(500)).is_empty(),
            "the boss spawns only once"
        );
        assert_eq!(session.alive_enemy_count(), 3);
    }

    #[rstest]
    fn defeating_regulars_does_not_clear_the_wave(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        session.advance(&stage, Tick(60));
        assert_eq!(
            session.enemy_fell(EnemyId(1)),
            [DomainEvent::EnemyDefeated { enemy: EnemyId(1) }]
        );
        session.enemy_fell(EnemyId(2));
        session.advance(&stage, Tick(100));
        assert_eq!(session.waves().wave(), 0);
    }

    #[rstest]
    fn defeating_the_mid_boss_clears_the_wave(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let boss = spawn_first_wave(&mut session, &stage);
        assert_eq!(
            session.enemy_fell(boss),
            [
                DomainEvent::EnemyDefeated { enemy: boss },
                DomainEvent::WaveCleared { wave: 1 }
            ]
        );
        assert_eq!(session.alive_enemy_count(), 2, "regular enemies stay");
        assert_eq!(session.status(), GameStatus::Playing);
    }

    /// 次のWaveは、クリア後に最初に進めたTickから始まり、出現時刻はそのTickからの相対。
    #[rstest]
    fn next_wave_starts_from_the_tick_after_clearing(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let boss = spawn_first_wave(&mut session, &stage);
        session.enemy_fell(boss);

        assert_eq!(
            ids(&session.advance(&stage, Tick(200))),
            [4],
            "wave 2 starts at tick 200"
        );
        assert_eq!(session.waves().wave(), 1);
        assert!(session.advance(&stage, Tick(229)).is_empty());
        assert_eq!(ids(&session.advance(&stage, Tick(230))), [5], "wave 2 boss at +30");
    }

    /// 最終Waveのボスを倒すと勝利する。
    #[rstest]
    fn defeating_the_final_boss_wins_the_game(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let mut now = 0;
        for wave in 1..=3 {
            now += 1_000;
            // Waveを始めるTickと、ボスが出るTick（+500 なら必ず過ぎている）まで進める
            let mut spawned = session.advance(&stage, Tick(now));
            spawned.extend(session.advance(&stage, Tick(now + 500)));
            let boss = spawned.last().expect("boss spawns last").enemy;
            assert_eq!(
                session.enemy_fell(boss).last(),
                Some(&DomainEvent::WaveCleared { wave })
            );
        }
        let final_boss = session
            .advance(&stage, Tick(now + 1_000))
            .last()
            .expect("final boss")
            .enemy;
        assert_eq!(session.waves().wave(), 3);

        assert_eq!(
            session.enemy_fell(final_boss),
            [DomainEvent::EnemyDefeated { enemy: final_boss }, DomainEvent::GameWon]
        );
        assert_eq!(session.status(), GameStatus::Won);
        assert_eq!(
            session.ensure_playing(),
            Err(GameSessionError::NotPlaying(GameStatus::Won))
        );
        assert!(session.advance(&stage, Tick(now + 10_000)).is_empty());
    }

    // ── 難易度とGoal ──

    /// Goal 2 は最難でだけ有効。第1Waveの通常敵が Goal 2 を目指す計画。
    fn stage_with_hard_goal(mut parts: StageParts) -> Stage {
        let hard_goal = Goal {
            id: GoalId(2),
            available_from: Difficulty::HARDEST,
            ..parts.goals[0].clone()
        };
        parts.goals.push(hard_goal);
        parts.wave_plan = waves_with_first(Wave::new(vec![spawn(0, 1, 2)], spawn(10, 1, 1)));
        Stage::new(parts).expect("valid")
    }

    #[rstest]
    #[case::easy_redirects_to_the_easy_goal(Difficulty::EASY, GoalId(1))]
    #[case::hardest_uses_the_planned_goal(Difficulty::HARDEST, GoalId(2))]
    fn enemies_head_only_for_goals_active_on_the_difficulty(
        parts: StageParts,
        #[case] difficulty: Difficulty,
        #[case] goal: GoalId,
    ) {
        let stage = stage_with_hard_goal(parts);
        let mut session = start(&stage, difficulty);
        assert_eq!(session.advance(&stage, Tick(0))[0].goal, goal);
    }

    // ── 敗北・退場 ──

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
        session.medals_fell(100.0, &catalog());
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        let lost = session.status();

        assert!(
            session
                .enemy_reached_goal(EnemyId(2), GoalId(1))
                .is_empty()
        );
        assert!(session.enemy_fell(EnemyId(2)).is_empty());
        assert!(session.medals_fell(100.0, &catalog()).is_empty());
        assert!(session.advance(&stage, Tick(600)).is_empty());
        assert_eq!(session.ensure_playing(), Err(GameSessionError::NotPlaying(lost)));
        assert_eq!(
            session.evolve(Evolution::LevelUp(MedalCode::normal()), &catalog()),
            Err(GameSessionError::NotPlaying(lost))
        );
    }

    #[rstest]
    fn enemy_leaves_only_once(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        assert_eq!(session.enemy_fell(enemy), [DomainEvent::EnemyDefeated { enemy }]);
        assert!(session.enemy_fell(enemy).is_empty());
        assert!(session.enemy_reached_goal(enemy, GoalId(1)).is_empty());
        assert_eq!(session.status(), GameStatus::Playing);
    }

    #[rstest]
    fn unknown_enemy_is_ignored(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        assert!(session.enemy_fell(EnemyId(99)).is_empty());
        assert!(
            session
                .enemy_reached_goal(EnemyId(99), GoalId(1))
                .is_empty()
        );
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

    // ── メダル進化 ──

    fn lv(n: u8) -> MedalLevel {
        MedalLevel::new(n).expect("valid level")
    }

    /// ゲージは10で満タン。Easy は1枚4ポイント、最難は1枚1ポイント（spec の fixture）。
    #[rstest]
    #[case::easy(Difficulty::EASY, 2.0, 3.0)]
    #[case::hardest(Difficulty::HARDEST, 9.0, 10.0)]
    fn falling_medals_fill_the_gauge_by_difficulty(
        stage: Stage,
        #[case] difficulty: Difficulty,
        #[case] not_yet: f32,
        #[case] enough: f32,
    ) {
        let mut session = start(&stage, difficulty);
        assert!(session.medals_fell(not_yet, &catalog()).is_empty());
        assert!(!session.gauge().is_full());
        assert_eq!(
            session.medals_fell(enough - not_yet, &catalog()),
            [DomainEvent::EvolutionReady]
        );
        assert!(session.gauge().is_full());
        assert!(session.medals_fell(5.0, &catalog()).is_empty(), "reported only once");
    }

    #[rstest]
    fn evolution_needs_a_full_gauge(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        let before = session.medals().clone();
        assert_eq!(
            session.evolve(Evolution::LevelUp(MedalCode::normal()), &catalog),
            Err(GameSessionError::Evolution(EvolutionError::NotReady))
        );
        assert_eq!(*session.medals(), before);
    }

    #[rstest]
    fn level_up_changes_the_medal_spec_and_empties_the_gauge(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        session.medals_fell(10.0, &catalog);
        assert_eq!(
            session.evolve(Evolution::LevelUp(MedalCode::normal()), &catalog),
            Ok(DomainEvent::Evolved { medal: MedalCode::normal(), level: lv(2) })
        );
        assert_eq!(
            session
                .medal_spec(&MedalCode::normal(), &catalog)
                .map(|s| s.mass.get()),
            Some(2.0)
        );
        assert!(!session.gauge().is_full());
    }

    #[rstest]
    fn rejected_evolution_keeps_the_gauge_full(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        session.medals_fell(10.0, &catalog);
        assert_eq!(
            session.evolve(Evolution::LevelUp(fire()), &catalog),
            Err(GameSessionError::Evolution(EvolutionError::NotOwned(fire())))
        );
        assert!(session.gauge().is_full(), "the player can choose again");
    }

    #[rstest]
    fn fusion_uses_the_recipes_of_the_catalog(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        // Normal を Lv.4 にする（3回）
        for _ in 0..3 {
            session.medals_fell(10.0, &catalog);
            session
                .evolve(Evolution::LevelUp(MedalCode::normal()), &catalog)
                .expect("level up");
        }
        session.medals_fell(10.0, &catalog);
        assert_eq!(
            session.evolve(Evolution::Fuse(MedalCode::normal(), fire()), &catalog),
            Err(GameSessionError::Evolution(EvolutionError::NotOwned(fire()))),
            "Fire is not owned yet"
        );
        assert_eq!(session.medals().level(&MedalCode::normal()), Some(lv(4)));
        assert_eq!(session.medal_spec(&sauna(), &catalog), None, "not owned");
    }

    // ── New! ──

    fn fill(session: &mut GameSession, catalog: &SpecCatalog) -> Option<MedalCode> {
        session.medals_fell(10.0, catalog);
        session.evolution_offer().cloned()
    }

    #[rstest]
    fn new_offer_is_an_unowned_element_fixed_when_the_gauge_fills(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        assert_eq!(session.evolution_offer(), None, "nothing before the gauge fills");
        let offer = fill(&mut session, &catalog).expect("an element is offered");
        assert!([fire(), water()].contains(&offer));
        session.medals_fell(10.0, &catalog);
        assert_eq!(
            session.evolution_offer(),
            Some(&offer),
            "fixed while the screen is open"
        );
    }

    #[rstest]
    fn same_seed_offers_the_same_medal_and_seeds_cover_every_element(
        stage: Stage,
        catalog: SpecCatalog,
    ) {
        let rules = *catalog.evolution();
        let offer = |seed| {
            let (mut session, _) = GameSession::start(
                GameSessionId(1),
                &stage,
                &character(),
                Difficulty::EASY,
                &rules,
                seed,
            );
            fill(&mut session, &catalog).expect("offered")
        };
        assert_eq!(offer(5), offer(5));
        let seen: HashSet<_> = (0..64).map(offer).collect();
        assert_eq!(seen, HashSet::from([fire(), water()]));
    }

    #[rstest]
    fn acquiring_the_offer_adds_it_and_clears_the_offer(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        let offer = fill(&mut session, &catalog).expect("offered");
        assert_eq!(
            session.evolve(Evolution::Acquire(offer.clone()), &catalog),
            Ok(DomainEvent::Evolved { medal: offer.clone(), level: MedalLevel::MIN })
        );
        assert_eq!(session.medals().level(&offer), Some(MedalLevel::MIN));
        assert_eq!(session.evolution_offer(), None);
        assert!(!session.gauge().is_full());
    }

    #[rstest]
    fn level_up_also_clears_the_offer(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        fill(&mut session, &catalog);
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &catalog)
            .expect("level up");
        assert_eq!(session.evolution_offer(), None);
    }

    #[rstest]
    fn acquiring_a_medal_not_offered_is_rejected(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        let offer = fill(&mut session, &catalog).expect("offered");
        let other = if offer == fire() {
            water()
        } else {
            fire()
        };
        let before = session.medals().clone();
        assert_eq!(
            session.evolve(Evolution::Acquire(other.clone()), &catalog),
            Err(GameSessionError::Evolution(EvolutionError::NotOffered(other)))
        );
        assert_eq!(*session.medals(), before);
        assert_eq!(session.evolution_offer(), Some(&offer), "the player can choose again");
    }

    #[rstest]
    fn offers_skip_owned_elements_and_stop_when_all_are_owned(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        let first = fill(&mut session, &catalog).expect("offered");
        session
            .evolve(Evolution::Acquire(first.clone()), &catalog)
            .expect("acquire");
        let second = fill(&mut session, &catalog).expect("the other element");
        assert_ne!(first, second);
        session
            .evolve(Evolution::Acquire(second), &catalog)
            .expect("acquire");
        assert_eq!(fill(&mut session, &catalog), None, "every element is owned");
        assert!(session.gauge().is_full(), "level up and fusion are still possible");
    }

    #[rstest]
    fn unowned_medal_has_no_spec(started: (GameSession, Stage), catalog: SpecCatalog) {
        assert_eq!(started.0.medal_spec(&fire(), &catalog), None);
        assert_eq!(
            started
                .0
                .medal_spec(&MedalCode::normal(), &catalog)
                .map(|s| s.mass.get()),
            Some(1.0)
        );
    }

    #[rstest]
    fn exhausted_evolutions_do_not_open_an_unusable_offer(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        // すべて取得・Lv.4化し、錬成後の素材も再取得する。
        for _ in 0..40 {
            fill(&mut session, &catalog);
            let next = session
                .medals()
                .owned()
                .find(|(_, l)| *l < MedalLevel::MAX)
                .map(|(m, _)| Evolution::LevelUp(m.clone()))
                .or_else(|| {
                    session
                        .evolution_offer()
                        .cloned()
                        .map(Evolution::Acquire)
                })
                .or_else(|| {
                    (session.medals().level(&sauna()).is_none())
                        .then(|| Evolution::Fuse(fire(), water()))
                });
            let Some(next) = next else {
                break;
            };
            session
                .evolve(next, &catalog)
                .expect("valid evolution");
        }
        assert_eq!(session.medals().owned().count(), 4);
        assert!(
            session
                .medals()
                .owned()
                .all(|(_, l)| l == MedalLevel::MAX)
        );
        assert!(!session.gauge().is_full(), "there must be no empty evolution screen");
        assert!(session.medals_fell(100.0, &catalog).is_empty());
    }

    // ── 能力の発動と再生 ──

    /// fire: 接触で爆風（Lv.2 は倍率2）。water: 直前の能力を再生する。
    fn ability_catalog() -> SpecCatalog {
        use crate::{
            ability::{AbilityDef, Area, CollideWith, Direction, Target, Trigger},
            spec::abilities::MedalAbilities,
        };
        let blast = AbilityDef {
            on: Trigger::Collide(CollideWith::Any),
            target: Target::Both,
            area: Area::Radius(p(1.0)),
            effects: vec![Effect::Push { dir: Direction::Outward, strength: p(2.0) }],
            uses: None,
        };
        let replay = AbilityDef { effects: vec![Effect::Replay], ..blast.clone() };
        let mut parts = spec_parts();
        parts.abilities.scale = [p(1.0), p(2.0), p(1.0), p(1.0)];
        parts.abilities.medals = std::collections::BTreeMap::from([
            (fire(), MedalAbilities { base: vec![blast], awaken: vec![] }),
            (water(), MedalAbilities { base: vec![replay], awaken: vec![] }),
        ]);
        SpecCatalog::new(parts).expect("valid")
    }

    fn fired(medal: MedalCode, level: u8) -> AbilityRef {
        AbilityRef { medal, level: lv(level), index: 0 }
    }

    fn strength(ability: &crate::ability::AbilityDef) -> f32 {
        match ability.effects[0] {
            Effect::Push { strength, .. } => strength.get(),
            _ => panic!("not a push"),
        }
    }

    #[rstest]
    fn an_ability_fires_without_replaying(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        let specs = ability_catalog();
        let out = session
            .fire_ability(&fired(fire(), 1), &specs)
            .expect("fired");
        assert_eq!(strength(out.ability), 2.0);
        assert_eq!(out.replayed, None);
    }

    #[rstest]
    fn replay_fires_the_last_ability_at_its_level(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        let specs = ability_catalog();
        session.fire_ability(&fired(fire(), 2), &specs);
        let out = session
            .fire_ability(&fired(water(), 1), &specs)
            .expect("fired");
        assert_eq!(out.ability.effects, [Effect::Replay]);
        assert_eq!(out.replayed.map(strength), Some(4.0), "the level 2 blast");
    }

    #[rstest]
    fn replay_with_nothing_before_replays_nothing(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        let specs = ability_catalog();
        let out = session
            .fire_ability(&fired(water(), 1), &specs)
            .expect("fired");
        assert_eq!(out.replayed, None);
    }

    /// 再生を記録すると、再生が再生を呼び続ける。
    #[rstest]
    fn a_replay_is_not_recorded(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        let specs = ability_catalog();
        session.fire_ability(&fired(fire(), 1), &specs);
        session.fire_ability(&fired(water(), 1), &specs);
        let again = session
            .fire_ability(&fired(water(), 1), &specs)
            .expect("fired");
        assert_eq!(again.replayed.map(strength), Some(2.0), "still the blast");
    }

    #[rstest]
    fn an_undefined_ability_fires_nothing_and_is_not_recorded(started: (GameSession, Stage)) {
        let (mut session, _) = started;
        let specs = ability_catalog();
        session.fire_ability(&fired(fire(), 1), &specs);
        let unknown = AbilityRef { index: 5, ..fired(fire(), 1) };
        assert_eq!(session.fire_ability(&unknown, &specs), None);
        assert_eq!(session.fire_ability(&fired(sauna(), 1), &specs), None);
        let out = session
            .fire_ability(&fired(water(), 1), &specs)
            .expect("fired");
        assert_eq!(out.replayed.map(strength), Some(2.0));
    }

    #[rstest]
    fn no_ability_fires_while_an_evolution_is_pending_or_after_the_game(
        started: (GameSession, Stage),
    ) {
        let (mut session, stage) = started;
        let specs = ability_catalog();
        session.medals_fell(10.0, &specs);
        assert_eq!(session.fire_ability(&fired(fire(), 1), &specs), None);
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &specs)
            .expect("level up");
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        assert_eq!(session.fire_ability(&fired(fire(), 1), &specs), None);
        assert_eq!(session.last_ability, None, "nothing was recorded");
    }

    /// Easy は1枚4ポイント、10で満タン（spec の fixture）。
    #[rstest]
    #[case::shiny_medals(2.5, true)]
    #[case::rounded_up(2.4, true)]
    #[case::rounded_down(2.3, false)]
    #[case::fakes_are_worth_nothing(0.0, false)]
    #[case::negative(-5.0, false)]
    #[case::nan(f32::NAN, false)]
    #[case::infinite(f32::INFINITY, false)]
    fn falling_value_fills_the_gauge(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
        #[case] value: f32,
        #[case] full: bool,
    ) {
        let (mut session, _) = started;
        session.medals_fell(value, &catalog);
        assert_eq!(session.gauge().is_full(), full);
    }

    #[rstest]
    fn huge_value_saturates_instead_of_overflowing(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        assert_eq!(session.medals_fell(f32::MAX, &catalog), [DomainEvent::EvolutionReady]);
    }

    #[rstest]
    fn abilities_add_points_like_falls(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        assert!(session.add_gauge(9, &catalog).is_empty());
        assert_eq!(session.add_gauge(1, &catalog), [DomainEvent::EvolutionReady]);
        assert!(session.evolution_offer().is_some(), "New! is chosen as with falls");
        assert!(session.add_gauge(5, &catalog).is_empty(), "reported once");
    }

    #[rstest]
    fn no_points_after_the_game(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, stage) = started;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        assert!(session.add_gauge(100, &catalog).is_empty());
        assert!(!session.gauge().is_full());
    }

    #[rstest]
    fn random_elements_follow_the_seed_and_cover_every_element(stage: Stage, catalog: SpecCatalog) {
        let rules = *catalog.evolution();
        let picks = |seed| {
            let (mut session, _) = GameSession::start(
                GameSessionId(1),
                &stage,
                &character(),
                Difficulty::EASY,
                &rules,
                seed,
            );
            (0..32)
                .map(|_| {
                    session
                        .random_element(&catalog)
                        .expect("elements exist")
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(picks(3), picks(3));
        assert_eq!(
            picks(3).into_iter().collect::<HashSet<_>>(),
            HashSet::from([fire(), water()])
        );
    }

    // ── 進行できる状態 ──

    #[rstest]
    fn running_until_the_gauge_fills(started: (GameSession, Stage), catalog: SpecCatalog) {
        let (mut session, _) = started;
        assert_eq!(session.ensure_running(), Ok(()));
        session.medals_fell(10.0, &catalog);
        assert_eq!(session.ensure_running(), Err(GameSessionError::EvolutionPending));
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &catalog)
            .expect("level up");
        assert_eq!(session.ensure_running(), Ok(()));
    }

    #[rstest]
    fn not_running_after_the_game_ends(started: (GameSession, Stage)) {
        let (mut session, stage) = started;
        let enemy = session.advance(&stage, Tick(0))[0].enemy;
        session.enemy_reached_goal(enemy, GoalId(1));
        assert_eq!(
            session.ensure_running(),
            Err(GameSessionError::NotPlaying(session.status()))
        );
    }

    /// 進化の選択中は敵が出ない。選んだ後に、溜まっていた出現をまとめて出す。
    #[rstest]
    fn no_spawns_while_an_evolution_is_pending(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, stage) = started;
        session.medals_fell(10.0, &catalog);
        assert!(session.advance(&stage, Tick(60)).is_empty());
        session
            .evolve(Evolution::LevelUp(MedalCode::normal()), &catalog)
            .expect("level up");
        assert_eq!(ids(&session.advance(&stage, Tick(60))), [1, 2]);
    }

    // ── 進化画面の選択肢 ──

    fn choices(session: &GameSession, catalog: &SpecCatalog) -> Vec<(Evolution, MedalCode, bool)> {
        session
            .evolution_choices(catalog)
            .map(|c| (c.evolution, c.result, c.available))
            .collect()
    }

    #[rstest]
    fn choices_list_level_ups_and_the_new_medal(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        let offer = fill(&mut session, &catalog).expect("offered");
        assert_eq!(
            choices(&session, &catalog),
            [
                (Evolution::LevelUp(MedalCode::normal()), MedalCode::normal(), true),
                (Evolution::Acquire(offer.clone()), offer, true),
            ]
        );
    }

    #[rstest]
    fn fusion_below_max_level_is_shown_but_unavailable(
        started: (GameSession, Stage),
        catalog: SpecCatalog,
    ) {
        let (mut session, _) = started;
        for _ in 0..2 {
            let offer = fill(&mut session, &catalog).expect("offered");
            session
                .evolve(Evolution::Acquire(offer), &catalog)
                .expect("acquire");
        }
        fill(&mut session, &catalog);
        let all = choices(&session, &catalog);
        assert!(all.contains(&(Evolution::Fuse(fire(), water()), sauna(), false)));
        assert_eq!(
            all.iter().filter(|(.., available)| *available).count(),
            3,
            "level up of normal, fire and water"
        );
    }

    /// 錬成・New! を含め、試し得るすべての進化。錬成は順序を問わないため、素材の組は a ≦ b
    /// の正規順だけを並べる（逆順は同じ選択肢）。
    fn every_evolution() -> Vec<Evolution> {
        let medals = [MedalCode::normal(), fire(), water(), sauna()];
        let mut all: Vec<_> = medals
            .iter()
            .flat_map(|m| [Evolution::LevelUp(m.clone()), Evolution::Acquire(m.clone())])
            .collect();
        for a in &medals {
            for b in medals.iter().filter(|b| a <= *b) {
                all.push(Evolution::Fuse(a.clone(), b.clone()));
            }
        }
        all
    }

    #[derive(Debug, Clone)]
    enum Op {
        Fell(u64),
        Goal(u64),
    }

    fn evolution() -> impl Strategy<Value = Evolution> {
        let medal =
            prop_oneof![Just(MedalCode::normal()), Just(fire()), Just(water()), Just(sauna())];
        prop_oneof![
            medal.clone().prop_map(Evolution::LevelUp),
            medal.clone().prop_map(Evolution::Acquire),
            (medal.clone(), medal).prop_map(|(a, b)| Evolution::Fuse(a, b)),
        ]
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
                    Op::Fell(id) => session.enemy_fell(EnemyId(id)),
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

        /// どんな進化を試しても、失敗した進化は所持メダルもゲージも変えない（全か無か）。
        #[test]
        fn evolutions_are_all_or_nothing(evolutions in prop::collection::vec(evolution(), 0..30)) {
            let catalog = SpecCatalog::new(spec_parts()).expect("valid");
            let (mut session, _) = started(stage(parts()));
            for evolution in evolutions {
                session.medals_fell(10.0, &catalog);
                let before = (session.medals().clone(), *session.gauge());
                if session.evolve(evolution, &catalog).is_err() {
                    prop_assert_eq!((session.medals().clone(), *session.gauge()), before);
                } else {
                    prop_assert!(!session.gauge().is_full());
                }
            }
        }

        /// 選べると示した進化は必ず成功し、成功する進化は必ず選べると示す（判定の二重化をしない）。
        #[test]
        fn choices_match_what_evolve_accepts(evolutions in prop::collection::vec(evolution(), 0..40)) {
            let catalog = SpecCatalog::new(spec_parts()).expect("valid");
            let (mut session, _) = started(stage(parts()));
            for evolution in evolutions {
                session.medals_fell(10.0, &catalog);
                let available: Vec<_> = session
                    .evolution_choices(&catalog)
                    .filter(|c| c.available)
                    .map(|c| c.evolution)
                    .collect();
                for candidate in every_evolution() {
                    let accepted = session.clone().evolve(candidate.clone(), &catalog).is_ok();
                    prop_assert_eq!(accepted, available.contains(&candidate), "{:?}", candidate);
                }
                let _ = session.evolve(evolution, &catalog);
            }
        }
    }
}
