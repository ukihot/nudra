//! 定義データは起動時に全件読込・検証済みのため、`find` は技術的に失敗しない（基本設計 3.11）。

use crate::{
    id::{PusherCharacterId, StageId},
    pusher::PusherCharacter,
    stage::Stage,
};

pub trait StageRepository {
    fn find(&self, id: &StageId) -> Option<Stage>;
}

pub trait PusherCharacterRepository {
    fn find(&self, id: &PusherCharacterId) -> Option<PusherCharacter>;
}
