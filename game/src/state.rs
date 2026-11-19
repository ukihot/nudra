//! アプリケーション状態（基本設計 5.2）。

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    /// 定義データの読込・検証
    #[default]
    Loading,
    CharacterSelect,
    InGame,
    Result,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_state::<AppState>();
}
