//! 敵の経路と進行方向（EnemyNavigationService相当、基本設計 3.8）。
//! 【仮決定】経路探索は持たず、Bridge外端→Bridge内端→Goalの経由点を順に目指す。

use crate::{
    id::{BridgeId, GoalId},
    math::Vec3,
    stage::Stage,
};

/// 経由点に到達したとみなす水平距離（m）。
const ARRIVE_RADIUS: f32 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    waypoints: Vec<Vec3>,
    next: usize,
}

impl Route {
    /// Stageの参照整合性は不変条件で保証済みだが、IDは外部入力のため `Option` で返す。
    pub fn new(stage: &Stage, bridge: BridgeId, goal: GoalId) -> Option<Self> {
        let bridge = stage.bridge(bridge)?;
        let goal = stage.goal(goal)?;
        Some(Self {
            waypoints: vec![bridge.entry, bridge.exit, goal.position],
            next: 1,
        })
    }

    pub fn start(&self) -> Vec3 {
        self.waypoints[0]
    }

    /// 現在位置から次に向かうべき水平方向（単位ベクトル）。到達した経由点は進める。
    pub fn steer(&mut self, position: Vec3) -> Vec3 {
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        while self.next < self.waypoints.len() - 1
            && flat(self.waypoints[self.next] - position).length() < ARRIVE_RADIUS
        {
            self.next += 1;
        }
        flat(self.waypoints[self.next] - position).normalize_or_zero()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::stage::fixture::stage;

    #[rstest]
    fn route_starts_at_bridge_entry_and_heads_to_exit(stage: Stage) {
        let mut route = Route::new(&stage, BridgeId(1), GoalId(1)).expect("known ids");
        assert_eq!(route.start(), Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(
            route.steer(Vec3::new(10.0, 0.3, 0.0)),
            Vec3::NEG_X,
            "height is ignored"
        );
    }

    #[rstest]
    fn route_switches_to_goal_after_bridge_exit(stage: Stage) {
        let mut route = Route::new(&stage, BridgeId(1), GoalId(1)).expect("known ids");
        let dir = route.steer(Vec3::new(5.1, 0.0, 0.0));
        assert!(
            (dir - Vec3::new(-5.1, 0.0, -4.0).normalize()).length() < 1e-5,
            "{dir:?}"
        );
    }

    #[rstest]
    fn route_stops_at_goal(stage: Stage) {
        let mut route = Route::new(&stage, BridgeId(1), GoalId(1)).expect("known ids");
        route.steer(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(route.steer(Vec3::new(0.0, 0.0, -4.0)), Vec3::ZERO);
    }

    #[rstest]
    #[case::unknown_bridge(BridgeId(9), GoalId(1))]
    #[case::unknown_goal(BridgeId(1), GoalId(9))]
    fn route_with_unknown_id_is_none(stage: Stage, #[case] bridge: BridgeId, #[case] goal: GoalId) {
        assert!(Route::new(&stage, bridge, goal).is_none());
    }
}
