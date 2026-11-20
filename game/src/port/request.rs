//! 入力: Presentation が書き、Infrastructure が読む（基本設計 6.4）。
//!
//! - 意図: 継続的な操作（旋回・発射ボタン）。[`ControlIntent`] という状態として扱い、台帳に載せない
//! - 指図: 結果の追跡が必要な離散操作（開始・進化・メニュー）。[`SubmitRequest`] で台帳へ提出し、
//!   必ず [`RequestOutcome`](super::RequestOutcome)（完了 / 却下 / 期限切れ）で終端が通知される

use bevy::prelude::*;
use domain::{
    difficulty::Difficulty,
    evolution::Evolution,
    id::{MedalCode, PusherCharacterId, StageId},
    progress::Language,
};

// ── 入力: 意図 ────────────────────────────────────────────

/// 継続的な操作意図。Presentation が毎フレーム書き、Infrastructure が固定ステップで読む。
/// 発射ボタンを押している間 `fire` は真で、実際に発射するかは Domain（発射周期）が決める。
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlIntent {
    /// まだ反映していない旋回量（ラジアン、+は左回り）。Infrastructure が反映したら0に戻す
    pub aim: f32,
    /// 発射ボタンが押されている
    pub fire: bool,
    /// オート射出。押していなくても発射周期ごとに撃つ
    pub auto_fire: bool,
}

// ── 入力: 指図 ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Navigate(OutgameScreen),
    SetLanguage(Language),
    /// キャラクターを選んでインゲームを開始する
    Start {
        character: PusherCharacterId,
        stage: StageId,
        difficulty: Difficulty,
    },
    Evolve(Evolution),
    SelectMedal(MedalCode),
    /// リザルトからキャラクター選択へ戻る
    Menu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutgameScreen {
    Title,
    Preparation,
    Options,
    Credits,
}

/// 指図の提出。台帳に記帳され、ID が振られる。
#[derive(Message, Debug, Clone)]
pub struct SubmitRequest(pub Request);
