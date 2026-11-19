//! Stage集約（基本設計 3.3）。

mod catapult;
mod error;
mod placement;
mod rail;
mod wave;

use std::collections::HashSet;

pub use catapult::Catapult;
pub use error::StageError;
pub use placement::InitialMedalPlacement;
pub use rail::Rail;
pub use wave::{SpawnEntry, Wave, WavePlan};

use crate::{
    id::{BridgeId, GoalId, RailId, StageId},
    math::Vec3,
};

/// ステージの形状。衝突形状そのものはアセット側が持ち、ここではルールに必要な寸法だけを持つ。
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rectangle { width: f32, depth: f32 },
    Circle { radius: f32 },
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
                p.x.abs() <= width / 2.0 && p.z.abs() <= depth / 2.0
            },
            Shape::Circle { radius } => p.x * p.x + p.z * p.z <= radius * radius,
        }
    }
}

/// 敵の侵入路。`entry`は外部の出現点、`exit`はステージ側の端点。
#[derive(Debug, Clone, PartialEq)]
pub struct Bridge {
    pub id: BridgeId,
    pub entry: Vec3,
    pub exit: Vec3,
    pub width: f32,
}

/// 落下領域（AABB）。侵入判定は物理エンジンのセンサーが行う。
#[derive(Debug, Clone, PartialEq)]
pub struct Abyss {
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub id: GoalId,
    pub position: Vec3,
    pub radius: f32,
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
        for spawn in parts.wave_plan.waves().iter().flat_map(Wave::spawns) {
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
    use std::time::Duration;

    use rstest::fixture;

    use super::*;
    use crate::id::EnemyTypeId;

    #[fixture]
    pub fn parts() -> StageParts {
        StageParts {
            id: StageId::new("test"),
            geometry: Geometry {
                shape: Shape::Rectangle { width: 10.0, depth: 10.0 },
                floor_y: 0.0,
            },
            catapult: Catapult::new(
                Vec3::new(0.0, 1.0, 5.0),
                0.0,
                (-0.5, 0.5),
                0.2,
                10.0,
                Duration::from_millis(200),
            )
            .expect("valid catapult"),
            bridges: vec![Bridge {
                id: BridgeId(1),
                entry: Vec3::new(10.0, 0.0, 0.0),
                exit: Vec3::new(5.0, 0.0, 0.0),
                width: 2.0,
            }],
            abysses: vec![Abyss {
                min: Vec3::splat(-50.0),
                max: Vec3::new(50.0, -2.0, 50.0),
            }],
            rails: vec![
                Rail::new(RailId(1), vec![Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)])
                    .expect("valid rail"),
            ],
            goals: vec![Goal {
                id: GoalId(1),
                position: Vec3::new(0.0, 0.0, -4.0),
                radius: 1.0,
            }],
            initial_medals: vec![InitialMedalPlacement::grid(Vec3::ZERO, 2, 2, 1, Vec3::ONE)],
            wave_plan: WavePlan::new(vec![Wave::new(vec![spawn(0, 1, 1), spawn(1, 1, 1)])]),
        }
    }

    #[fixture]
    pub fn stage(parts: StageParts) -> Stage {
        Stage::new(parts).expect("valid stage")
    }

    pub fn spawn(at_secs: u64, bridge: u16, goal: u16) -> SpawnEntry {
        SpawnEntry {
            at: Duration::from_secs(at_secs),
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
        fixture::{parts, spawn},
        *,
    };

    #[rstest]
    fn valid_stage_is_created(parts: StageParts) {
        let stage = Stage::new(parts).expect("valid");
        assert_eq!(stage.id().as_str(), "test");
        assert!(stage.goal(GoalId(1)).is_some());
        assert!(stage.bridge(BridgeId(9)).is_none());
    }

    #[rstest]
    #[case::no_goal(|p: &mut StageParts| p.goals.clear(), StageError::NoGoal)]
    #[case::no_bridge(|p: &mut StageParts| p.bridges.clear(), StageError::NoBridge)]
    #[case::no_rail(|p: &mut StageParts| p.rails.clear(), StageError::NoRail)]
    #[case::no_medals(|p: &mut StageParts| p.initial_medals.clear(), StageError::NoInitialMedals)]
    #[case::empty_grid(
        |p: &mut StageParts| p.initial_medals = vec![InitialMedalPlacement::grid(Vec3::ZERO, 0, 3, 1, Vec3::ONE)],
        StageError::NoInitialMedals
    )]
    #[case::duplicate_goal(|p: &mut StageParts| p.goals.push(p.goals[0].clone()), StageError::DuplicateId { kind: "goal", id: 1 })]
    #[case::duplicate_bridge(|p: &mut StageParts| p.bridges.push(p.bridges[0].clone()), StageError::DuplicateId { kind: "bridge", id: 1 })]
    #[case::duplicate_rail(|p: &mut StageParts| p.rails.push(p.rails[0].clone()), StageError::DuplicateId { kind: "rail", id: 1 })]
    #[case::unknown_bridge(
        |p: &mut StageParts| p.wave_plan = WavePlan::new(vec![Wave::new(vec![spawn(0, 9, 1)])]),
        StageError::UnknownReference { kind: "bridge", id: 9 }
    )]
    #[case::unknown_goal(
        |p: &mut StageParts| p.wave_plan = WavePlan::new(vec![Wave::new(vec![spawn(0, 1, 7)])]),
        StageError::UnknownReference { kind: "goal", id: 7 }
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
    #[case::inside_rect(Shape::Rectangle { width: 4.0, depth: 2.0 }, Vec3::new(2.0, 9.0, -1.0), true)]
    #[case::outside_rect(Shape::Rectangle { width: 4.0, depth: 2.0 }, Vec3::new(2.1, 0.0, 0.0), false)]
    #[case::inside_circle(Shape::Circle { radius: 1.0 }, Vec3::new(0.6, 0.0, 0.6), true)]
    #[case::outside_circle(Shape::Circle { radius: 1.0 }, Vec3::new(0.8, 0.0, 0.8), false)]
    fn geometry_contains_xz(#[case] shape: Shape, #[case] p: Vec3, #[case] expected: bool) {
        assert_eq!(Geometry { shape, floor_y: 0.0 }.contains_xz(p), expected);
    }
}
