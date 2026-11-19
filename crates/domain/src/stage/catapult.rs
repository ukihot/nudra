use super::StageError;
use crate::{
    math::{EulerRot, Quat, Vec3},
    time::Ticks,
};

/// 固定配置の射出装置。旋回角（yaw）はラジアン、基準方向 `base_yaw` からの相対値。
#[derive(Debug, Clone, PartialEq)]
pub struct Catapult {
    position: Vec3,
    base_yaw: f32,
    yaw_range: (f32, f32),
    pitch: f32,
    speed: f32,
    cooldown: Ticks,
}

impl Catapult {
    pub fn new(
        position: Vec3,
        base_yaw: f32,
        yaw_range: (f32, f32),
        pitch: f32,
        speed: f32,
        cooldown: Ticks,
    ) -> Result<Self, StageError> {
        let (min, max) = yaw_range;
        if min > max {
            return Err(StageError::InvalidCatapult { min, max });
        }
        Ok(Self {
            position,
            base_yaw,
            yaw_range,
            pitch,
            speed,
            cooldown,
        })
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }
    pub fn cooldown(&self) -> Ticks {
        self.cooldown
    }

    pub fn clamp_yaw(&self, yaw: f32) -> f32 {
        yaw.clamp(self.yaw_range.0, self.yaw_range.1)
    }

    /// 相対旋回角 `yaw` での射出方向（単位ベクトル）。
    pub fn direction(&self, yaw: f32) -> Vec3 {
        Quat::from_euler(EulerRot::YXZ, self.base_yaw + self.clamp_yaw(yaw), self.pitch, 0.0)
            * Vec3::NEG_Z
    }

    pub fn launch_velocity(&self, yaw: f32) -> Vec3 {
        self.direction(yaw) * self.speed
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::ordered((-0.5, 0.5), Ok(()))]
    #[case::single_angle((0.2, 0.2), Ok(()))]
    #[case::reversed((0.5, -0.5), Err(StageError::InvalidCatapult { min: 0.5, max: -0.5 }))]
    fn yaw_range_must_be_ordered(
        #[case] range: (f32, f32),
        #[case] expected: Result<(), StageError>,
    ) {
        let result = Catapult::new(Vec3::ZERO, 0.0, range, 0.0, 1.0, Ticks(0)).map(|_| ());
        assert_eq!(result, expected);
    }
}
