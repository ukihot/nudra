//! 出力: Infrastructure が書き、Presentation が読む。指図の終端・ドメインイベント・読み取り専用の
//! Resource。

use std::time::Duration;

use bevy::prelude::*;
use domain::{
    difficulty::Difficulty,
    event::DomainEvent,
    evolution::EvolutionChoice,
    id::{EnemyId, GoalId, MedalCode, PusherCharacterId, StageId},
    motion::MedalMotion,
    progress::Language,
};

use super::Request;

// ── 指図の終端 ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub enum RejectReason {
    /// 今の状態では受け付けない（例: インゲーム中の開始）
    WrongState,
    /// 同じ種類の指図が同じ受付で重複した（連打など）。最初の1件だけ実行する
    Duplicate,
    /// 受付時の検証に失敗した（理由はログ用の英語）
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestStatus {
    Completed,
    Rejected(RejectReason),
    /// 受け付けられないまま期限が過ぎた
    Expired,
}

/// 指図の終端。提出されたすべての指図について、ちょうど1回届く。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct RequestOutcome {
    pub id: RequestId,
    pub request: Request,
    pub status: RequestStatus,
}

// ── 出力 ──────────────────────────────────────────────────

/// ドメインイベントの配信。順序はFixedUpdateのSystemSetで確定済み。
#[derive(Message, Debug, Clone)]
pub struct DomainEventMsg(pub DomainEvent);

#[derive(Debug, Clone, PartialEq)]
pub struct RosterEntry {
    pub id: PusherCharacterId,
    pub name: String,
}

/// キャラクター選択画面に並べる一覧（ID順）。
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct CharacterRoster(pub Vec<RosterEntry>);

#[derive(Debug, Clone, PartialEq)]
pub struct StageEntry {
    pub id: StageId,
    pub unlocked: Difficulty,
}

#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct StageRoster(pub Vec<StageEntry>);

#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct OptionsModel {
    pub language: Language,
}

/// インゲームHUD用の値。
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct HudModel {
    /// 1始まりのWave番号
    pub wave: usize,
    pub alive_enemies: usize,
    pub level: u32,
    pub evolution_percent: u32,
}

/// 進化画面の提示内容。選択肢と可否は Domain（`GameSession::evolution_choices`）が決める。
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct EvolutionModel(pub Vec<EvolutionChoice>);

#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct MedalModel {
    pub owned: Vec<MedalCode>,
    pub selected: Option<MedalCode>,
}

/// 照準表示用の値。
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct AimModel {
    pub origin: Vec3,
    pub direction: Vec3,
}

/// 直近のゲーム結果。
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub enum GameResult {
    Won {
        defeated: u32,
        best_defeated: u32,
        new_record: bool,
        play_time: Duration,
    },
    Lost {
        enemy: EnemyId,
        goal: GoalId,
        defeated: u32,
        /// このステージの最多撃破数（今回を含む）
        best_defeated: u32,
        new_record: bool,
        /// ゲームオーバーまでのゲーム内経過時間（一時停止を含まない）
        play_time: Duration,
    },
}

/// メダルの挙動ステータス（飛行中・転がり中・静止、基本設計 3.15）。Infrastructure
/// が物理から更新し、 Presentation
/// はアニメ・アセットの切り替えに読む。能力の発動も同じ値で決まるため、見た目とずれない。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MedalMotionState(pub MedalMotion);
