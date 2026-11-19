//! Infrastructure と Presentation の境界（Bevy流の依存性逆転）。
//!
//! 入力は2種類に分ける（基本設計 6.4）:
//! - 意図: 継続的な操作（旋回・発射ボタン）。[`ControlIntent`] という状態として扱い、台帳に載せない
//! - 指図: 結果の追跡が必要な離散操作（開始・強化・メニュー）。[`SubmitRequest`] で台帳へ提出し、
//!   必ず [`RequestOutcome`]（完了 / 却下 / 期限切れ）で終端が通知される
//!
//! 出力はドメインイベントの Message と、読み取り専用の Resource（`*Model`）。
//! 見た目の指定は Infrastructure が付ける [`Appearance`] で、Presentation がアセットに解決する。

use std::{borrow::Cow, time::Duration};

use bevy::prelude::*;
use domain::{
    event::DomainEvent,
    id::{EnemyId, EnemyTypeId, GoalId, PusherCharacterId},
    session::Upgrade,
    spec::{MedalType, SpecialKind},
};

// ── 入力: 意図 ────────────────────────────────────────────

/// 継続的な操作意図。Presentation が毎フレーム書き、Infrastructure が固定ステップで読む。
/// 発射ボタンを押している間 `fire` は真で、実際に発射するかは Domain（クールダウン）が決める。
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct ControlIntent {
    /// まだ反映していない旋回量（ラジアン、+は左回り）。Infrastructure が反映したら0に戻す
    pub aim: f32,
    /// 発射ボタンが押されている
    pub fire: bool,
}

// ── 入力: 指図 ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// キャラクターを選んでインゲームを開始する
    Start { character: PusherCharacterId },
    /// 【仮】成長の選択方式は未確定。開発用の操作から送る
    LevelUp(Upgrade),
    /// リザルトからキャラクター選択へ戻る
    Menu,
}

/// 指図の提出。台帳に記帳され、ID が振られる。
#[derive(Message, Debug, Clone)]
pub struct SubmitRequest(pub Request);

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

/// インゲームHUD用の値。
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct HudModel {
    /// 1始まりのWave番号
    pub wave: usize,
    pub alive_enemies: usize,
    pub level: u32,
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

// ── 見た目の指定 ──────────────────────────────────────────

/// 仮の形状。アセットが無いときのプレースホルダと、ビルボードの大きさに使う。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Silhouette {
    /// Y軸方向の円柱
    Cylinder {
        radius: f32,
        height: f32,
    },
    Cuboid {
        size: Vec3,
    },
    /// Y軸方向のカプセル（`length` は円柱部分の長さ）
    Capsule {
        radius: f32,
        length: f32,
    },
}

impl Silhouette {
    /// 外形の高さ（ビルボードの大きさの基準）。
    pub fn height(self) -> f32 {
        match self {
            Self::Cylinder { height, .. } => height,
            Self::Cuboid { size } => size.y,
            Self::Capsule { radius, length } => length + radius * 2.0,
        }
    }
}

/// 「何に見えるべきか」の指定。`key` を Presentation が見た目の定義ファイルで解決する。
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Appearance {
    pub key: Cow<'static, str>,
    pub silhouette: Silhouette,
}

impl Appearance {
    pub fn new(key: impl Into<Cow<'static, str>>, silhouette: Silhouette) -> Self {
        Self { key: key.into(), silhouette }
    }

    pub fn medal_key(medal: MedalType) -> &'static str {
        match medal {
            MedalType::Normal => "medal/normal",
            MedalType::Special(kind) => match kind {
                SpecialKind::Fire => "medal/fire",
                SpecialKind::Ice => "medal/ice",
                SpecialKind::Explosion => "medal/explosion",
                SpecialKind::Heavy => "medal/heavy",
                SpecialKind::Magnet => "medal/magnet",
                SpecialKind::Bounce => "medal/bounce",
            },
        }
    }

    pub fn enemy_key(kind: &EnemyTypeId) -> String {
        format!("enemy/{kind}")
    }

    pub fn pusher_key(character: &PusherCharacterId) -> String {
        format!("pusher/{character}")
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<SubmitRequest>()
        .add_message::<RequestOutcome>()
        .add_message::<DomainEventMsg>()
        .init_resource::<ControlIntent>()
        .init_resource::<CharacterRoster>()
        .init_resource::<HudModel>()
        .init_resource::<AimModel>();
}
