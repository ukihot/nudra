//! Stage集約（基本設計 3.3）。

mod catapult;
mod error;
mod placement;
mod rail;
mod wave;

use std::collections::HashSet;

#[cfg(test)]
pub(crate) use catapult::fixture as catapult_fixture;
pub use catapult::{Catapult, CatapultParts};
pub use error::StageError;
pub use placement::InitialMedalPlacement;
pub use rail::Rail;
pub use wave::{SpawnEntry, Wave, WavePlan};

use crate::{
    difficulty::Difficulty,
    id::{BridgeId, GoalId, RailId, StageId},
    math::Vec3,
    positive::Positive,
};

/// ステージの形状。寸法はコライダーにもなるため `Positive`。
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rectangle { width: Positive, depth: Positive },
    Circle { radius: Positive },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub shape: Shape,
    pub floor_y: f32,
}

impl Geometry {
    /// 床面上（XZ平面）で形状の内側にあるか。
    pub fn contains_xz(&self, p: Vec3) -> bool {
        match self.shape {
            Shape::Rectangle { width, depth } => {
                p.x.abs() <= width.get() / 2.0 && p.z.abs() <= depth.get() / 2.0
            },
            Shape::Circle { radius } => p.x * p.x + p.z * p.z <= radius.get() * radius.get(),
        }
    }
}

/// 敵の侵入路。`entry`は外部の出現点、`exit`はステージ側の端点。
#[derive(Debug, Clone, PartialEq)]
pub struct Bridge {
    pub id: BridgeId,
    pub entry: Vec3,
    pub exit: Vec3,
    pub width: Positive,
}

/// 落下領域（AABB）。侵入判定は物理エンジンのセンサーが行う。
/// センサーのコライダーになるため、全軸で `min < max` を保証する。
#[derive(Debug, Clone, PartialEq)]
pub struct Abyss {
    min: Vec3,
    max: Vec3,
}

impl Abyss {
    pub fn new(min: Vec3, max: Vec3) -> Result<Self, StageError> {
        // NaN・無限大も `min < max` かつ有限の判定で弾く
        let size = max - min;
        if size.is_finite() && size.cmpgt(Vec3::ZERO).all() {
            Ok(Self { min, max })
        } else {
            Err(StageError::InvalidAbyss { min, max })
        }
    }

    pub fn min(&self) -> Vec3 {
        self.min
    }

    pub fn max(&self) -> Vec3 {
        self.max
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) / 2.0
    }

    /// 各軸の長さ（すべて正）。
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub id: GoalId,
    pub position: Vec3,
    pub radius: Positive,
    /// この難易度以上で有効になる（Easy で有効なGoalはちょうど1つ、基本設計 3.12）
    pub available_from: Difficulty,
}

/// `Stage::new`に渡す組み立て用の値。
#[derive(Debug, Clone)]
pub struct StageParts {
    pub id: StageId,
    pub geometry: Geometry,
    pub catapult: Catapult,
    pub bridges: Vec<Bridge>,
    pub abysses: Vec<Abyss>,
    pub rails: Vec<Rail>,
    pub goals: Vec<Goal>,
    pub initial_medals: Vec<InitialMedalPlacement>,
    pub wave_plan: WavePlan,
}

/// 生成後は不変。インゲーム中の敵・メダルの状態は持たない。
#[derive(Debug, Clone)]
pub struct Stage(StageParts);

