//! 見た目の指定。`key` を Presentation が見た目の定義ファイル（`visuals.visuals.ron`）で解決する。

use std::borrow::Cow;

use bevy::prelude::*;
use domain::id::{EnemyTypeId, MedalCode, PusherCharacterId};

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

    pub fn medal_key(medal: &MedalCode) -> String {
        format!("medal/{medal}")
    }

    pub fn enemy_key(kind: &EnemyTypeId) -> String {
        format!("enemy/{kind}")
    }

    pub fn pusher_key(character: &PusherCharacterId) -> String {
        format!("pusher/{character}")
    }
}
