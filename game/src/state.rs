//! アプリケーション状態（基本設計 5.2）とインゲームのフェーズ（基本設計 6.4）。

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

/// インゲームのライフサイクル。ゲーム進行（固定ステップのSystem）は `Running` の間だけ動く。
/// `Running` の間は `ActiveGame` が必ず存在する。
#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(AppState = AppState::InGame)]
pub enum GamePhase {
    /// セッションと盤面を作っている
    #[default]
    Starting,
    Running,
    /// 結果の記録・保存をしてリザルトへ移る
    Ending,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_state::<AppState>()
        .add_sub_state::<GamePhase>();
}