impl Stage {
    /// 唯一の生成経路。不変条件 S1〜S8 を満たすStageしか存在できない。
    pub fn new(parts: StageParts) -> Result<Self, StageError> {
        if parts.goals.is_empty() {
            return Err(StageError::NoGoal);
        }
        let easy_goals = parts
            .goals
            .iter()
            .filter(|g| g.available_from == Difficulty::EASY)
            .count();
        if easy_goals != 1 {
            return Err(StageError::EasyGoalCount(easy_goals));
        }
        if parts.bridges.is_empty() {
            return Err(StageError::NoBridge);
        }
        if parts.rails.is_empty() {
            return Err(StageError::NoRail);
        }
        if parts
            .initial_medals
            .iter()
            .map(InitialMedalPlacement::count)
            .sum::<usize>()
            == 0
        {
            return Err(StageError::NoInitialMedals);
        }
        ensure_unique("goal", parts.goals.iter().map(|g| g.id.0))?;
        ensure_unique("bridge", parts.bridges.iter().map(|b| b.id.0))?;
        ensure_unique("rail", parts.rails.iter().map(|r| r.id().0))?;
        // S6（Railの形状）は Rail::new で保証済み。S8 は Catapult::new で保証済み。
        let waves = parts.wave_plan.waves();
        if waves.len() != WavePlan::WAVES {
            return Err(StageError::WaveCount(waves.len()));
        }
        if waves
            .last()
            .is_some_and(|last| !last.spawns().is_empty())
        {
            return Err(StageError::FinalWaveHasRegulars);
        }
        for spawn in waves.iter().flat_map(Wave::entries) {
            if !parts.bridges.iter().any(|b| b.id == spawn.bridge) {
                return Err(StageError::UnknownReference { kind: "bridge", id: spawn.bridge.0 });
            }
            if !parts.goals.iter().any(|g| g.id == spawn.goal) {
                return Err(StageError::UnknownReference { kind: "goal", id: spawn.goal.0 });
            }
        }
        Ok(Self(parts))
    }

    pub fn id(&self) -> &StageId {
        &self.0.id
    }
    pub fn geometry(&self) -> &Geometry {
        &self.0.geometry
    }
    pub fn catapult(&self) -> &Catapult {
        &self.0.catapult
    }
    pub fn bridges(&self) -> &[Bridge] {
        &self.0.bridges
    }
    pub fn abysses(&self) -> &[Abyss] {
        &self.0.abysses
    }
    pub fn rails(&self) -> &[Rail] {
        &self.0.rails
    }
    pub fn goals(&self) -> &[Goal] {
        &self.0.goals
    }
    pub fn initial_medals(&self) -> &[InitialMedalPlacement] {
        &self.0.initial_medals
    }
    pub fn wave_plan(&self) -> &WavePlan {
        &self.0.wave_plan
    }
    pub fn bridge(&self, id: BridgeId) -> Option<&Bridge> {
        self.0.bridges.iter().find(|b| b.id == id)
    }
    /// 選んだ難易度で有効なGoal。
    pub fn active_goals(&self, difficulty: Difficulty) -> impl Iterator<Item = &Goal> {
        self.0
            .goals
            .iter()
            .filter(move |g| g.available_from <= difficulty)
    }

    pub fn goal(&self, id: GoalId) -> Option<&Goal> {
        self.0.goals.iter().find(|g| g.id == id)
    }
    pub fn rail(&self, id: RailId) -> Option<&Rail> {
        self.0.rails.iter().find(|r| r.id() == id)
    }
}

