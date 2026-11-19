use crate::{id::RailId, math::Vec3};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StageError {
    #[error("stage has no goal")]
    NoGoal,
    #[error("exactly one goal must be available on Easy (found {0})")]
    EasyGoalCount(usize),
    #[error("stage has no bridge")]
    NoBridge,
    #[error("stage has no rail")]
    NoRail,
    #[error("stage has no initial medals")]
    NoInitialMedals,
    #[error("duplicate {kind} id {id}")]
    DuplicateId { kind: &'static str, id: u16 },
    #[error("rail {0} must have at least 2 points and a positive length")]
    InvalidRail(RailId),
    #[error("stage must have exactly 4 waves (found {0})")]
    WaveCount(usize),
    #[error("the final wave must have only the final boss")]
    FinalWaveHasRegulars,
    #[error("wave plan refers to unknown {kind} {id}")]
    UnknownReference { kind: &'static str, id: u16 },
    #[error("catapult yaw range is invalid ({min} > {max})")]
    InvalidCatapult { min: f32, max: f32 },
    #[error("catapult initial yaw {yaw} is outside {min}..={max}")]
    InitialYawOutOfRange { yaw: f32, min: f32, max: f32 },
    #[error("abyss must satisfy min < max on every axis ({min:?}, {max:?})")]
    InvalidAbyss { min: Vec3, max: Vec3 },
}
