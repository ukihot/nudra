//! 定義データは起動時に全件読込・検証済みのため、`find` は技術的に失敗しない（基本設計 3.11）。

use crate::{
    id::{PusherCharacterId, StageId},
    progress::PlayerProgress,
    pusher::PusherCharacter,
    stage::Stage,
};

pub trait StageRepository {
    fn find(&self, id: &StageId) -> Option<Stage>;
}

pub trait PusherCharacterRepository {
    fn find(&self, id: &PusherCharacterId) -> Option<PusherCharacter>;
}

/// 進行記録の保存先（オフラインファースト、基本設計 5.7）。
/// 定義データのCatalogと違いローカル保存は失敗し得るため `Result` を返す。
/// エラーの具体型は実装（Infrastructure）が決める。
pub trait ProgressRepository {
    type Error: std::error::Error + Send + Sync + 'static;

    /// まだ保存が無ければ `Ok(None)`。
    fn load(&self) -> Result<Option<PlayerProgress>, Self::Error>;

    fn save(&mut self, progress: &PlayerProgress) -> Result<(), Self::Error>;
}