fn ensure_unique(kind: &'static str, ids: impl IntoIterator<Item = u16>) -> Result<(), StageError> {
    let mut seen = HashSet::new();
    match ids.into_iter().find(|id| !seen.insert(*id)) {
        Some(id) => Err(StageError::DuplicateId { kind, id }),
        None => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use rstest::fixture;

    use super::*;
    use crate::{id::EnemyTypeId, positive::fixture::p, time::Ticks};

    #[fixture]
    pub fn parts() -> StageParts {
        StageParts {
            id: StageId::new("test"),
            geometry: Geometry {
                shape: Shape::Rectangle { width: p(10.0), depth: p(10.0) },
                floor_y: 0.0,
            },
            catapult: Catapult::new(super::catapult::fixture::catapult_parts())
                .expect("valid catapult"),
            bridges: vec![Bridge {
                id: BridgeId(1),
                entry: Vec3::new(10.0, 0.0, 0.0),
                exit: Vec3::new(5.0, 0.0, 0.0),
                width: p(2.0),
            }],
            abysses: vec![
                Abyss::new(Vec3::splat(-50.0), Vec3::new(50.0, -2.0, 50.0)).expect("valid abyss"),
            ],
            rails: vec![
                Rail::new(RailId(1), vec![Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)])
                    .expect("valid rail"),
            ],
            goals: vec![Goal {
                id: GoalId(1),
                position: Vec3::new(0.0, 0.0, -4.0),
                radius: p(1.0),
                available_from: Difficulty::EASY,
            }],
            initial_medals: vec![InitialMedalPlacement::grid(Vec3::ZERO, 2, 2, 1, Vec3::ONE)],
            wave_plan: wave_plan(),
        }
    }

    #[fixture]
    pub fn stage(parts: StageParts) -> Stage {
        Stage::new(parts).expect("valid stage")
    }

    /// 4Wave: 第1Waveは通常2体（Tick 0,
    /// 60）と中ボス（120）、第2・3Waveは通常1体と中ボス（30）、第4Waveは最終ボス。
    pub fn wave_plan() -> WavePlan {
        WavePlan::new(vec![
            Wave::new(vec![spawn(0, 1, 1), spawn(60, 1, 1)], spawn(120, 1, 1)),
            Wave::new(vec![spawn(0, 1, 1)], spawn(30, 1, 1)),
            Wave::new(vec![spawn(0, 1, 1)], spawn(30, 1, 1)),
            Wave::boss_only(spawn(0, 1, 1)),
        ])
    }

    /// 第1Waveだけを差し替えた計画。
    pub fn waves_with_first(first: Wave) -> WavePlan {
        let mut waves = wave_plan().waves().to_vec();
        waves[0] = first;
        WavePlan::new(waves)
    }

    pub fn spawn(at_tick: u64, bridge: u16, goal: u16) -> SpawnEntry {
        SpawnEntry {
            at: Ticks(at_tick),
            enemy: EnemyTypeId::new("grunt"),
            bridge: BridgeId(bridge),
            goal: GoalId(goal),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{
        fixture::{parts, spawn, wave_plan, waves_with_first},
        *,
    };
    use crate::positive::fixture::p;

    #[rstest]
    fn valid_stage_is_created(parts: StageParts) {
        let stage = Stage::new(parts).expect("valid");
        assert_eq!(stage.id().as_str(), "test");
        assert!(stage.goal(GoalId(1)).is_some());
        assert!(stage.bridge(BridgeId(9)).is_none());
    }

    #[rstest]
    #[case::no_goal(|p: &mut StageParts| p.goals.clear(), StageError::NoGoal)]
    #[case::no_easy_goal(
        |p: &mut StageParts| p.goals[0].available_from = Difficulty::HARDEST,
        StageError::EasyGoalCount(0)
    )]
    #[case::two_easy_goals(
        |p: &mut StageParts| p.goals.push(Goal { id: GoalId(2), ..p.goals[0].clone() }),
        StageError::EasyGoalCount(2)
    )]
    #[case::no_bridge(|p: &mut StageParts| p.bridges.clear(), StageError::NoBridge)]
    #[case::no_rail(|p: &mut StageParts| p.rails.clear(), StageError::NoRail)]
    #[case::no_medals(|p: &mut StageParts| p.initial_medals.clear(), StageError::NoInitialMedals)]
    #[case::empty_grid(
        |p: &mut StageParts| p.initial_medals = vec![InitialMedalPlacement::grid(Vec3::ZERO, 0, 3, 1, Vec3::ONE)],
        StageError::NoInitialMedals
    )]
    #[case::duplicate_goal(
        |p: &mut StageParts| p.goals.push(Goal { available_from: Difficulty::HARDEST, ..p.goals[0].clone() }),
        StageError::DuplicateId { kind: "goal", id: 1 }
    )]
    #[case::duplicate_bridge(|p: &mut StageParts| p.bridges.push(p.bridges[0].clone()), StageError::DuplicateId { kind: "bridge", id: 1 })]
    #[case::duplicate_rail(|p: &mut StageParts| p.rails.push(p.rails[0].clone()), StageError::DuplicateId { kind: "rail", id: 1 })]
    #[case::unknown_bridge(
        |p: &mut StageParts| p.wave_plan = waves_with_first(Wave::new(vec![spawn(0, 9, 1)], spawn(10, 1, 1))),
        StageError::UnknownReference { kind: "bridge", id: 9 }
    )]
    #[case::unknown_goal(
        |p: &mut StageParts| p.wave_plan = waves_with_first(Wave::new(vec![spawn(0, 1, 7)], spawn(10, 1, 1))),
        StageError::UnknownReference { kind: "goal", id: 7 }
    )]
    #[case::unknown_boss_bridge(
        |p: &mut StageParts| p.wave_plan = waves_with_first(Wave::new(vec![], spawn(10, 9, 1))),
        StageError::UnknownReference { kind: "bridge", id: 9 }
    )]
    #[case::three_waves(
        |p: &mut StageParts| p.wave_plan = WavePlan::new(wave_plan().waves()[..3].to_vec()),
        StageError::WaveCount(3)
    )]
    #[case::five_waves(
        |p: &mut StageParts| p.wave_plan = WavePlan::new([wave_plan().waves(), &wave_plan().waves()[..1]].concat()),
        StageError::WaveCount(5)
    )]
    #[case::final_wave_with_regulars(
        |p: &mut StageParts| {
            let mut waves = wave_plan().waves().to_vec();
            waves[3] = Wave::new(vec![spawn(0, 1, 1)], spawn(0, 1, 1));
            p.wave_plan = WavePlan::new(waves);
        },
        StageError::FinalWaveHasRegulars
    )]
    fn invalid_stage_is_rejected(
        mut parts: StageParts,
        #[case] break_it: fn(&mut StageParts),
        #[case] expected: StageError,
    ) {
        break_it(&mut parts);
        assert_eq!(Stage::new(parts).err(), Some(expected));
    }

    #[rstest]
    #[case::inside_rect(Shape::Rectangle { width: p(4.0), depth: p(2.0) }, Vec3::new(2.0, 9.0, -1.0), true)]
    #[case::outside_rect(Shape::Rectangle { width: p(4.0), depth: p(2.0) }, Vec3::new(2.1, 0.0, 0.0), false)]
    #[case::inside_circle(Shape::Circle { radius: p(1.0) }, Vec3::new(0.6, 0.0, 0.6), true)]
    #[case::outside_circle(Shape::Circle { radius: p(1.0) }, Vec3::new(0.8, 0.0, 0.8), false)]
    fn geometry_contains_xz(#[case] shape: Shape, #[case] p: Vec3, #[case] expected: bool) {
        assert_eq!(Geometry { shape, floor_y: 0.0 }.contains_xz(p), expected);
    }

    #[rstest]
    fn abyss_exposes_center_and_size() {
        let abyss =
            Abyss::new(Vec3::new(-2.0, -10.0, 0.0), Vec3::new(2.0, -2.0, 6.0)).expect("valid");
        assert_eq!(abyss.center(), Vec3::new(0.0, -6.0, 3.0));
        assert_eq!(abyss.size(), Vec3::new(4.0, 8.0, 6.0));
        assert_eq!(
            (abyss.min(), abyss.max()),
            (Vec3::new(-2.0, -10.0, 0.0), Vec3::new(2.0, -2.0, 6.0))
        );
    }

    #[rstest]
    #[case::flat_x(Vec3::ZERO, Vec3::new(0.0, 1.0, 1.0))]
    #[case::flat_y(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0))]
    #[case::flat_z(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0))]
    #[case::inverted(Vec3::ONE, Vec3::ZERO)]
    #[case::nan(Vec3::ZERO, Vec3::new(f32::NAN, 1.0, 1.0))]
    #[case::infinite(Vec3::ZERO, Vec3::new(f32::INFINITY, 1.0, 1.0))]
    fn degenerate_abyss_is_rejected(#[case] min: Vec3, #[case] max: Vec3) {
        assert!(matches!(Abyss::new(min, max), Err(StageError::InvalidAbyss { .. })));
    }

    #[rstest]
    fn harder_difficulties_add_goals(mut parts: StageParts) {
        let level = |n| Difficulty::new(n).expect("valid");
        parts.goals.push(Goal {
            id: GoalId(2),
            available_from: level(3),
            ..parts.goals[0].clone()
        });
        parts.goals.push(Goal {
            id: GoalId(3),
            available_from: level(4),
            ..parts.goals[0].clone()
        });
        let stage = Stage::new(parts).expect("valid");
        let ids = |d| {
            stage
                .active_goals(d)
                .map(|g| g.id.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Difficulty::EASY), [1]);
        assert_eq!(ids(level(2)), [1]);
        assert_eq!(ids(level(3)), [1, 2]);
        assert_eq!(ids(Difficulty::HARDEST), [1, 2, 3]);
    }
}
