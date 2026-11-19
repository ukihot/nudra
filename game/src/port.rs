//! Infrastructure と Presentation の境界（Bevy流の依存性逆転）。
//!
//! - 入力ポート: Presentation → Infrastructure の Message（`*Requested`）
//! - 出力ポート: ドメインイベントの Message と、読み取り専用の Resource（`*Model`）
//! - 見た目の指定: Infrastructure が付ける [`Appearance`]。Presentation がアセットに解決する

use std::borrow::Cow;

use bevy::prelude::*;
use domain::{
    event::DomainEvent,
    id::{EnemyId, EnemyTypeId, GoalId, PusherCharacterId},
    session::Upgrade,
    spec::{MedalType, SpecialKind},
};

// ── 入力ポート ─────────────────────────────────────────────

/// キャラクターを選んでインゲームを開始する。
#[derive(Message, Debug, Clone)]
pub struct StartRequested {
    pub character: PusherCharacterId,
}

/// カタパルトの旋回（ラジアン、相対値）。範囲外はDomainが丸める。
#[derive(Message, Debug, Clone, Copy)]
pub struct AimRequested {
    pub delta: f32,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct LaunchRequested;

/// 【仮】成長の選択方式は未確定。開発用の操作から送る。
#[derive(Message, Debug, Clone, Copy)]
pub struct LevelUpRequested(pub Upgrade);

/// リザルトからキャラクター選択へ戻る。
#[derive(Message, Debug, Clone, Copy)]
pub struct MenuRequested;

// ── 出力ポート ─────────────────────────────────────────────

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
    app.add_message::<StartRequested>()
        .add_message::<AimRequested>()
        .add_message::<LaunchRequested>()
        .add_message::<LevelUpRequested>()
        .add_message::<MenuRequested>()
        .add_message::<DomainEventMsg>()
        .init_resource::<CharacterRoster>()
        .init_resource::<HudModel>()
        .init_resource::<AimModel>();
}
