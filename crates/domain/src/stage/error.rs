use crate::id::RailId;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StageError {
    #[error("stage has no goal")]
    NoGoal,
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
    #[error("wave plan refers to unknown {kind} {id}")]
    UnknownReference { kind: &'static str, id: u16 },
    #[error("catapult yaw range is invalid ({min} > {max})")]
    InvalidCatapult { min: f32, max: f32 },
}
