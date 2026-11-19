//! 射出装置の定義（基本設計 3.12）。
//!
//! 射出の**制約**はステージで決まる（位置・初期の旋回角・旋回できる範囲・仰角・速度・発射周期）。
//! ゲーム中の**現在の状態**（現在の旋回角・次に発射できるTick）は `CatapultState` が持つ。

use super::StageError;
use crate::{
    math::{EulerRot, Quat, Vec3},
    time::Ticks,
};

/// `Catapult::new` に渡す組み立て用の値。旋回角はラジアンで、`base_yaw` からの相対値。
#[derive(Debug, Clone, PartialEq)]
pub struct CatapultParts {
    pub position: Vec3,
    pub base_yaw: f32,
    pub yaw_range: (f32, f32),
    /// ゲーム開始時の旋回角（`yaw_range` の内側）
    pub initial_yaw: f32,
    pub pitch: f32,
    pub speed: f32,
    /// 連射の周期。射出中はこの間隔ごとに1枚出る（クールダウンではない）
    pub fire_interval: Ticks,
}

/// 固定配置の射出装置の定義。
#[derive(Debug, Clone, PartialEq)]
pub struct Catapult(CatapultParts);

impl Catapult {
    pub fn new(parts: CatapultParts) -> Result<Self, StageError> {
        let (min, max) = parts.yaw_range;
        if min > max {
            return Err(StageError::InvalidCatapult { min, max });
        }
        // NaN は範囲判定に通らないため、ここで弾かれる
        if !(min..=max).contains(&parts.initial_yaw) {
            return Err(StageError::InitialYawOutOfRange { yaw: parts.initial_yaw, min, max });
        }
        Ok(Self(parts))
    }

    pub fn position(&self) -> Vec3 {
        self.0.position
    }

    pub fn initial_yaw(&self) -> f32 {
        self.0.initial_yaw
    }

    pub fn fire_interval(&self) -> Ticks {
        self.0.fire_interval
    }

    pub fn clamp_yaw(&self, yaw: f32) -> f32 {
        yaw.clamp(self.0.yaw_range.0, self.0.yaw_range.1)
    }

    /// 相対旋回角 `yaw` での射出方向（単位ベクトル）。
    pub fn direction(&self, yaw: f32) -> Vec3 {
        Quat::from_euler(
            EulerRot::YXZ,
            self.0.base_yaw + self.clamp_yaw(yaw),
            self.0.pitch,
            0.0,
        ) * Vec3::NEG_Z
    }

    pub fn launch_velocity(&self, yaw: f32) -> Vec3 {
        self.direction(yaw) * self.0.speed
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    pub fn catapult_parts() -> CatapultParts {
        CatapultParts {
            position: Vec3::new(0.0, 1.0, 5.0),
            base_yaw: 0.0,
            yaw_range: (-0.5, 0.5),
            initial_yaw: 0.0,
            pitch: 0.2,
            speed: 10.0,
            fire_interval: Ticks(12),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{fixture::catapult_parts, *};

    #[rstest]
    #[case::ordered((-0.5, 0.5), Ok(()))]
    #[case::single_angle((0.0, 0.0), Ok(()))]
    #[case::reversed((0.5, -0.5), Err(StageError::InvalidCatapult { min: 0.5, max: -0.5 }))]
    fn yaw_range_must_be_ordered(
        #[case] yaw_range: (f32, f32),
        #[case] expected: Result<(), StageError>,
    ) {
        let result = Catapult::new(CatapultParts { yaw_range, ..catapult_parts() }).map(|_| ());
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::inside(0.3, Ok(()))]
    #[case::on_edge(-0.5, Ok(()))]
    #[case::outside(0.6, Err(StageError::InitialYawOutOfRange { yaw: 0.6, min: -0.5, max: 0.5 }))]
    #[case::nan(f32::NAN, Err(StageError::InitialYawOutOfRange { yaw: f32::NAN, min: -0.5, max: 0.5 }))]
    fn initial_yaw_must_be_within_range(
        #[case] initial_yaw: f32,
        #[case] expected: Result<(), StageError>,
    ) {
        let result = Catapult::new(CatapultParts { initial_yaw, ..catapult_parts() }).map(|_| ());
        match (result, expected) {
            // NaN は等価比較できないため、エラーの種類だけを確かめる
            (Err(StageError::InitialYawOutOfRange { yaw, .. }), Err(_)) if yaw.is_nan() => {},
            (result, expected) => assert_eq!(result, expected),
        }
    }

    #[rstest]
    fn stage_defines_the_initial_yaw_and_interval() {
        let catapult =
            Catapult::new(CatapultParts { initial_yaw: 0.25, ..catapult_parts() }).expect("valid");
        assert_eq!(catapult.initial_yaw(), 0.25);
        assert_eq!(catapult.fire_interval(), Ticks(12));
    }
}
